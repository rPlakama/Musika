/*
 * Musika — a keyboard-driven music player.
 * Copyright (C) 2026 rPlakama <rPlakama@proton.me>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

/* Minimal libpipewire output sink.  PCM is S16LE stereo at 48 kHz and moves
 * through a lock-free SPSC ring into the real-time process callback.
 *
 * PipeWire >= 1.6.6 supplies the priority-inversion-aware pw_thread_loop
 * implementation used here; the ring atomics do not depend on that lock. */

#include <errno.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <pipewire/pipewire.h>
#include <spa/param/audio/format-utils.h>
#include <spa/param/audio/raw.h>
#include <spa/pod/builder.h>

#define MUSIKA_RATE 48000
#define MUSIKA_CHANNELS 2
#define RING_FRAMES (1u << 15)

typedef struct MusikaSink {
    struct pw_thread_loop *loop;
    struct pw_context *context;
    struct pw_core *core;
    struct pw_stream *stream;
    int16_t *ring;
    _Atomic size_t read_pos;
    _Atomic size_t write_pos;
    _Atomic int paused;
    _Atomic int muted;
    _Atomic float volume;
    _Atomic int failed;
    _Atomic int closed;
    _Atomic uint64_t played_frames;
} MusikaSink;

static void set_error(char *err, size_t len, const char *message)
{
    if (err && len)
        snprintf(err, len, "%s", message);
}

static void ring_write(MusikaSink *sink, const int16_t *data, size_t frames)
{
    while (frames > 0) {
        if (atomic_load_explicit(&sink->closed, memory_order_relaxed))
            return;
        size_t read = atomic_load_explicit(&sink->read_pos, memory_order_acquire);
        size_t write = atomic_load_explicit(&sink->write_pos, memory_order_relaxed);
        size_t used = write - read;
        size_t room = RING_FRAMES - used;
        if (room == 0) {
            struct timespec pause = { 0, 1000000 };
            nanosleep(&pause, NULL);
            continue;
        }
        size_t take = frames < room ? frames : room;
        for (size_t i = 0; i < take; ++i) {
            size_t at = (write + i) % RING_FRAMES;
            sink->ring[at * MUSIKA_CHANNELS] = data[i * MUSIKA_CHANNELS];
            sink->ring[at * MUSIKA_CHANNELS + 1] = data[i * MUSIKA_CHANNELS + 1];
        }
        atomic_store_explicit(&sink->write_pos, write + take, memory_order_release);
        data += take * MUSIKA_CHANNELS;
        frames -= take;
    }
}

static void on_process(void *userdata)
{
    MusikaSink *sink = userdata;
    struct pw_buffer *buffer = pw_stream_dequeue_buffer(sink->stream);
    if (!buffer)
        return;

    struct spa_buffer *spa = buffer->buffer;
    if (spa->n_datas == 0 || !spa->datas[0].data) {
        pw_stream_queue_buffer(sink->stream, buffer);
        return;
    }

    struct spa_data *data = &spa->datas[0];
    uint32_t max_frames = data->maxsize / (MUSIKA_CHANNELS * sizeof(int16_t));
    uint32_t frames = buffer->requested ? buffer->requested : max_frames;
    if (frames > max_frames)
        frames = max_frames;

    size_t read = atomic_load_explicit(&sink->read_pos, memory_order_relaxed);
    size_t write = atomic_load_explicit(&sink->write_pos, memory_order_acquire);
    size_t available = write - read;
    uint32_t take = (uint32_t)(available < frames ? available : frames);
    int16_t *out = data->data;
    for (uint32_t i = 0; i < take; ++i) {
        size_t at = (read + i) % RING_FRAMES;
        out[i * MUSIKA_CHANNELS] = sink->ring[at * MUSIKA_CHANNELS];
        out[i * MUSIKA_CHANNELS + 1] = sink->ring[at * MUSIKA_CHANNELS + 1];
    }
    for (uint32_t i = take; i < frames; ++i) {
        out[i * MUSIKA_CHANNELS] = 0;
        out[i * MUSIKA_CHANNELS + 1] = 0;
    }

    if (atomic_load_explicit(&sink->muted, memory_order_relaxed)) {
        memset(out, 0, (size_t)frames * MUSIKA_CHANNELS * sizeof(int16_t));
    } else {
        float volume = atomic_load_explicit(&sink->volume, memory_order_relaxed);
        if (volume < 0.0f)
            volume = 0.0f;
        if (volume > 1.0f)
            volume = 1.0f;
        if (volume != 1.0f) {
            for (uint32_t i = 0; i < frames * MUSIKA_CHANNELS; ++i)
                out[i] = (int16_t)((float)out[i] * volume);
        }
    }

    atomic_store_explicit(&sink->read_pos, read + take, memory_order_release);
    atomic_fetch_add_explicit(&sink->played_frames, take, memory_order_relaxed);
    data->chunk->offset = 0;
    data->chunk->stride = MUSIKA_CHANNELS * (int)sizeof(int16_t);
    data->chunk->size = frames * MUSIKA_CHANNELS * sizeof(int16_t);
    pw_stream_queue_buffer(sink->stream, buffer);
}

