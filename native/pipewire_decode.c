/*
 * Musika — a keyboard-driven music player.
 * Copyright (C) 2026 rPlakama <rPlakama@proton.me>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

/* FFmpeg decoder shim for the PipeWire player.  Output is always interleaved
 * S16 stereo at 48 kHz: FFmpeg/swresample owns rate conversion, so PipeWire
 * receives a fixed format and its graph resampler is not in this path. */

#include <errno.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/log.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>

#define MUSIKA_RATE 48000
#define MUSIKA_CHANNELS 2

typedef struct MusikaDecoder MusikaDecoder;
void musika_decoder_close(MusikaDecoder *d);

struct MusikaDecoder {
    AVFormatContext *format;
    AVCodecContext *codec;
    SwrContext *swr;
    AVPacket *packet;
    AVFrame *frame;
    int stream_index;
    int eof;
    int eof_sent;
    int16_t *pcm;
    int pcm_capacity;
    int pending_frames;
    int pending_offset;
    double duration;
    AVIOContext *avio;
    unsigned char *avio_buffer;
    void *buffer_io;
};

typedef struct {
    const unsigned char *data;
    size_t len;
    size_t pos;
} BufferIO;

static int buffer_read(void *opaque, uint8_t *buf, int size)
{
    BufferIO *io = opaque;
    if (io->pos >= io->len)
        return AVERROR_EOF;
    size_t left = io->len - io->pos;
    if ((size_t)size > left)
        size = (int)left;
    memcpy(buf, io->data + io->pos, (size_t)size);
    io->pos += (size_t)size;
    return size;
}

static int64_t buffer_seek(void *opaque, int64_t offset, int whence)
{
    BufferIO *io = opaque;
    int64_t target = 0;
    if (whence == AVSEEK_SIZE)
        return (int64_t)io->len;
    switch (whence & ~AVSEEK_FORCE) {
    case SEEK_SET:
        target = offset;
        break;
    case SEEK_CUR:
        target = (int64_t)io->pos + offset;
        break;
    case SEEK_END:
        target = (int64_t)io->len + offset;
        break;
    default:
        return -1;
    }
    if (target < 0 || (uint64_t)target > io->len)
        return -1;
    io->pos = (size_t)target;
    return target;
}

static void set_error(char *err, size_t len, const char *fmt, ...)
{
    if (!err || len == 0)
        return;
    va_list args;
    va_start(args, fmt);
    vsnprintf(err, len, fmt, args);
    va_end(args);
}

static int ensure_pcm(MusikaDecoder *d, int frames)
{
    if (frames <= d->pcm_capacity)
        return 0;
    int16_t *next = realloc(d->pcm, (size_t)frames * MUSIKA_CHANNELS * sizeof(int16_t));
    if (!next)
        return -ENOMEM;
    d->pcm = next;
    d->pcm_capacity = frames;
    return 0;
}

/* Receives one decoded frame and resamples it into `pcm`.  Returns 1 when a
 * frame was produced, 0 at EOF, or a negative errno/AVERROR. */
static int decode_more(MusikaDecoder *d)
{
    for (;;) {
        int ret = avcodec_receive_frame(d->codec, d->frame);
        if (ret == 0) {
            int delay = (int)swr_get_delay(d->swr, d->codec->sample_rate);
            int max_frames = (int)av_rescale_rnd(
                delay + d->frame->nb_samples, MUSIKA_RATE, d->codec->sample_rate, AV_ROUND_UP);
            if (max_frames < 1)
                max_frames = 1;
            if (ensure_pcm(d, max_frames) < 0) {
                av_frame_unref(d->frame);
                return -ENOMEM;
            }
            uint8_t *out[] = { (uint8_t *)d->pcm };
            int got = swr_convert(
                d->swr, out, max_frames,
                (const uint8_t **)d->frame->extended_data, d->frame->nb_samples);
            av_frame_unref(d->frame);
            if (got < 0)
                return got;
            if (got > 0) {
                d->pending_frames = got;
                d->pending_offset = 0;
                return 1;
            }
            continue;
        }
        if (ret == AVERROR_EOF) {
            d->eof = 1;
            return 0;
        }
        if (ret != AVERROR(EAGAIN))
            return ret;

        if (d->eof_sent)
            return 0;

        ret = av_read_frame(d->format, d->packet);
        if (ret == AVERROR_EOF) {
            avcodec_send_packet(d->codec, NULL);
            d->eof_sent = 1;
            continue;
        }
        if (ret < 0)
            return ret;
        if (d->packet->stream_index != d->stream_index) {
            av_packet_unref(d->packet);
            continue;
        }
        ret = avcodec_send_packet(d->codec, d->packet);
        av_packet_unref(d->packet);
        if (ret < 0 && ret != AVERROR(EAGAIN))
            return ret;
    }
}

