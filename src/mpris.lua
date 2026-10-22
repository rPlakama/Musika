-- Musika — native MPRIS service, implemented as an mpv Lua script.
--
-- mpv is the playback backend, so the MPRIS object can live next to it.
-- State arrives as `script-message musika-mpris <json>` from the Rust side;
-- transport verbs are sent back to Musika's own IPC socket (`ipc.sock`) so
-- they go through the same dispatcher as keys and external IPC.

local socket = require("socket")
local unix = require("socket.unix")
local mp = require("mp")
local utils = require("mp.utils")

local BUS_NAME = "org.mpris.MediaPlayer2.musika"
local ROOT_PATH = "/org/mpris/MediaPlayer2"
local ROOT_IFACE = "org.mpris.MediaPlayer2"
local PLAYER_IFACE = "org.mpris.MediaPlayer2.Player"
local PROPS_IFACE = "org.freedesktop.DBus.Properties"
local DBUS = "org.freedesktop.DBus"

local SOCKET = mp.get_opt("musika-mpris-socket")
local state = nil
local last_key = ""
local last_position = 0
local last_position_at = 0
local last_position_emit = 0
local last_track_id = ""
local owned = false
local connection = nil
local inbuf = ""
local serial = 0
local replies = {}

local POSITION_UPDATE_INTERVAL = 1.0
local SEEK_JUMP_US = 2000000

-- ---------------------------------------------------------------------------
-- D-Bus wire format (little-endian)
-- ---------------------------------------------------------------------------

local BASIC_ALIGN = {
    y = 1, b = 4, n = 2, q = 2, i = 4, u = 4, x = 8, t = 8,
    d = 8, s = 4, o = 4, g = 1, v = 1,
}

local function align(pos, n)
    return pos + ((n - pos % n) % n)
end

