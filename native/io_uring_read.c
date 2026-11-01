/*
 * Musika — a keyboard-driven music player.
 * Copyright (C) 2026 rPlakama <rPlakama@proton.me>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

/* Reusable liburing file reader.  One ring lives for the reader's lifetime;
 * a batch opens its files and submits all reads before draining CQEs. */

#include <errno.h>
#include <fcntl.h>
#include <liburing.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

typedef struct {
    unsigned char *data;
    size_t len;
    int error;
} MusikaUringResult;

typedef struct {
    struct io_uring ring;
    unsigned entries;
} MusikaUringReader;

typedef struct {
    int fd;
    size_t len;
    size_t offset;
    unsigned char *buf;
} Slot;

MusikaUringReader *musika_uring_reader_new(unsigned entries, char *err, size_t err_len);
void musika_uring_reader_free(MusikaUringReader *reader);
int musika_uring_read_many(MusikaUringReader *reader, const char *const *paths, size_t count,
                           MusikaUringResult *results);
void musika_uring_result_free(MusikaUringResult *result);

static void fail(char *err, size_t err_len, const char *message)
{
    if (err && err_len)
        snprintf(err, err_len, "%s", message);
}

MusikaUringReader *musika_uring_reader_new(unsigned entries, char *err, size_t err_len)
{
    if (err && err_len)
        err[0] = '\0';
    if (entries == 0)
        entries = 64;

    MusikaUringReader *reader = calloc(1, sizeof(*reader));
    if (!reader) {
        fail(err, err_len, "reader allocation failed");
        return NULL;
    }
    int ret = io_uring_queue_init(entries, &reader->ring, 0);
    if (ret < 0) {
        fail(err, err_len, "io_uring queue init failed");
        free(reader);
        return NULL;
    }
    reader->entries = entries;
    return reader;
}

void musika_uring_reader_free(MusikaUringReader *reader)
{
    if (!reader)
        return;
    io_uring_queue_exit(&reader->ring);
    free(reader);
}

void musika_uring_result_free(MusikaUringResult *result)
{
    if (!result)
        return;
    free(result->data);
    result->data = NULL;
    result->len = 0;
    result->error = 0;
}

int musika_uring_read_many(MusikaUringReader *reader, const char *const *paths, size_t count,
                           MusikaUringResult *results)
{
    if (!reader || !paths || (!results && count > 0))
        return -EINVAL;

    for (size_t chunk_start = 0; chunk_start < count; chunk_start += reader->entries) {
        size_t chunk_count = count - chunk_start;
        if (chunk_count > reader->entries)
            chunk_count = reader->entries;
        Slot *slots = calloc(chunk_count, sizeof(*slots));
        if (!slots)
            return -ENOMEM;
        for (size_t i = 0; i < chunk_count; ++i)
            slots[i].fd = -1;

        size_t submitted = 0;
        for (size_t i = 0; i < chunk_count; ++i) {
            size_t result_index = chunk_start + i;
            results[result_index].data = NULL;
            results[result_index].len = 0;
            results[result_index].error = 0;

            const char *path = paths[result_index];
            int fd = path ? open(path, O_RDONLY | O_CLOEXEC) : -1;
            if (fd < 0) {
                results[result_index].error = -(path ? errno : EINVAL);
                continue;
            }
            struct stat st;
            if (fstat(fd, &st) != 0) {
                results[result_index].error = -errno;
                close(fd);
                continue;
            }
            if (st.st_size <= 0) {
                close(fd);
                continue;
            }
            size_t len = (size_t)st.st_size;
            if (len > (size_t)UINT32_MAX) {
                results[result_index].error = -EOVERFLOW;
                close(fd);
                continue;
            }
            unsigned char *buf = malloc(len);
            if (!buf) {
                results[result_index].error = -ENOMEM;
                close(fd);
                continue;
            }
            struct io_uring_sqe *sqe = io_uring_get_sqe(&reader->ring);
            if (!sqe) {
                free(buf);
                close(fd);
                results[result_index].error = -EAGAIN;
                continue;
            }
            slots[i].fd = fd;
            slots[i].len = len;
            slots[i].buf = buf;
            io_uring_prep_read(sqe, fd, buf, (unsigned)len, 0);
            sqe->user_data = (uint64_t)result_index;
            ++submitted;
        }

        if (submitted > 0) {
            int ret = io_uring_submit(&reader->ring);
            if (ret < 0) {
                for (size_t i = 0; i < chunk_count; ++i) {
                    if (slots[i].buf)
                        results[chunk_start + i].error = ret;
                }
            } else {
                size_t completed = 0;
                while (completed < submitted) {
                    struct io_uring_cqe *cqe = NULL;
                    if (io_uring_wait_cqe(&reader->ring, &cqe) < 0)
                        break;
                    size_t result_index = (size_t)cqe->user_data;
                    int read_result = cqe->res;
                    io_uring_cqe_seen(&reader->ring, cqe);
                    ++completed;
                    Slot *slot = &slots[result_index - chunk_start];
                    if (slot->buf == NULL)
                        continue;
                    if (read_result < 0)
                        results[result_index].error = read_result;
                    else
                        results[result_index].len = (size_t)read_result;
                }
            }
        }

        for (size_t i = 0; i < chunk_count; ++i) {
            size_t result_index = chunk_start + i;
            Slot *slot = &slots[i];
            if (slot->fd >= 0)
                close(slot->fd);
            if (slot->buf != NULL) {
                if (results[result_index].error < 0 || results[result_index].len == 0) {
                    free(slot->buf);
                    results[result_index].data = NULL;
                    results[result_index].len = 0;
                } else {
                    results[result_index].data = slot->buf;
                }
                slot->buf = NULL;
            }
        }
        free(slots);
    }
    return 0;
}