static MusikaDecoder *decoder_open(const char *url, const unsigned char *buffer, size_t buffer_len,
                                   int (*interrupt)(void *), void *opaque,
                                   char *err, size_t err_len)
{
    if (err && err_len)
        err[0] = '\0';
    if (!url && !buffer)
        return NULL;

    avformat_network_init();

    MusikaDecoder *d = calloc(1, sizeof(*d));
    if (!d) {
        set_error(err, err_len, "decoder allocation failed");
        return NULL;
    }
    d->stream_index = -1;

    int ret;
    if (interrupt) {
        d->format = avformat_alloc_context();
        if (!d->format) {
            set_error(err, err_len, "format allocation failed");
            goto fail;
        }
        d->format->interrupt_callback.callback = interrupt;
        d->format->interrupt_callback.opaque = opaque;
    }
    if (buffer && buffer_len > 0) {
        BufferIO *io = calloc(1, sizeof(*io));
        if (!io) {
            set_error(err, err_len, "buffer allocation failed");
            goto fail;
        }
        io->data = buffer;
        io->len = buffer_len;
        d->buffer_io = io;
        d->avio_buffer = av_malloc(4096);
        if (!d->avio_buffer) {
            set_error(err, err_len, "io buffer allocation failed");
            goto fail;
        }
        d->avio = avio_alloc_context(d->avio_buffer, 4096, 0, io, buffer_read, NULL, buffer_seek);
        if (!d->avio) {
            set_error(err, err_len, "io context allocation failed");
            goto fail;
        }
        if (!d->format) {
            d->format = avformat_alloc_context();
            if (!d->format) {
                set_error(err, err_len, "format allocation failed");
                goto fail;
            }
        }
        d->format->pb = d->avio;
        d->format->flags |= AVFMT_FLAG_CUSTOM_IO;
        ret = avformat_open_input(&d->format, NULL, NULL, NULL);
    } else {
        if (!d->format) {
            d->format = avformat_alloc_context();
            if (!d->format) {
                set_error(err, err_len, "format allocation failed");
                goto fail;
            }
        }
        ret = avformat_open_input(&d->format, url, NULL, NULL);
    }
    if (ret < 0) {
        set_error(err, err_len, "cannot open input: %s", av_err2str(ret));
        goto fail;
    }
    ret = avformat_find_stream_info(d->format, NULL);
    if (ret < 0) {
        set_error(err, err_len, "cannot read stream info: %s", av_err2str(ret));
        goto fail;
    }

    const AVCodec *codec = NULL;
    ret = av_find_best_stream(d->format, AVMEDIA_TYPE_AUDIO, -1, -1, &codec, 0);
    if (ret < 0 || !codec) {
        set_error(err, err_len, "no audio stream: %s", av_err2str(ret));
        goto fail;
    }
    d->stream_index = ret;
    AVStream *stream = d->format->streams[d->stream_index];

    d->codec = avcodec_alloc_context3(codec);
    if (!d->codec) {
        set_error(err, err_len, "codec allocation failed");
        goto fail;
    }
    ret = avcodec_parameters_to_context(d->codec, stream->codecpar);
    if (ret < 0) {
        set_error(err, err_len, "codec parameters: %s", av_err2str(ret));
        goto fail;
    }
    ret = avcodec_open2(d->codec, codec, NULL);
    if (ret < 0) {
        set_error(err, err_len, "cannot open codec: %s", av_err2str(ret));
        goto fail;
    }

    if (d->codec->ch_layout.nb_channels == 0) {
        av_channel_layout_default(&d->codec->ch_layout, MUSIKA_CHANNELS);
    }
    AVChannelLayout out_layout;
    av_channel_layout_default(&out_layout, MUSIKA_CHANNELS);
    ret = swr_alloc_set_opts2(
        &d->swr, &out_layout, AV_SAMPLE_FMT_S16, MUSIKA_RATE,
        &d->codec->ch_layout, d->codec->sample_fmt, d->codec->sample_rate, 0, NULL);
    av_channel_layout_uninit(&out_layout);
    if (ret < 0 || !d->swr) {
        set_error(err, err_len, "resampler allocation failed");
        goto fail;
    }
    ret = swr_init(d->swr);
    if (ret < 0) {
        set_error(err, err_len, "cannot initialise resampler: %s", av_err2str(ret));
        goto fail;
    }

    d->packet = av_packet_alloc();
    d->frame = av_frame_alloc();
    if (!d->packet || !d->frame) {
        set_error(err, err_len, "packet/frame allocation failed");
        goto fail;
    }

    if (d->format->duration > 0)
        d->duration = (double)d->format->duration / AV_TIME_BASE;
    else if (stream->duration > 0)
        d->duration = (double)stream->duration * av_q2d(stream->time_base);

    return d;

fail:
    musika_decoder_close(d);
    return NULL;
}

