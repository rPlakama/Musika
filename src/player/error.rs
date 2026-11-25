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

//! Structured playback failures.
//!
//! The engine boundary used to carry bare strings. Recovery, offline
//! decisions and telemetry need to know *what* failed (network, auth, HTTP,
//! decoder, device) and *where* (open/read/seek/sink), so the FFmpeg/worker
//! messages are classified once here and the rest of the tree keeps the
//! human-readable detail for toasts.

use std::fmt;

/// What kind of failure the transport hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorKind {
    /// The server could not be reached at all.
    Network,
    /// Credentials were rejected.
    Auth,
    /// The server answered with an HTTP status.
    Http(u16),
    /// The media could not be demuxed/decoded.
    Decode,
    /// A container/codec/feature the pipeline does not support.
    Unsupported,
    /// The audio device or PipeWire stream failed.
    Device,
    /// The engine itself could not start or answer.
    Engine,
    /// The operation was deliberately superseded (pause/stop/new load).
    Canceled,
    /// Anything else; never retried automatically.
    Other,
}

impl ErrorKind {
    /// Whether a failed track may be reopened at the same position.
    pub fn retryable(self) -> bool {
        match self {
            Self::Network => true,
            Self::Http(status) => status >= 500,
            Self::Auth
            | Self::Decode
            | Self::Unsupported
            | Self::Device
            | Self::Engine
            | Self::Canceled
            | Self::Other => false,
        }
    }
}

/// Where the failure happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorStage {
    Open,
    Read,
    Seek,
    Sink,
    Decode,
    Load,
}

/// One structured playback failure, carrying the detail string every toast
/// and log line already used.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaybackError {
    pub kind: ErrorKind,
    pub stage: ErrorStage,
    pub message: String,
}

impl PlaybackError {
    /// Classifies a decoder/worker message by stage. The message itself is
    /// preserved verbatim for diagnostics and the final toast.
    pub fn classify(stage: ErrorStage, message: impl Into<String>) -> Self {
        let message = message.into();
        let kind = classify_kind(&message);
        Self {
            kind,
            stage,
            message,
        }
    }

    /// Only the kind determines retry: a 404 or a decoder error must never be
    /// retried, while a reset connection should be. A seek that needs a fresh
    /// buffer is always recoverable by reopening the row at the target; it must
    /// never advance the queue just because the current connection cannot jump.
    pub fn retryable(&self) -> bool {
        if self.stage == ErrorStage::Seek
            && !matches!(self.kind, ErrorKind::Auth | ErrorKind::Canceled)
        {
            return true;
        }
        self.kind.retryable()
    }

    pub fn is_network(&self) -> bool {
        self.kind == ErrorKind::Network
    }

    pub fn is_auth(&self) -> bool {
        self.kind == ErrorKind::Auth
    }
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Pulls the HTTP status out of an FFmpeg AVIO/HTTP error string.
fn http_status(message: &str) -> Option<u16> {
    let lower = message.to_ascii_lowercase();
    let rest = lower
        .split_once("server returned ")
        .map(|(_, rest)| rest)
        .or_else(|| lower.split_once("http error ").map(|(_, rest)| rest))
        .or_else(|| lower.split_once("http status ").map(|(_, rest)| rest))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn is_auth_status(lower: &str) -> bool {
    lower.contains("unauthorized")
        || lower.contains("forbidden")
        || [
            "http 401",
            "401 unauthorized",
            "status 401",
            "error 401",
            "code 401",
            "http/1.1 401",
            "http/2 401",
            "http 403",
            "403 forbidden",
            "status 403",
            "error 403",
            "code 403",
            "http/1.1 403",
            "http/2 403",
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// The one classification table. Order matters: HTTP/status and auth are
/// checked before the generic network phrases because messages often contain
/// both ("Server returned 401 Unauthorized").
fn classify_kind(message: &str) -> ErrorKind {
    let lower = message.to_ascii_lowercase();
    if let Some(status) = http_status(message) {
        return match status {
            401 | 403 => ErrorKind::Auth,
            _ => ErrorKind::Http(status),
        };
    }
    if is_auth_status(&lower) {
        return ErrorKind::Auth;
    }
    if lower.contains("connection refused")
        || lower.contains("connection timed out")
        || lower.contains("timed out")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("name or service not known")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("could not resolve")
        || lower.contains("failed to resolve")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("remote end closed")
        || lower.contains("tls fatal alert")
    {
        return ErrorKind::Network;
    }
    if lower.contains("unsupported") {
        return ErrorKind::Unsupported;
    }
    if lower.contains("invalid data")
        || lower.contains("could not find codec")
        || lower.contains("could not open codec")
        || lower.contains("file format not recognized")
        || lower.contains("failed to recognize file format")
        || lower.contains("decoder:")
        || lower.contains("codec")
        || lower.contains("malformed")
    {
        return ErrorKind::Decode;
    }
    if lower.contains("no such device")
        || lower.contains("device busy")
        || lower.contains("could not open/initialize audio device")
        || lower.contains("audio output")
        || lower.contains("ao/")
        || lower.contains("pipewire")
        || lower.contains("alsa")
    {
        return ErrorKind::Device;
    }
    ErrorKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_classification_ignores_numeric_substrings_in_filenames_and_offsets() {
        let local_err = PlaybackError::classify(
            ErrorStage::Read,
            "track_4012.flac: read error at offset 40300",
        );
        assert_ne!(local_err.kind, ErrorKind::Auth);

        let unauthorized =
            PlaybackError::classify(ErrorStage::Open, "Server returned 401 Unauthorized");
        assert_eq!(unauthorized.kind, ErrorKind::Auth);

        let forbidden = PlaybackError::classify(ErrorStage::Open, "HTTP 403 Forbidden");
        assert_eq!(forbidden.kind, ErrorKind::Auth);
    }
}
