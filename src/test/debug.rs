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

//! Debug-build diagnostics shared by `cargo run` and the test module.
//!
//! The module lives under `src/test/` because it is part of the testing
//! surface, but the debug binary also compiles it so `cargo run` can report
//! Vulkan/GPU information and startup timings. Release builds do not include
//! any of it.

#![allow(dead_code)]

use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Starts the process-wide clock. Call once, after the `--version` probe.
pub(crate) fn start() {
    let _ = PROCESS_START.set(Instant::now());
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

pub(crate) fn report(label: &str, started: Instant) {
    println!("[musika/debug] {label}: {:.2} ms", ms(started));
}

pub(crate) fn report_with(label: &str, started: Instant, detail: &str) {
    println!("[musika/debug] {label}: {:.2} ms ({detail})", ms(started));
}

pub(crate) fn since_start(label: &str) {
    let started = *PROCESS_START.get_or_init(Instant::now);
    report(label, started);
}

pub(crate) fn idle_event(entered: bool) -> &'static str {
    if entered { "enter" } else { "leave" }
}

pub(crate) fn idle(entered: bool) {
    let started = *PROCESS_START.get_or_init(Instant::now);
    println!(
        "[musika/debug] idle.{} at {:.2} ms",
        idle_event(entered),
        ms(started)
    );
}

pub(crate) fn albums_loaded(started: Instant, rows: usize, bytes: usize) {
    report_with(
        "albums.load",
        started,
        &format!("{rows} rows, {bytes} bytes"),
    );
}

fn summary_field<'a>(summary: &'a str, key: &str) -> Option<&'a str> {
    summary.lines().find_map(|line| {
        let value = line.trim().strip_prefix(key)?.trim_start();
        Some(value.strip_prefix('=')?.trim())
    })
}

fn device_kind(kind: &str) -> &'static str {
    match kind {
        "PHYSICAL_DEVICE_TYPE_CPU" => "software/CPU",
        "PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU" => "integrated GPU",
        "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU" => "discrete GPU",
        "PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU" => "virtual GPU",
        _ => "unknown GPU type",
    }
}

/// Hardware/DRM and Vulkan loader diagnostics, one line per fact.
pub(crate) fn diagnostics() -> Vec<String> {
    let mut lines = Vec::new();

    for entry in std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let path = entry.path().join("device/uevent");
        if let Ok(text) = std::fs::read_to_string(path) {
            let driver = text
                .lines()
                .find_map(|line| line.strip_prefix("DRIVER="))
                .unwrap_or("unknown");
            let pci = text
                .lines()
                .find_map(|line| line.strip_prefix("PCI_ID="))
                .unwrap_or("unknown");
            lines.push(format!("DRM {name}: {driver} ({pci})"));
        }
    }

    for entry in std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !name.starts_with("renderD") {
            continue;
        }
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(_) => lines.push(format!("{}: read-write access ok", path.display())),
            Err(err) => lines.push(format!(
                "{}: read-write access failed: {err}",
                path.display()
            )),
        }
    }

    match Command::new("vulkaninfo").arg("--summary").output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(name) = summary_field(&stdout, "deviceName") {
                let kind = summary_field(&stdout, "deviceType")
                    .map(device_kind)
                    .unwrap_or("unknown GPU type");
                let driver = summary_field(&stdout, "driverName").unwrap_or("unknown");
                lines.push(format!(
                    "Vulkan: Available, using {name} ({kind}; driver {driver})"
                ));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let reason = stderr.lines().next_back().unwrap_or("no diagnostics");
                lines.push(format!("Vulkan: unavailable ({reason})"));
            }
        }
        Err(err) => lines.push(format!(
            "Vulkan: unknown (vulkaninfo not found: {err}; install vulkan-tools)"
        )),
    }

    lines
}

pub(crate) fn print_diagnostics() {
    for line in diagnostics() {
        println!("[musika/debug] {line}");
    }
}