/* ARC-RS-4.2 streaming AVIO: read via callback, 64 KB avio ring only. */
static MusikaDecoder *decoder_open_stream(int (*read_cb)(void *, uint8_t *, int),
                                          void *opaque,
                                          int (*interrupt)(void *),
                                          char *err, size_t err_len)
{
    if (err && err_len)
        err[0] = '\0';
    if (!read_cb)
        return NULL;
    avformat_network_init();
    MusikaDecoder *d = calloc(1, sizeof(*d));
    if (!d) {
        set_error(err, err_len, "decoder allocation failed");
        return NULL;
    }
    d->stream_index = -1;
    int ret;
    if (interrupt) {
        d->format = avformat_alloc_context();
        if (!d->format) {
            set_error(err, err_len, "format allocation failed");
            goto fail;
        }
        d->format->interrupt_callback.callback = interrupt;
        d->format->interrupt_callback.opaque = opaque;
    }
    d->avio_buffer = av_malloc(65536);
    if (!d->avio_buffer) {
        set_error(err, err_len, "io buffer allocation failed");
        goto fail;
    }
    d->avio = avio_alloc_context(d->avio_buffer, 65536, 0, opaque, read_cb, NULL, NULL);
    if (!d->avio) {
        set_error(err, err_len, "io context allocation failed");
        goto fail;
    }
    if (!d->format) {
        d->format = avformat_alloc_context();
        if (!d->format) {
            set_error(err, err_len, "format allocation failed");
            goto fail;
        }
    }
    d->format->pb = d->avio;
    d->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    ret = avformat_open_input(&d->format, NULL, NULL, NULL);
    if (ret < 0) {
        set_error(err, err_len, "cannot open input: %s", av_err2str(ret));
        goto fail;
    }
    ret = avformat_find_stream_info(d->format, NULL);
    if (ret < 0) {
        set_error(err, err_len, "cannot read stream info: %s", av_err2str(ret));
        goto fail;
    }
    {
        const AVCodec *codec = NULL;
        ret = av_find_best_stream(d->format, AVMEDIA_TYPE_AUDIO, -1, -1, &codec, 0);
        if (ret < 0 || !codec) {
            set_error(err, err_len, "no audio stream: %s", av_err2str(ret));
            goto fail;
        }
        d->stream_index = ret;
        AVStream *stream = d->format->streams[d->stream_index];
        d->codec = avcodec_alloc_context3(codec);
        if (!d->codec) {
            set_error(err, err_len, "codec allocation failed");
            goto fail;
        }
        ret = avcodec_parameters_to_context(d->codec, stream->codecpar);
        if (ret < 0) {
            set_error(err, err_len, "codec parameters: %s", av_err2str(ret));
            goto fail;
        }
        ret = avcodec_open2(d->codec, codec, NULL);
        if (ret < 0) {
            set_error(err, err_len, "cannot open codec: %s", av_err2str(ret));
            goto fail;
        }
        if (d->codec->ch_layout.nb_channels == 0) {
            av_channel_layout_default(&d->codec->ch_layout, MUSIKA_CHANNELS);
        }
        AVChannelLayout out_layout;
        av_channel_layout_default(&out_layout, MUSIKA_CHANNELS);
        ret = swr_alloc_set_opts2(&d->swr, &out_layout, AV_SAMPLE_FMT_S16, MUSIKA_RATE,
                                  &d->codec->ch_layout, d->codec->sample_fmt,
                                  d->codec->sample_rate, 0, NULL);
        av_channel_layout_uninit(&out_layout);
        if (ret < 0 || !d->swr) {
            set_error(err, err_len, "resampler allocation failed");
            goto fail;
        }
        ret = swr_init(d->swr);
        if (ret < 0) {
            set_error(err, err_len, "cannot initialise resampler: %s", av_err2str(ret));
            goto fail;
        }
        d->packet = av_packet_alloc();
        d->frame = av_frame_alloc();
        if (!d->packet || !d->frame) {
            set_error(err, err_len, "packet/frame allocation failed");
            goto fail;
        }
        if (d->format->duration > 0)
            d->duration = (double)d->format->duration / AV_TIME_BASE;
        else if (stream->duration > 0)
            d->duration = (double)stream->duration * av_q2d(stream->time_base);
    }
    return d;

fail:
    musika_decoder_close(d);
    return NULL;
}

