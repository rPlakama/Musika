// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! `sqlx::migrate!` embeds the migration files at compile time but does not
//! ask cargo to watch their directories, so a new or edited migration would
//! not trigger a rebuild and the binary would keep running the old schema.
//! Emit the watch list explicitly (sqlx's recommended build.rs).

fn main() {
    for dir in [
        "migrations",
        "migrations_artist_user",
        "migrations_colors",
        "migrations_covers",
        "migrations_download",
        "migrations_library",
        "migrations_lyrics",
        "migrations_media",
        "migrations_playlist",
        "migrations_search",
        "migrations_status",
        // qtbridge embeds these trees at compile time; without the watch a
        // QML-only edit keeps `cargo run` on the previous engine payload.
        "src/qml",
        "src/icons",
        "native",
    ] {
        println!("cargo:rerun-if-changed={dir}");
    }
    if std::env::var_os("CARGO_FEATURE_PIPEWIRE").is_some() {
        let mut build = cc::Build::new();
        for lib in [
            "libpipewire-0.3",
            "libavformat",
            "libavcodec",
            "libswresample",
            "libavutil",
        ] {
            let output = std::process::Command::new("pkg-config")
                .args(["--cflags", lib])
                .output()
                .unwrap_or_else(|err| panic!("pkg-config failed for {lib}: {err}"));
            for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                if let Some(path) = flag.strip_prefix("-I") {
                    build.include(path);
                }
            }
            let output = std::process::Command::new("pkg-config")
                .args(["--libs-only-L", lib])
                .output()
                .unwrap_or_else(|err| panic!("pkg-config failed for {lib}: {err}"));
            for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                if let Some(path) = flag.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={path}");
                }
            }
        }
        build
            .file("native/pipewire_decode.c")
            .file("native/pipewire_sink.c")
            .compile("pipewire_shims");
        for lib in ["pipewire-0.3", "avformat", "avcodec", "swresample", "avutil"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    if std::env::var_os("CARGO_FEATURE_IO_URING").is_some() {
        cc::Build::new()
            .file("native/io_uring_read.c")
            .compile("io_uring_read");
        println!("cargo:rustc-link-lib=uring");
    }

}