static void on_state_changed(void *userdata, enum pw_stream_state old,
                             enum pw_stream_state state, const char *error)
{
    (void)old;
    MusikaSink *sink = userdata;
    if (state == PW_STREAM_STATE_ERROR) {
        atomic_store_explicit(&sink->failed, 1, memory_order_relaxed);
        if (error)
            fprintf(stderr, "pipewire: stream error: %s\n", error);
    }
}

static const struct pw_stream_events STREAM_EVENTS = {
    .version = PW_VERSION_STREAM_EVENTS,
    .state_changed = on_state_changed,
    .process = on_process,
};

MusikaSink *musika_sink_init(char *err, size_t err_len)
{
    if (err && err_len)
        err[0] = '\0';
    pw_init(NULL, NULL);

    MusikaSink *sink = calloc(1, sizeof(*sink));
    if (!sink) {
        set_error(err, err_len, "sink allocation failed");
        return NULL;
    }
    atomic_init(&sink->read_pos, 0);
    atomic_init(&sink->write_pos, 0);
    atomic_init(&sink->paused, 0);
    atomic_init(&sink->muted, 0);
    atomic_init(&sink->volume, 1.0f);
    atomic_init(&sink->failed, 0);
    atomic_init(&sink->closed, 0);
    atomic_init(&sink->played_frames, 0);
    sink->ring = calloc((size_t)RING_FRAMES * MUSIKA_CHANNELS, sizeof(int16_t));
    if (!sink->ring) {
        set_error(err, err_len, "ring allocation failed");
        free(sink);
        return NULL;
    }

    /* PipeWire 1.6+ owns the loop locking; do not layer epoll/eventfd here. */
    sink->loop = pw_thread_loop_new("musika", NULL);
    if (!sink->loop) {
        set_error(err, err_len, "cannot create PipeWire thread loop");
        goto fail;
    }
    sink->context = pw_context_new(pw_thread_loop_get_loop(sink->loop), NULL, 0);
    if (!sink->context) {
        set_error(err, err_len, "cannot create PipeWire context");
        goto fail;
    }
    sink->core = pw_context_connect(sink->context, NULL, 0);
    if (!sink->core) {
        set_error(err, err_len, "cannot connect to PipeWire");
        goto fail;
    }

    struct pw_properties *props = pw_properties_new(
        PW_KEY_MEDIA_TYPE, "Audio",
        PW_KEY_MEDIA_CATEGORY, "Playback",
        PW_KEY_MEDIA_ROLE, "Music",
        PW_KEY_APP_NAME, "Musika",
        NULL);
    if (!props) {
        set_error(err, err_len, "cannot create stream properties");
        goto fail;
    }
    sink->stream = pw_stream_new_simple(
        pw_thread_loop_get_loop(sink->loop), "Musika", props, &STREAM_EVENTS, sink);
    if (!sink->stream) {
        set_error(err, err_len, "cannot create PipeWire stream");
        goto fail;
    }

    /* The POD builder is linked from libpipewire >= 1.6.6 (no vendored
     * builder).  Keep the capability fixed to S16LE/stereo/48 kHz. */
    struct spa_audio_info_raw info = { 0 };
    info.format = SPA_AUDIO_FORMAT_S16_LE;
    info.rate = MUSIKA_RATE;
    info.channels = MUSIKA_CHANNELS;
    uint8_t pod_buffer[1024];
    struct spa_pod_builder builder = SPA_POD_BUILDER_INIT(pod_buffer, sizeof(pod_buffer));
    const struct spa_pod *params[1];
    params[0] = spa_format_audio_raw_build(&builder, SPA_PARAM_EnumFormat, &info);
    int ret = pw_stream_connect(
        sink->stream, PW_DIRECTION_OUTPUT, PW_ID_ANY,
        PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_RT_PROCESS,
        params, 1);
    if (ret < 0) {
        set_error(err, err_len, "cannot connect PipeWire stream");
        goto fail;
    }
    if (pw_thread_loop_start(sink->loop) < 0) {
        set_error(err, err_len, "cannot start PipeWire thread loop");
        goto fail;
    }
    return sink;

fail:
    if (sink->stream)
        pw_stream_destroy(sink->stream);
    if (sink->core)
        pw_core_disconnect(sink->core);
    if (sink->context)
        pw_context_destroy(sink->context);
    if (sink->loop)
        pw_thread_loop_destroy(sink->loop);
    free(sink->ring);
    free(sink);
    pw_deinit();
    return NULL;
}

