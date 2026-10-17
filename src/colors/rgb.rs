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

use material_colors::color::Rgb;

pub fn argb(color: Rgb) -> u32 {
    color.as_argb_u32()
}

pub fn from_argb(argb: u32) -> Rgb {
    Rgb::new(
        ((argb >> 16) & 0xff) as u8,
        ((argb >> 8) & 0xff) as u8,
        (argb & 0xff) as u8,
    )
}

pub fn hex(color: Rgb) -> String {
    format!("#{}", color.as_hex())
}

pub fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|rgb| 0xFF00_0000 | rgb)
}
