// Musika — row shaping worker. Parses payloads and maps them to model rows
// off the GUI thread; QML delegate creation itself stays on the GUI thread.
function formatTime(seconds) {
    var total = Math.max(0, Math.floor(seconds || 0));
    var h = Math.floor(total / 3600);
    var m = Math.floor((total % 3600) / 60);
    var s = total % 60;
    var mm = (h > 0 && m < 10 ? "0" : "") + m;
    return (h > 0 ? h + ":" + mm : String(m)) + ":" + (s < 10 ? "0" : "") + s;
}

function mapAlbums(list) {
    var rows = [];
    for (var i = 0; i < list.length; i++) {
        var album = list[i];
        var artist = album.artist ? album.artist : "";
        var year = album.year ? String(album.year) : "";
        rows.push({
            title: album.title,
            subtitle: artist + (year.length > 0 ? " · " + year : ""),
            meta: String(album.song_count) + " tracks",
            cover: album.cover ? album.cover : "",
            albumId: album.id,
            artist: artist,
            year: year,
            tracks: String(album.song_count),
            songId: "",
            starred: album.starred === true,
            rating: Number(album.rating) || 0
        });
    }
    return rows;
}

function mapArtists(list) {
    var rows = [];
    for (var i = 0; i < list.length; i++) {
        var artist = list[i];
        var count = artist.album_count;
        rows.push({
            title: artist.name,
            subtitle: "",
            meta: count + (count === 1 ? " album" : " albums"),
            cover: artist.cover ? artist.cover : "",
            artistId: artist.id,
            artistName: artist.name,
            artistCover: artist.cover ? artist.cover : "",
            songId: "",
            albumId: "",
            starred: artist.starred === true,
            rating: Number(artist.rating) || 0
        });
    }
    return rows;
}

function mapPlaylists(list) {
    var rows = [];
    for (var i = 0; i < list.length; i++) {
        var item = list[i];
        var origin = item.source === "m3u"
            ? "M3U"
            : (item.owner && item.owner.length > 0 ? item.owner : "Playlist");
        rows.push({
            title: item.name,
            subtitle: origin + " · " + item.song_count
                + (item.song_count === 1 ? " song" : " songs"),
            meta: item.duration > 0 ? formatTime(item.duration) : "",
            cover: "",
            playlistId: item.id,
            songId: "",
            albumId: "",
            starred: false,
            rating: 0
        });
    }
    return rows;
}

function mapSongsPage(data) {
    var rows = [];
    for (var i = 0; i < data.rows.length; i++) {
        var song = data.rows[i];
        var artist = song.artist ? song.artist : "";
        var album = song.album ? song.album : "";
        rows.push({
            title: song.title,
            subtitle: artist + (album.length > 0 ? " · " + album : ""),
            meta: formatTime(song.duration),
            cover: song.cover ? song.cover : "",
            songId: song.id,
            albumId: song.album_id ? song.album_id : "",
            starred: song.starred === true,
            rating: 0
        });
    }
    return rows;
}

function mapAlbumTracks(list) {
    var rows = [];
    var total = 0;
    var formats = {};
    for (var i = 0; i < list.length; i++) {
        var song = list[i];
        total += Number(song.duration) || 0;
        var suffix = song.suffix ? String(song.suffix).toLowerCase() : "";
        if (suffix.charAt(0) === ".")
            suffix = suffix.substring(1);
        if (suffix.length > 0)
            formats[suffix] = (formats[suffix] || 0) + 1;
        rows.push({
            track: song.track ? String(song.track) : String(i + 1),
            title: song.title,
            subtitle: song.artist ? String(song.artist) : "",
            duration: formatTime(song.duration),
            starred: song.starred === true,
            songId: song.id,
            albumId: song.album_id ? song.album_id : ""
        });
    }
    var format = "";
    var formatCount = 0;
    for (var key in formats) {
        if (formats[key] > formatCount) {
            format = key;
            formatCount = formats[key];
        }
    }
    return { rows: rows, meta: { total: total, format: format.toUpperCase() } };
}

function mapQueue(data) {
    var rows = [];
    for (var i = 0; i < data.items.length; i++) {
        var track = data.items[i];
        rows.push({
            title: track.title,
            subtitle: track.artist ? String(track.artist) : "",
            duration: formatTime(track.duration),
            songId: track.id,
            albumId: track.album_id ? track.album_id : "",
            starred: track.starred === true
        });
    }
    return {
        rows: rows,
        meta: { hash: data.hash, index: data.index }
    };
}

WorkerScript.onMessage = function (message) {
    try {
        var data = JSON.parse(message.payload);
        var rows = [];
        var meta = {};
        if (message.kind === "albums")
            rows = mapAlbums(data);
        else if (message.kind === "artists")
            rows = mapArtists(data);
        else if (message.kind === "playlists")
            rows = mapPlaylists(data);
        else if (message.kind === "songs-page") {
            rows = mapSongsPage(data);
            meta = { offset: data.offset, next: data.next, has_more: data.has_more };
        } else if (message.kind === "album-tracks") {
            var tracks = mapAlbumTracks(data);
            rows = tracks.rows;
            meta = tracks.meta;
        } else if (message.kind === "queue") {
            var queue = mapQueue(data);
            rows = queue.rows;
            meta = queue.meta;
        } else
            throw new Error("unknown row kind: " + message.kind);

        var batch = 128;
        for (var start = 0; start < rows.length; start += batch) {
            var end = Math.min(rows.length, start + batch);
            WorkerScript.sendMessage({
                id: message.id,
                kind: message.kind,
                rows: rows.slice(start, end),
                done: end >= rows.length,
                meta: meta
            });
        }
        if (rows.length === 0)
            WorkerScript.sendMessage({
                id: message.id, kind: message.kind, rows: [], done: true, meta: meta
            });
    } catch (error) {
        WorkerScript.sendMessage({
            id: message.id,
            kind: message.kind,
            error: String(error)
        });
    }
};