MusikaDecoder *musika_decoder_open_stream(int (*read_cb)(void *, uint8_t *, int),
                                          void *opaque, int (*interrupt)(void *),
                                          char *err, size_t err_len)
{
    av_log_set_level(AV_LOG_ERROR);
    return decoder_open_stream(read_cb, opaque, interrupt, err, err_len);
}

MusikaDecoder *musika_decoder_open(const char *url, const unsigned char *buffer, size_t buffer_len,
                                   char *err, size_t err_len)
{
    av_log_set_level(AV_LOG_ERROR);
    return decoder_open(url, buffer, buffer_len, NULL, NULL, err, err_len);
}

MusikaDecoder *musika_decoder_open_url(const char *url, int (*interrupt)(void *),
                                       void *opaque, char *err, size_t err_len)
{
    av_log_set_level(AV_LOG_ERROR);
    return decoder_open(url, NULL, 0, interrupt, opaque, err, err_len);
}

int musika_decoder_read(MusikaDecoder *d, int16_t *out, int max_frames, int *frames_read)
{
    *frames_read = 0;
    if (!d || !out || max_frames <= 0)
        return -EINVAL;
    if (d->eof)
        return 0;

    while (*frames_read < max_frames) {
        if (d->pending_frames == 0) {
            int ret = decode_more(d);
            if (ret < 0)
                return ret;
            if (ret == 0)
                break;
        }
        int available = d->pending_frames - d->pending_offset;
        int room = max_frames - *frames_read;
        int take = available < room ? available : room;
        memcpy(
            out + (size_t)(*frames_read) * MUSIKA_CHANNELS,
            d->pcm + (size_t)d->pending_offset * MUSIKA_CHANNELS,
            (size_t)take * MUSIKA_CHANNELS * sizeof(int16_t));
        d->pending_offset += take;
        *frames_read += take;
        if (d->pending_offset >= d->pending_frames) {
            d->pending_frames = 0;
            d->pending_offset = 0;
        }
    }
    return 0;
}

int musika_decoder_seek(MusikaDecoder *d, double seconds)
{
    if (!d || seconds < 0)
        return -EINVAL;
    AVStream *stream = d->format->streams[d->stream_index];
    int64_t target = (int64_t)(seconds * AV_TIME_BASE);
    int64_t timestamp = av_rescale_q(target, AV_TIME_BASE_Q, stream->time_base);
    int ret = av_seek_frame(d->format, d->stream_index, timestamp, AVSEEK_FLAG_BACKWARD);
    if (ret < 0)
        return ret;
    avcodec_flush_buffers(d->codec);
    swr_close(d->swr);
    swr_init(d->swr);
    d->pending_frames = 0;
    d->pending_offset = 0;
    d->eof = 0;
    d->eof_sent = 0;
    return 0;
}

double musika_decoder_duration(MusikaDecoder *d)
{
    return d ? d->duration : 0.0;
}

void musika_decoder_close(MusikaDecoder *d)
{
    if (!d)
        return;
    av_frame_free(&d->frame);
    av_packet_free(&d->packet);
    swr_free(&d->swr);
    avcodec_free_context(&d->codec);
    avformat_close_input(&d->format);
    if (d->avio) {
        av_freep(&d->avio->buffer);
        avio_context_free(&d->avio);
    }
    free(d->buffer_io);
    free(d->pcm);
    free(d);
}
