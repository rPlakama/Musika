/*
 * Musika — a keyboard-driven music player.
 * Copyright (C) 2026 rPlakama <rPlakama@proton.me>
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

/* Narrow liburing file reader used by the cover-load benchmark (and later,
 * if measurements justify it, by the cover pipeline).  The Rust side owns the
 * path and copies the result; this shim owns only the ring and the bytes. */

#include <errno.h>
#include <fcntl.h>
#include <liburing.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

int musika_io_uring_read_file(const char *path, unsigned char **out, size_t *out_len);

void musika_io_uring_free(unsigned char *data);

static int read_all(int fd, unsigned char *buf, size_t len, size_t *read_len)
{
    *read_len = 0;
    struct io_uring ring;
    if (io_uring_queue_init(8, &ring, 0) < 0)
        return -ENOSYS;

    size_t offset = 0;
    int error = 0;
    while (offset < len) {
        struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
        if (!sqe) {
            error = -EAGAIN;
            break;
        }
        io_uring_prep_read(sqe, fd, buf + offset, len - offset, (off_t)offset);
        sqe->user_data = offset;
        if (io_uring_submit(&ring) < 0) {
            error = -EIO;
            break;
        }
        struct io_uring_cqe *cqe = NULL;
        if (io_uring_wait_cqe(&ring, &cqe) < 0) {
            error = -EIO;
            break;
        }
        int result = cqe->res;
        io_uring_cqe_seen(&ring, cqe);
        if (result < 0) {
            error = result;
            break;
        }
        if (result == 0)
            break;
        offset += (size_t)result;
    }

    *read_len = offset;
    io_uring_queue_exit(&ring);
    return error;
}

int musika_io_uring_read_file(const char *path, unsigned char **out, size_t *out_len)
{
    *out = NULL;
    *out_len = 0;
    if (!path || !out || !out_len)
        return -EINVAL;

    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return -errno;

    struct stat st;
    if (fstat(fd, &st) != 0) {
        int error = errno;
        close(fd);
        return -error;
    }
    if (st.st_size <= 0) {
        close(fd);
        return 0;
    }

    size_t len = (size_t)st.st_size;
    unsigned char *buf = malloc(len);
    if (!buf) {
        close(fd);
        return -ENOMEM;
    }

    size_t read_len = 0;
    int error = read_all(fd, buf, len, &read_len);
    close(fd);
    if (error != 0) {
        free(buf);
        return error;
    }
    if (read_len == 0) {
        free(buf);
        return 0;
    }

    *out = buf;
    *out_len = read_len;
    return 0;
}

void musika_io_uring_free(unsigned char *data)
{
    free(data);
}