local function parse_type(sig, i)
    local c = sig:sub(i, i)
    if c == "a" then
        local child, j = parse_type(sig, i + 1)
        return { code = "a", child = child }, j
    elseif c == "(" then
        local fields, j = {}, i + 1
        while sig:sub(j, j) ~= ")" do
            local t
            t, j = parse_type(sig, j)
            fields[#fields + 1] = t
        end
        return { code = "(", fields = fields }, j + 1
    elseif c == "{" then
        local key, j = parse_type(sig, i + 1)
        local value
        value, j = parse_type(sig, j)
        return { code = "{", key = key, value = value }, j + 1
    end
    return { code = c }, i + 1
end

local function type_align(t)
    if t.code == "a" then
        return 4
    elseif t.code == "(" or t.code == "{" then
        return 8
    end
    return BASIC_ALIGN[t.code] or 1
end

local Writer = {}
Writer.__index = Writer

function Writer.new()
    return setmetatable({ b = {}, pos = 0 }, Writer)
end

function Writer:byte(v)
    self.b[self.pos + 1] = v
    self.pos = self.pos + 1
end

function Writer:raw(s)
    for i = 1, #s do
        self.b[self.pos + i] = s:byte(i)
    end
    self.pos = self.pos + #s
end

function Writer:pad(n)
    local target = align(self.pos, n)
    while self.pos < target do
        self:byte(0)
    end
end

function Writer:u32(v)
    self:pad(4)
    local b = self.b
    b[self.pos + 1] = v % 256
    b[self.pos + 2] = math.floor(v / 256) % 256
    b[self.pos + 3] = math.floor(v / 65536) % 256
    b[self.pos + 4] = math.floor(v / 16777216) % 256
    self.pos = self.pos + 4
end

function Writer:i32(v)
    if v < 0 then
        v = v + 4294967296
    end
    self:u32(v)
end

function Writer:i64(v)
    self:pad(8)
    local hi = math.floor(v / 4294967296)
    local lo = v - hi * 4294967296
    self:u32(lo % 4294967296)
    self:u32(hi % 4294967296)
end

function Writer:double(x)
    self:pad(8)
    local sign = 0
    if x < 0 then
        sign = 1
        x = -x
    end
    local hi, lo
    if x == 0 then
        hi, lo = 0, 0
    else
        local m, e = math.frexp(x)
        local exp = e + 1022
        local mant = math.floor((m * 2 - 1) * 2 ^ 52)
        hi = sign * 2147483648 + exp * 1048576 + math.floor(mant / 4294967296)
        lo = mant % 4294967296
    end
    self:u32(lo)
    self:u32(hi)
end

function Writer:str(s)
    self:pad(4)
    self:u32(#s)
    self:raw(s)
    self:byte(0)
end

function Writer:sig(s)
    self:byte(#s)
    self:raw(s)
    self:byte(0)
end

function Writer:bytes()
    if self.pos == 0 then
        return ""
    end
    return string.char(table.unpack(self.b, 1, self.pos))
end

local function set_u32(bytes, pos, v)
    bytes[pos + 1] = v % 256
    bytes[pos + 2] = math.floor(v / 256) % 256
    bytes[pos + 3] = math.floor(v / 65536) % 256
    bytes[pos + 4] = math.floor(v / 16777216) % 256
end

local write_type

local function write_sig(w, sig, values)
    local i, vi = 1, 1
    while i <= #sig do
        local t
        t, i = parse_type(sig, i)
        write_type(w, t, values[vi])
        vi = vi + 1
    end
end

write_type = function(w, t, v)
    local c = t.code
    if c == "y" then
        w:byte(v or 0)
    elseif c == "b" then
        w:u32(v and 1 or 0)
    elseif c == "u" then
        w:u32(v)
    elseif c == "i" then
        w:i32(v)
    elseif c == "x" or c == "t" then
        w:i64(v)
    elseif c == "d" then
        w:double(v)
    elseif c == "s" or c == "o" then
        w:str(tostring(v))
    elseif c == "g" then
        w:sig(v)
    elseif c == "v" then
        w:sig(v.sig)
        write_sig(w, v.sig, { v.value })
    elseif c == "a" then
        w:pad(4)
        local len_pos = w.pos
        w:u32(0)
        w:pad(type_align(t.child))
        local start = w.pos
        if t.child.code == "{" then
            for key, value in pairs(v or {}) do
                w:pad(8)
                write_type(w, t.child.key, key)
                write_type(w, t.child.value, value)
            end
        else
            for _, item in ipairs(v or {}) do
                write_type(w, t.child, item)
            end
        end
        set_u32(w.b, len_pos, w.pos - start)
    elseif c == "(" then
        w:pad(8)
        for i, field in ipairs(t.fields) do
            write_type(w, field, v[i])
        end
    elseif c == "{" then
        w:pad(8)
        write_type(w, t.key, v.key)
        write_type(w, t.value, v.value)
    end
end

local Reader = {}
Reader.__index = Reader

function Reader.new(s)
    return setmetatable({ s = s, pos = 0 }, Reader)
end

function Reader:byte()
    local v = self.s:byte(self.pos + 1)
    self.pos = self.pos + 1
    return v or 0
end

function Reader:raw(n)
    local v = self.s:sub(self.pos + 1, self.pos + n)
    self.pos = self.pos + n
    return v
end

function Reader:pad(n)
    self.pos = align(self.pos, n)
end

function Reader:u32()
    self:pad(4)
    local a, b, c, d = self.s:byte(self.pos + 1, self.pos + 4)
    self.pos = self.pos + 4
    return a + b * 256 + c * 65536 + d * 16777216
end

function Reader:i32()
    local v = self:u32()
    if v >= 2147483648 then
        v = v - 4294967296
    end
    return v
end

function Reader:i64()
    self:pad(8)
    local lo = self:u32()
    local hi = self:u32()
    if hi >= 2147483648 then
        return (hi - 4294967296) * 4294967296 + lo
    end
    return hi * 4294967296 + lo
end

function Reader:double()
    self:pad(8)
    local lo = self:u32()
    local hi = self:u32()
    local sign = math.floor(hi / 2147483648)
    local exp = math.floor(hi / 1048576) % 2048
    local mant = (hi % 1048576) * 4294967296 + lo
    local value
    if exp == 0 then
        value = mant * 2 ^ -1074
    else
        value = (1 + mant / 2 ^ 52) * 2 ^ (exp - 1023)
    end
    if sign == 1 then
        value = -value
    end
    return value
end

function Reader:str()
    self:pad(4)
    local n = self:u32()
    local s = self:raw(n)
    self:byte()
    return s
end

function Reader:sig()
    local n = self:byte()
    local s = self:raw(n)
    self:byte()
    return s
end

local read_type

read_type = function(r, t)
    local c = t.code
    if c == "y" then
        return r:byte()
    elseif c == "b" then
        return r:u32() ~= 0
    elseif c == "u" then
        return r:u32()
    elseif c == "i" then
        return r:i32()
    elseif c == "x" or c == "t" then
        return r:i64()
    elseif c == "d" then
        return r:double()
    elseif c == "s" or c == "o" then
        return r:str()
    elseif c == "g" then
        return r:sig()
    elseif c == "v" then
        local sig = r:sig()
        return { sig = sig, value = read_sig(r, sig)[1] }
    elseif c == "a" then
        local len = r:u32()
        r:pad(type_align(t.child))
        local stop = r.pos + len
        if t.child.code == "{" then
            local map = {}
            while r.pos < stop do
                r:pad(8)
                local key = read_type(r, t.child.key)
                map[key] = read_type(r, t.child.value)
            end
            return map
        end
        local arr = {}
        while r.pos < stop do
            arr[#arr + 1] = read_type(r, t.child)
        end
        return arr
    elseif c == "(" then
        r:pad(8)
        local arr = {}
        for _, field in ipairs(t.fields) do
            arr[#arr + 1] = read_type(r, field)
        end
        return arr
    elseif c == "{" then
        r:pad(8)
        return {
            key = read_type(r, t.key),
            value = read_type(r, t.value),
        }
    end
end

function read_sig(r, sig)
    local out, i = {}, 1
    while i <= #sig do
        local t
        t, i = parse_type(sig, i)
        out[#out + 1] = read_type(r, t)
    end
    return out
end

-- ---------------------------------------------------------------------------
-- Messages
-- ---------------------------------------------------------------------------

local function next_serial()
    serial = serial + 1
    return serial
end

local function send_message(mtype, flags, fields, body_sig, body_values)
    local fw = Writer.new()
    for _, field in ipairs(fields) do
        fw:pad(8)
        fw:byte(field[1])
        fw:sig(field[2])
        write_type(fw, parse_type(field[2], 1), field[3])
    end

    local bw = Writer.new()
    if body_sig and body_sig ~= "" then
        write_sig(bw, body_sig, body_values or {})
    end

    local msg_serial = next_serial()
    local hw = Writer.new()
    hw:byte(108)
    hw:byte(mtype)
    hw:byte(flags or 0)
    hw:byte(1)
    hw:u32(bw.pos)
    hw:u32(msg_serial)
    hw:u32(fw.pos)
    hw:raw(fw:bytes())
    hw:pad(8)
    hw:raw(bw:bytes())

    local data = hw:bytes()
    local sent = 0
    while sent < #data do
        local last, err = connection:send(data, sent + 1)
        if not last then
            return nil, err
        end
        sent = last
    end
    return true, nil, msg_serial
end

local function send_call(destination, path, iface, member, sig, values)
    local fields = {
        { 1, "o", path },
        { 2, "s", iface },
        { 3, "s", member },
        { 6, "s", destination },
    }
    if sig and sig ~= "" then
        fields[#fields + 1] = { 8, "g", sig }
    end
    return send_message(1, 0, fields, sig, values)
end

local function send_reply(msg, sig, values)
    local fields = {
        { 1, "o", msg.path },
        { 5, "u", msg.serial },
        { 6, "s", msg.sender },
    }
    if sig and sig ~= "" then
        fields[#fields + 1] = { 8, "g", sig }
    end
    return send_message(2, 0, fields, sig, values)
end

local function send_error(msg, name, text)
    local fields = {
        { 1, "o", msg.path },
        { 4, "s", name },
        { 5, "u", msg.serial },
        { 6, "s", msg.sender },
        { 8, "g", "s" },
    }
    return send_message(3, 0, fields, "s", { text })
end

local function send_signal(path, iface, member, sig, values)
    local fields = {
        { 1, "o", path },
        { 2, "s", iface },
        { 3, "s", member },
    }
    if sig and sig ~= "" then
        fields[#fields + 1] = { 8, "g", sig }
    end
    return send_message(4, 0, fields, sig, values)
end

local function parse_message(bytes)
    local r = Reader.new(bytes)
    local endian = r:byte()
    if endian ~= 108 then
        return nil
    end
    local mtype = r:byte()
    local flags = r:byte()
    r:byte()
    local body_len = r:u32()
    local msg_serial = r:u32()
    local fields_len = r:u32()
    local fields_end = 16 + fields_len
    local fields = {}
    while r.pos < fields_end do
        r:pad(8)
        local code = r:byte()
        local sig = r:sig()
        fields[code] = read_sig(r, sig)[1]
    end
    r:pad(8)
    local body_sig = fields[8] or ""
    local body = {}
    if body_len > 0 then
        local br = Reader.new(bytes:sub(r.pos + 1, r.pos + body_len))
        body = read_sig(br, body_sig)
    end
    return {
        type = mtype,
        flags = flags,
        serial = msg_serial,
        path = fields[1],
        iface = fields[2],
        member = fields[3],
        error = fields[4],
        reply_serial = fields[5],
        destination = fields[6],
        sender = fields[7],
        sig = body_sig,
        body = body,
    }
end

-- ---------------------------------------------------------------------------
-- State
-- ---------------------------------------------------------------------------

local function track_path(id)
    if not id or id == "" then
        return "/org/mpris/MediaPlayer2/TrackList/NoTrack"
    end
    local safe = id:gsub("[^%w_]", "_")
    return "/org/musika/track/" .. safe
end

local function str(value)
    if value == nil then
        return ""
    end
    return tostring(value)
end

local function from_mpv()
    local meta = mp.get_property_native("metadata") or {}
    local pause = mp.get_property_native("pause")
    local idle = mp.get_property_native("idle-active")
    local pos = mp.get_property_number("playlist-pos")
    local count = mp.get_property_number("playlist-count") or 0
    local volume = mp.get_property_number("volume") or 100
    local mute = mp.get_property_native("mute")
    local duration = mp.get_property_number("duration")
    local position = mp.get_property_number("time-pos")
    return {
        playback_status = idle and "Stopped" or (pause and "Paused" or "Playing"),
        title = str(meta["title"] or mp.get_property("media-title")),
        artist = str(meta["artist"]),
        album = str(meta["album"]),
        track_id = "",
        duration_us = math.floor((duration or 0) * 1000000),
        position_us = math.floor((position or 0) * 1000000),
        volume = mute and 0 or (volume / 100),
        can_next = pos ~= nil and pos + 1 < count,
        can_prev = pos ~= nil and pos > 0,
        art_url = str(meta["mpris:artUrl"] or meta["artUrl"]),
    }
end

local function current_state()
    local live = from_mpv()
    if not state then
        return live
    end
    local out = {}
    for key, value in pairs(state) do
        out[key] = value
    end
    if out.playback_status == nil then
        out.playback_status = live.playback_status
    end
    out.position_us = live.position_us
    if out.title == nil or out.title == "" then
        out.title = live.title
    end
    if out.artist == nil or out.artist == "" then
        out.artist = live.artist
    end
    if out.album == nil or out.album == "" then
        out.album = live.album
    end
    if out.art_url == nil or out.art_url == "" then
        out.art_url = live.art_url
    end
    if out.duration_us == nil or out.duration_us <= 0 then
        out.duration_us = live.duration_us
    end
    if out.volume == nil then
        out.volume = live.volume
    end
    if out.can_next == nil then
        out.can_next = live.can_next
    end
    if out.can_prev == nil then
        out.can_prev = live.can_prev
    end
    return out
end

local function metadata(st)
    local map = {}
    map["mpris:trackid"] = { sig = "o", value = track_path(st.track_id) }
    if st.duration_us and st.duration_us > 0 then
        map["mpris:length"] = { sig = "x", value = st.duration_us }
    end
    if st.art_url and st.art_url ~= "" then
        map["mpris:artUrl"] = { sig = "s", value = st.art_url }
    end
    if st.title and st.title ~= "" then
        map["xesam:title"] = { sig = "s", value = st.title }
    end
    if st.artist and st.artist ~= "" then
        map["xesam:artist"] = { sig = "as", value = { st.artist } }
    end
    if st.album and st.album ~= "" then
        map["xesam:album"] = { sig = "s", value = st.album }
    end
    return map
end

local function root_properties()
    return {
        Identity = { sig = "s", value = "Musika" },
        DesktopEntry = { sig = "s", value = "musika" },
        CanQuit = { sig = "b", value = false },
        CanRaise = { sig = "b", value = false },
        HasTrackList = { sig = "b", value = false },
        SupportedUriSchemes = { sig = "as", value = {} },
        SupportedMimeTypes = { sig = "as", value = {} },
    }
end

local function player_properties()
    local st = current_state()
    return {
        PlaybackStatus = { sig = "s", value = st.playback_status },
        LoopStatus = { sig = "s", value = "None" },
        Rate = { sig = "d", value = 1.0 },
        Shuffle = { sig = "b", value = false },
        Metadata = { sig = "a{sv}", value = metadata(st) },
        Volume = { sig = "d", value = st.volume or 0 },
        Position = { sig = "x", value = st.position_us or 0 },
        MinimumRate = { sig = "d", value = 1.0 },
        MaximumRate = { sig = "d", value = 1.0 },
        CanControl = { sig = "b", value = true },
        CanPlay = { sig = "b", value = true },
        CanPause = { sig = "b", value = true },
        CanGoNext = { sig = "b", value = st.can_next == true },
        CanGoPrevious = { sig = "b", value = st.can_prev == true },
        CanSeek = { sig = "b", value = (st.duration_us or 0) > 0 },
        CanStop = { sig = "b", value = true },
    }
end

local function signal_key(st)
    return table.concat({
        str(st.playback_status), str(st.title), str(st.artist), str(st.album),
        str(st.track_id), str(st.duration_us), str(st.volume),
        str(st.can_next), str(st.can_prev), str(st.art_url),
    }, "\0")
end

local function emit_changes()
    local pending = current_state()
    local key = signal_key(pending)
    if key == last_key then
        return
    end
    last_key = key
    send_signal(ROOT_PATH, PROPS_IFACE, "PropertiesChanged", "sa{sv}as", {
        PLAYER_IFACE,
        {
            Metadata = { sig = "a{sv}", value = metadata(pending) },
            PlaybackStatus = { sig = "s", value = pending.playback_status },
            Volume = { sig = "d", value = pending.volume or 0 },
            CanGoNext = { sig = "b", value = pending.can_next == true },
            CanGoPrevious = { sig = "b", value = pending.can_prev == true },
            CanSeek = { sig = "b", value = (pending.duration_us or 0) > 0 },
        },
        {},
    })
end

local function emit_seeked(position)
    send_signal(ROOT_PATH, PLAYER_IFACE, "Seeked", "x", { position })
end

-- Track the playhead: signal real jumps and publish progress for clients that
-- do not interpolate Position themselves.
local function emit_progress(st)
    local now = socket.gettime()
    local position = st.position_us or 0
    local track_id = str(st.track_id)
    local expected = last_position
    if last_position_at > 0 and st.playback_status == "Playing" then
        expected = last_position + math.floor((now - last_position_at) * 1000000)
    end

    local seeked = last_position_at > 0
        and now - last_position_at < 5
        and track_id == last_track_id
        and math.abs(position - expected) > SEEK_JUMP_US
    last_position = position
    last_position_at = now
    last_track_id = track_id

    if seeked then
        emit_seeked(position)
    end
    if st.playback_status == "Playing"
        and now - last_position_emit >= POSITION_UPDATE_INTERVAL then
        last_position_emit = now
        send_signal(ROOT_PATH, PROPS_IFACE, "PropertiesChanged", "sa{sv}as", {
            PLAYER_IFACE,
            { Position = { sig = "x", value = position } },
            {},
        })
    end
end

-- ---------------------------------------------------------------------------
-- Transport
-- ---------------------------------------------------------------------------

local function direct_command(command)
    if command == "mpris_play" then
        mp.set_property("pause", false)
    elseif command == "mpris_pause" then
        mp.set_property("pause", true)
    elseif command == "mpris_toggle" then
        mp.command("cycle pause")
    elseif command == "mpris_stop" then
        mp.command("stop")
    elseif command == "mpris_next" then
        mp.command("playlist-next")
    elseif command == "mpris_prev" then
        mp.command("playlist-prev")
    else
        local value = tonumber(command:match("^mpris_seek:(.-)$"))
        if value then
            mp.commandv("seek", value / 1000000, "relative")
            return
        end
        value = tonumber(command:match("^mpris_position:(.-)$"))
        if value then
            mp.commandv("seek", value / 1000000, "absolute")
            return
        end
        value = tonumber(command:match("^mpris_volume:(.-)$"))
        if value then
            mp.set_property("volume", value * 100)
            if value > 0 then
                mp.set_property("mute", false)
            end
        end
    end
end

local function send_command(command)
    if SOCKET then
        local sock = unix.stream()
        if sock then
            sock:settimeout(0.2)
            if sock:connect(SOCKET) then
                sock:send(command .. "\n")
                sock:close()
                return
            end
            sock:close()
        end
    end
    direct_command(command)
end

-- ---------------------------------------------------------------------------
-- Interface methods and properties
-- ---------------------------------------------------------------------------

local INTROSPECTION = [[<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
    <method name="SetPosition"><arg name="TrackId" type="o" direction="in"/><arg name="Position" type="x" direction="in"/></method>
    <method name="OpenUri"><arg name="Uri" type="s" direction="in"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanControl" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanStop" type="b" access="read"/>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get"><arg name="interface" type="s" direction="in"/><arg name="property" type="s" direction="in"/><arg name="value" type="v" direction="out"/></method>
    <method name="GetAll"><arg name="interface" type="s" direction="in"/><arg name="properties" type="a{sv}" direction="out"/></method>
    <method name="Set"><arg name="interface" type="s" direction="in"/><arg name="property" type="s" direction="in"/><arg name="value" type="v" direction="in"/></method>
    <signal name="PropertiesChanged"><arg name="interface" type="s"/><arg name="changed" type="a{sv}"/><arg name="invalidated" type="as"/></signal>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect"><arg name="xml" type="s" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
    <method name="GetMachineId"><arg name="id" type="s" direction="out"/></method>
  </interface>
</node>
]]

local function handle_properties(msg)
    local iface = msg.body[1]
    local properties
    if iface == ROOT_IFACE then
        properties = root_properties()
    elseif iface == PLAYER_IFACE then
        properties = player_properties()
    else
        send_error(msg, "org.freedesktop.DBus.Error.UnknownInterface", "no such interface")
        return
    end
    if msg.member == "Get" then
        local prop = properties[msg.body[2]]
        if not prop then
            send_error(msg, "org.freedesktop.DBus.Error.UnknownProperty", "no such property")
            return
        end
        send_reply(msg, "v", { { sig = prop.sig, value = prop.value } })
    elseif msg.member == "GetAll" then
        send_reply(msg, "a{sv}", { properties })
    elseif msg.member == "Set" then
        local prop = msg.body[2]
        local variant = msg.body[3]
        if iface == PLAYER_IFACE and prop == "Volume" and variant then
            local value = tonumber(variant.value) or 0
            send_command("mpris_volume:" .. tostring(math.max(0, math.min(1, value))))
        end
        send_reply(msg, nil, nil)
    else
        send_error(msg, "org.freedesktop.DBus.Error.UnknownMethod", "no such method")
    end
end

local function handle_player(msg)
    local command = ({
        Play = "mpris_play",
        Pause = "mpris_pause",
        PlayPause = "mpris_toggle",
        Stop = "mpris_stop",
        Next = "mpris_next",
        Previous = "mpris_prev",
    })[msg.member]

    if command then
        send_command(command)
        send_reply(msg, nil, nil)
    elseif msg.member == "Seek" then
        local offset = msg.body[1] or 0
        local st = current_state()
        local position = (st.position_us or 0) + offset
        send_command("mpris_seek:" .. tostring(offset))
        emit_seeked(position)
        send_reply(msg, nil, nil)
    elseif msg.member == "SetPosition" then
        local track = msg.body[1] or ""
        local position = msg.body[2] or 0
        local st = current_state()
        if track == track_path(st.track_id) then
            send_command("mpris_position:" .. tostring(position))
            emit_seeked(position)
        end
        send_reply(msg, nil, nil)
    elseif msg.member == "OpenUri" then
        send_reply(msg, nil, nil)
    else
        send_error(msg, "org.freedesktop.DBus.Error.UnknownMethod", "no such method")
    end
end

local function dispatch(msg)
    if not msg then
        return
    end
    if msg.type == 2 or msg.type == 3 then
        if msg.reply_serial then
            replies[msg.reply_serial] = msg
        end
        return
    end
    if msg.type ~= 1 then
        return
    end

    if msg.iface == "org.freedesktop.DBus.Peer" then
        if msg.member == "Ping" then
            send_reply(msg, nil, nil)
        elseif msg.member == "GetMachineId" then
            local id = ""
            local file = io.open("/etc/machine-id")
            if file then
                id = file:read("*l") or ""
                file:close()
            end
            send_reply(msg, "s", { id })
        end
    elseif msg.iface == "org.freedesktop.DBus.Introspectable" then
        send_reply(msg, "s", { INTROSPECTION })
    elseif msg.iface == PROPS_IFACE then
        handle_properties(msg)
    elseif msg.iface == ROOT_IFACE then
        send_reply(msg, nil, nil)
    elseif msg.iface == PLAYER_IFACE then
        handle_player(msg)
    else
        send_error(msg, "org.freedesktop.DBus.Error.UnknownInterface", "no such interface")
    end
end

-- ---------------------------------------------------------------------------
-- Connection
-- ---------------------------------------------------------------------------

local function read_line(sock)
    local line = ""
    sock:settimeout(2)
    while not line:find("\r\n", 1, true) do
        local chunk, err = sock:receive(1)
        if not chunk then
            return nil, err
        end
        line = line .. chunk
    end
    return (line:gsub("\r\n$", ""))
end

local function uid_hex()
    local uid = "1000"
    local file = io.open("/proc/self/status")
    if file then
        for line in file:lines() do
            local value = line:match("^Uid:%s*(%d+)")
            if value then
                uid = value
                break
            end
        end
        file:close()
    end
    return (uid:gsub(".", function(c)
        return string.format("%02x", c:byte())
    end))
end

local function drain_socket(timeout)
    if not connection then
        return
    end
    connection:settimeout(timeout)

    while true do
        local chunk, err, partial = connection:receive(65536)
        if chunk then
            inbuf = inbuf .. chunk
            if #chunk < 65536 then
                break
            end
        elseif partial and #partial > 0 then
            inbuf = inbuf .. partial
            break
        else
            if err == "closed" then
                connection = nil
            end
            break
        end
    end

    while #inbuf >= 16 do
        local fields_len = inbuf:byte(13) + inbuf:byte(14) * 256
            + inbuf:byte(15) * 65536 + inbuf:byte(16) * 16777216
        local body_len = inbuf:byte(5) + inbuf:byte(6) * 256
            + inbuf:byte(7) * 65536 + inbuf:byte(8) * 16777216
        local pad = (8 - fields_len % 8) % 8
        local total = 16 + fields_len + pad + body_len
        if #inbuf < total then
            break
        end
        local msg = parse_message(inbuf:sub(1, total))
        inbuf = inbuf:sub(total + 1)
        dispatch(msg)
    end
end

local function wait_reply(id, timeout)
    local deadline = socket.gettime() + timeout
    while socket.gettime() < deadline do
        if replies[id] then
            return replies[id]
        end
        drain_socket(0.1)
        if not connection then
            return nil
        end
    end
    return replies[id]
end

local function connect()
    local address = os.getenv("DBUS_SESSION_BUS_ADDRESS") or ""
    local path = address:match("unix:path=([^,;]+)")
    if not path then
        return nil, "no unix session bus"
    end

    local sock = unix.stream()
    if not sock then
        return nil, "no unix socket"
    end
    sock:settimeout(2)
    local ok, err = sock:connect(path)
    if not ok then
        sock:close()
        return nil, err
    end

    sock:send("\0")
    sock:send("AUTH EXTERNAL " .. uid_hex() .. "\r\n")
    local line, line_err = read_line(sock)
    if not line then
        sock:close()
        return nil, line_err
    end
    if line:sub(1, 2) ~= "OK" then
        sock:close()
        return nil, "auth rejected: " .. line
    end
    sock:send("BEGIN\r\n")

    connection = sock
    return sock
end

local function own_name()
    local ok, err, hello_id = send_call(DBUS, "/org/freedesktop/DBus", DBUS, "Hello")
    if not ok then
        return nil, err
    end
    drain_socket(0.2)
    if not wait_reply(hello_id, 2) then
        return nil, "no Hello reply"
    end

    ok, err, request_id = send_call(DBUS, "/org/freedesktop/DBus", DBUS, "RequestName", "su", {
        BUS_NAME, 6,
    })
    if not ok then
        return nil, err
    end
    drain_socket(0.2)
    local reply = wait_reply(request_id, 2)
    if not reply then
        return nil, "no RequestName reply"
    end
    local result = reply.body[1]
    owned = result == 1 or result == 4
    return owned, owned and nil or ("RequestName returned " .. tostring(result))
end

-- ---------------------------------------------------------------------------
-- Lifecycle
-- ---------------------------------------------------------------------------

mp.register_script_message("musika-mpris", function(payload)
    local parsed = utils.parse_json(payload)
    if type(parsed) == "table" then
        state = parsed
        emit_changes()
    end
end)

local function tick()
    if not connection then
        local _, err = connect()
        if not err then
            local ok, owner_err = own_name()
            if not ok then
                mp.msg.warn("mpris: " .. tostring(owner_err))
            end
        end
    end
    if connection then
        drain_socket(0)
        emit_changes()
        emit_progress(current_state())
    end
    mp.add_timeout(0.05, tick)
end

local _, err = connect()
if err then
    mp.msg.warn("mpris: " .. tostring(err))
else
    local ok, owner_err = own_name()
    if not ok then
        mp.msg.warn("mpris: " .. tostring(owner_err))
    else
        mp.msg.info("mpris: serving " .. BUS_NAME)
        last_key = signal_key(current_state())
    end
end

mp.add_timeout(0.05, tick)
