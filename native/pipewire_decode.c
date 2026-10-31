/*
 * Musika — a keyboard-driven music player.
 * Copyright (C) 2026 rPlakama <rPlakama@proton.me>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

/* FFmpeg decoder shim for the opt-in PipeWire player.  Output is always
 * interleaved S16 stereo at 48 kHz; the Rust side owns scheduling and the
 * PipeWire ring buffer. */

#include <errno.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
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
};

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

MusikaDecoder *musika_decoder_open(const char *url, char *err, size_t err_len)
{
    if (err && err_len)
        err[0] = '\0';
    if (!url)
        return NULL;

    avformat_network_init();

    MusikaDecoder *d = calloc(1, sizeof(*d));
    if (!d) {
        set_error(err, err_len, "decoder allocation failed");
        return NULL;
    }
    d->stream_index = -1;

    int ret = avformat_open_input(&d->format, url, NULL, NULL);
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
    int64_t target = (int64_t)(seconds * AV_TIME_BASE);
    int ret = av_seek_frame(d->format, -1, target, AVSEEK_FLAG_BACKWARD);
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
    free(d->pcm);
    free(d);
}
