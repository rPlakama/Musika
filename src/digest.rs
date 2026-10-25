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

//! Shared content digests: one MD5 implementation for cache names and the
//! Subsonic token.

use openssl::hash::{Hasher, MessageDigest};

/// Lowercase hex MD5 of one byte slice.
pub(crate) fn md5_hex(bytes: impl AsRef<[u8]>) -> String {
    crate::hex::encode(
        openssl::hash::hash(MessageDigest::md5(), bytes.as_ref()).expect("openssl md5"),
    )
}

/// The Subsonic authentication token: hex MD5 of `password + salt`.
pub(crate) fn md5_token(password: &str, salt: &str) -> String {
    let mut hasher = Hasher::new(MessageDigest::md5()).expect("openssl md5");
    hasher.update(password.as_bytes()).expect("openssl md5");
    hasher.update(salt.as_bytes()).expect("openssl md5");
    crate::hex::encode(hasher.finish().expect("openssl md5"))
}