void musika_sink_write(MusikaSink *sink, const int16_t *data, size_t frames)
{
    if (sink && data && frames)
        ring_write(sink, data, frames);
}

void musika_sink_set_paused(MusikaSink *sink, int paused)
{
    if (!sink)
        return;
    atomic_store_explicit(&sink->paused, paused != 0, memory_order_relaxed);
    pw_thread_loop_lock(sink->loop);
    pw_stream_set_active(sink->stream, paused ? false : true);
    pw_thread_loop_unlock(sink->loop);
}

void musika_sink_set_muted(MusikaSink *sink, int muted)
{
    if (sink)
        atomic_store_explicit(&sink->muted, muted != 0, memory_order_relaxed);
}

void musika_sink_set_volume(MusikaSink *sink, float volume)
{
    if (!sink)
        return;
    if (volume < 0.0f)
        volume = 0.0f;
    if (volume > 1.0f)
        volume = 1.0f;
    atomic_store_explicit(&sink->volume, volume, memory_order_relaxed);
}

int musika_sink_failed(MusikaSink *sink)
{
    return sink ? atomic_load_explicit(&sink->failed, memory_order_relaxed) : 1;
}

void musika_sink_destroy(MusikaSink *sink)
{
    if (!sink)
        return;
    if (sink->loop) {
        pw_thread_loop_stop(sink->loop);
    }
    if (sink->stream) {
        pw_stream_disconnect(sink->stream);
        pw_stream_destroy(sink->stream);
    }
    if (sink->core)
        pw_core_disconnect(sink->core);
    if (sink->context)
        pw_context_destroy(sink->context);
    if (sink->loop)
        pw_thread_loop_destroy(sink->loop);
    free(sink->ring);
    free(sink);
    pw_deinit();
}

void musika_sink_reset(MusikaSink *sink)
{
    if (!sink)
        return;
    atomic_store_explicit(&sink->closed, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->read_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->write_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->played_frames, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->failed, 0, memory_order_relaxed);
}

void musika_sink_close_writes(MusikaSink *sink)
{
    if (sink)
        atomic_store_explicit(&sink->closed, 1, memory_order_relaxed);
}

uint64_t musika_sink_played_frames(MusikaSink *sink)
{
    return sink ? atomic_load_explicit(&sink->played_frames, memory_order_relaxed) : 0;
}

void musika_sink_flush(MusikaSink *sink)
{
    if (!sink)
        return;
    atomic_store_explicit(&sink->closed, 1, memory_order_relaxed);
    atomic_store_explicit(&sink->read_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->write_pos, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->played_frames, 0, memory_order_relaxed);
    atomic_store_explicit(&sink->closed, 0, memory_order_relaxed);
}
