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

//! Subsonic HTTP transport.
//!
//! Compression policy: JSON API responses negotiate gzip (the `reqwest`
//! `gzip` feature is enabled, so the shared client advertises it and
//! transparently decodes the body before the envelope parser sees it).
//! Binary transfers — media downloads and cover art — are already compressed
//! files, so they explicitly ask for `identity`; negotiating an encoding there
//! would only add an inflate step before the bytes reach the decoder.
//!
//! The `zstd` feature remains enabled for backends that prefer it; the server
//! picks one of the advertised encodings.

use anyhow::{Context, Result, bail};
use reqwest::Url;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

pub(crate) const API_VERSION: &str = "1.16.1";
/// Client identification reported to the server (`c=` on every call and
/// the login `ping`). Uppercase on purpose: servers surface it verbatim.
pub(crate) const CLIENT_NAME: &str = "MUSIKA";

/// A stalled transfer (the server stopped sending mid-body) aborts instead of
/// hanging a sync, a cover fetch or a download forever (N-02).
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// JSON API calls are small and interactive: a whole-request deadline keeps a
/// slow-but-not-stalled server from wedging the caller (N-02).
const JSON_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Hard cap on one cover-art body: the bytes are decoded later, and a hostile
/// or misconfigured provider must not be able to OOM the process (N-02).
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Bound on an error body drained for its Subsonic message (N-01).
const MAX_ERROR_BODY_BYTES: u64 = 64 * 1024;
/// Idempotent GETs are retried on transient failures: one flaky response must
/// not abort a whole sync (N-02).
const GET_ATTEMPTS: u32 = 3;
/// Linear backoff between GET attempts.
const GET_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// One process-wide reqwest client: every `SubsonicClient` shares its
/// connection pool and TLS session cache. Cloning a reqwest client is a
/// refcount bump, so per-action clients stay cheap.
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub(crate) fn shared_http_client() -> reqwest::Client {
    HTTP_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .read_timeout(HTTP_READ_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// Marks a binary-transfer request as `identity`.
///
/// Media streams and cover art are compressed formats already; asking for a
/// content encoding would only add an inflate step. `reqwest` only adds its
/// automatic `Accept-Encoding` when the request does not already carry one, so
/// setting this header opts the request out of negotiation.
fn identity_encoding(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request.header(reqwest::header::ACCEPT_ENCODING, "identity")
}

/// A reusable client for the [Subsonic API](https://www.subsonic.org/pages/api.jsp).
///
/// It owns the server URL and credentials and takes care of the shared
/// authentication query parameters (`u`, `t`, `s`, `v`, `c`, `f`) on every
/// call, so endpoint wrappers only have to name their method and extra params.
#[derive(Clone)]
pub struct SubsonicClient {
    base: String,
    username: String,
    password: String,
    http: reqwest::Client,
}

impl SubsonicClient {
    pub fn new(server_url: &str, username: &str, password: &str) -> Self {
        Self {
            base: normalize_base(server_url),
            username: username.to_string(),
            password: password.to_string(),
            http: shared_http_client(),
        }
    }

    /// The normalized server base, for cache fingerprints (CACHE-1).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Builds the full URL for any endpoint (`method` is the name without the
    /// `.view` suffix, e.g. `"getArtists"`), including the authentication
    /// parameters and any extra `params`.
    ///
    /// Useful for endpoints that return something other than JSON, such as
    /// `stream` or `getCoverArt`, whose URLs can be handed straight to legacy UI.
    pub fn endpoint_url(&self, method: &str, params: &[(&str, String)]) -> Result<Url> {
        self.resource_url(&format!("{method}.view"), params)
    }

    /// Builds a URL for an arbitrary `/rest/<resource>` path, including the
    /// authentication parameters. Used for endpoints whose file name is not
    /// `<method>.view` (for example `hls.m3u8`).
    pub fn resource_url(&self, resource: &str, params: &[(&str, String)]) -> Result<Url> {
        let salt = generate_salt()?;
        let token = md5_token(&self.password, &salt)?;

        let mut url = Url::parse(&format!("{}/rest/{resource}", self.base))
            .with_context(|| format!("Invalid server URL: {}", self.base))?;

        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("u", &self.username)
                .append_pair("t", &token)
                .append_pair("s", &salt)
                .append_pair("v", API_VERSION)
                .append_pair("c", CLIENT_NAME)
                .append_pair("f", "json");
            for (key, value) in params {
                query.append_pair(key, value);
            }
        }

        Ok(url)
    }

    /// Calls a JSON endpoint and deserializes its body into `T`.
    ///
    /// For fully typed endpoints define a `Deserialize` struct for the body;
    /// for ad-hoc use, `T` can be [`serde_json::Value`].
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let response = self.get_checked(self.endpoint_url(method, params)?).await?;

        let envelope: Envelope<T> = response
            .json()
            .await
            .context("Server did not return a valid Subsonic response")?;

        envelope.response.into_result()
    }

    async fn get_checked(&self, url: Url) -> Result<reqwest::Response> {
        self.send_checked(self.http.get(url).timeout(JSON_REQUEST_TIMEOUT))
            .await
    }

    /// Sends an idempotent GET, retrying transient failures with a bounded
    /// linear backoff (N-02). The request is cloned per attempt; a request
    /// whose body cannot be cloned (none of ours carry one) is not retried.
    async fn send_checked(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let mut request = request;
        let mut attempt = 1;
        loop {
            let retry = (attempt < GET_ATTEMPTS)
                .then(|| request.try_clone())
                .flatten();
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() || status == reqwest::StatusCode::NOT_MODIFIED {
                        return Ok(response);
                    }
                    if retry.is_none() || !is_transient_status(status) {
                        bail!("Server returned HTTP {status}");
                    }
                    // Drop the failed response before the next attempt so its
                    // connection is released back to the pool.
                    drop(response);
                }
                Err(error) => {
                    if retry.is_none() || !is_transient_error(&error) {
                        return Err(error).context("Could not reach the server");
                    }
                }
            }
            let Some(retry) = retry else {
                break;
            };
            tokio::time::sleep(GET_RETRY_BACKOFF * attempt).await;
            request = retry;
            attempt += 1;
        }
        bail!("Could not reach the server")
    }

    /// Streams the original media file (`download`) into `dest`, returning the
    /// number of bytes written (CACHE-1/8).
    ///
    /// The body is never buffered whole: downloads run several at a time
    /// ([`crate::consts::DOWNLOAD_CONCURRENCY`]) and a long lossless track can
    /// be tens of megabytes, so a `Vec<u8>` per fetch would multiply straight
    /// into the resident set. The caller supplies a `.part` path and adopts it
    /// atomically once the body is complete.
    pub async fn download_media_to(&self, id: &str, dest: &std::path::Path) -> Result<u64> {
        let url = self.endpoint_url("download", &[("id", id.to_string())])?;
        self.download_url_to(url, dest).await
    }

    /// Streams `/rest/stream` with an optional bitrate cap/format into `dest`
    /// (the playback-cache path; downloads still use the original `/download`).
    pub async fn download_stream_to(
        &self,
        id: &str,
        max_bitrate: u32,
        format: Option<&str>,
        dest: &std::path::Path,
    ) -> Result<u64> {
        let mut params: Vec<(&str, String)> = vec![("id", id.to_string())];
        if max_bitrate > 0 {
            params.push(("maxBitRate", max_bitrate.to_string()));
        }
        if let Some(format) = format.filter(|format| !format.is_empty() && *format != "original") {
            params.push(("format", format.to_string()));
        }
        let url = self.endpoint_url("stream", &params)?;
        self.download_url_to(url, dest).await
    }

    async fn download_url_to(&self, url: reqwest::Url, dest: &std::path::Path) -> Result<u64> {
        use tokio::io::AsyncWriteExt;

        let mut response = self
            .send_checked(identity_encoding(self.http.get(url)))
            .await?;
        // Subsonic answers a failed binary fetch with HTTP 200 and a JSON/XML
        // error envelope; streaming that into the `.part` file would adopt an
        // error message as an audio blob (and mark it downloaded) (N-01).
        reject_error_payload(&mut response, "media download").await?;
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("Could not create {}", dest.display()))?;

        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Could not read the media body")?
        {
            file.write_all(&chunk)
                .await
                .with_context(|| format!("Could not write {}", dest.display()))?;
            written += chunk.len() as u64;
        }
        file.flush()
            .await
            .with_context(|| format!("Could not flush {}", dest.display()))?;
        if written == 0 {
            drop(file);
            _ = tokio::fs::remove_file(dest).await;
            bail!("the server returned an empty download");
        }
        Ok(written)
    }

    /// Fetches cover art, optionally conditionally.
    ///
    /// The cached provider date (`last_modified`) is preferred as
    /// `If-Modified-Since`; when it is absent the `etag` is sent as
    /// `If-None-Match`. Sending both would let an unchanged ETag hide a newer
    /// provider date (RFC 9110 evaluates `If-None-Match` first). The server may
    /// answer `304 Not Modified`, in which case [`CoverArt::not_modified`] is
    /// `true` and no bytes are transferred.
    pub async fn get_cover_art(
        &self,
        id: &str,
        size: Option<u32>,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<CoverArt> {
        let mut params: Vec<(&str, String)> = vec![("id", id.to_string())];
        if let Some(size) = size {
            params.push(("size", size.to_string()));
        }
        let url = self.endpoint_url("getCoverArt", &params)?;
        self.get_image_url(url, etag, last_modified).await
    }

    /// Fetches an absolute image URL (artist images are signed `/share/img/`
    /// links outside `getCoverArt`), with the same conditional/response shape.
    pub async fn get_external_image(
        &self,
        url: reqwest::Url,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<CoverArt> {
        self.get_image_url(url, etag, last_modified).await
    }

    async fn get_image_url(
        &self,
        url: reqwest::Url,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<CoverArt> {
        let mut request = identity_encoding(self.http.get(url));
        if let Some(last_modified) = last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        } else if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }

        let mut response = self.send_checked(request).await?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            // A `304` may refresh the validators even though no bytes moved:
            // keep the server's new ETag/Last-Modified over our request copy
            // so the next revalidation compares against the latest version
            // (N-21).
            return Ok(CoverArt {
                not_modified: true,
                etag: header_string(&response, reqwest::header::ETAG)
                    .or_else(|| etag.map(str::to_string)),
                last_modified: header_string(&response, reqwest::header::LAST_MODIFIED)
                    .or_else(|| last_modified.map(str::to_string)),
                content_type: None,
                bytes: Vec::new(),
            });
        }

        // A provider that answers an image request with an HTTP 200 JSON/XML
        // error envelope must never reach the cache as `raw-*.img` (N-01).
        reject_error_payload(&mut response, "cover art").await?;
        if let Some(length) = response.content_length()
            && length > MAX_IMAGE_BYTES
        {
            bail!(
                "cover art is {} bytes, above the {} MiB cap",
                length,
                MAX_IMAGE_BYTES / (1024 * 1024)
            );
        }
        let etag = header_string(&response, reqwest::header::ETAG);
        let last_modified = header_string(&response, reqwest::header::LAST_MODIFIED);
        let content_type = header_string(&response, reqwest::header::CONTENT_TYPE);
        // Chunks into one owned buffer: `bytes().to_vec()` held the whole body
        // twice at the peak, and a provider's original art can be several MB.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Could not read cover art body")?
        {
            if bytes.len() as u64 + chunk.len() as u64 > MAX_IMAGE_BYTES {
                bail!(
                    "cover art exceeds the {} MiB body cap",
                    MAX_IMAGE_BYTES / (1024 * 1024)
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            bail!("the server returned an empty cover-art body");
        }

        Ok(CoverArt {
            not_modified: false,
            etag,
            last_modified,
            content_type,
            bytes,
        })
    }
}

/// Result of a cover-art request. See [`SubsonicClient::get_cover_art`].
#[derive(Debug)]
pub struct CoverArt {
    pub not_modified: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

fn header_string(
    response: &reqwest::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Whether a `Content-Type` names a textual error envelope rather than the
/// binary body a download or an image fetch asked for (N-01).
///
/// `image/svg+xml` is the one XML type that is legitimate art (it is
/// rasterized at ingest), so it is explicitly allowed through.
pub(crate) fn is_error_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "image/svg+xml" {
        return false;
    }
    mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("html")
}

/// Drains (a bounded prefix of) an error body so its Subsonic message can be
/// surfaced instead of the opaque "HTTP 200 with the wrong body" (N-01).
async fn read_error_body(response: &mut reqwest::Response) -> Vec<u8> {
    let mut body = Vec::new();
    // A read error on a body that is already an error envelope is not worth
    // surfacing separately: keep the prefix read so far.
    while let Ok(Some(chunk)) = response.chunk().await {
        if body.len() as u64 + chunk.len() as u64 > MAX_ERROR_BODY_BYTES {
            break;
        }
        body.extend_from_slice(&chunk);
    }
    body
}

/// Rejects an HTTP 200 whose body is a textual error envelope (N-01), with the
/// Subsonic error message when the body carries one.
async fn reject_error_payload(response: &mut reqwest::Response, context: &str) -> Result<()> {
    let content_type = header_string(response, reqwest::header::CONTENT_TYPE);
    if !is_error_content_type(content_type.as_deref()) {
        return Ok(());
    }
    let body = read_error_body(response).await;
    let detail = subsonic_error_message(&body).unwrap_or_else(|| {
        format!(
            "the server answered with {} instead of the requested data",
            content_type.as_deref().unwrap_or("an unknown content type")
        )
    });
    bail!("{context}: {detail}");
}

/// Extracts the Subsonic `error.message` (or its code) from a JSON or XML
/// error envelope carried by an HTTP 200 response (N-01).
pub(crate) fn subsonic_error_message(body: &[u8]) -> Option<String> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
        && let Some(error) = value
            .get("subsonic-response")
            .and_then(|response| response.get("error"))
    {
        if let Some(message) = error
            .get("message")
            .and_then(|message| message.as_str())
            .filter(|message| !message.trim().is_empty())
        {
            return Some(message.to_string());
        }
        if let Some(code) = error.get("code").and_then(|code| code.as_i64()) {
            return Some(format!("Server rejected the request (code {code})"));
        }
        return Some("Server rejected the request".to_string());
    }

    let text = std::str::from_utf8(body).ok()?;
    let element = &text[text.find("<error")?..];
    if let Some(message) = xml_attribute(element, "message") {
        return Some(message);
    }
    xml_attribute(element, "code").map(|code| format!("Server rejected the request (code {code})"))
}

/// Reads one quoted XML attribute from the head of `element` (no XML parser is
/// linked in; Subsonic error envelopes are a single flat element).
fn xml_attribute(element: &str, name: &str) -> Option<String> {
    let pattern = format!("{name}=");
    let start = element.find(&pattern)? + pattern.len();
    let rest = element[start..].trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|quote| *quote == '"' || *quote == '\'')?;
    let value = rest.get(1..)?;
    Some(value[..value.find(quote)?].to_string())
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn is_transient_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

/// What the server reports in the ping envelope: `type` (e.g. `navidrome`,
/// `subsonic`, `gonic`, …) and `serverVersion`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerInfo {
    pub kind: String,
    pub version: String,
}

/// Convenience wrapper used by the login flow.
pub async fn authenticate(server_url: &str, username: &str, password: &str) -> Result<ServerInfo> {
    SubsonicClient::new(server_url, username, password)
        .ping()
        .await
}

/// Extracts the server identity from a ping body.
pub(crate) fn server_info(body: &serde_json::Value) -> ServerInfo {
    ServerInfo {
        kind: body
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        version: body
            .get("serverVersion")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<T> {
    #[serde(rename = "subsonic-response")]
    pub(crate) response: SubsonicResponse<T>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubsonicResponse<T> {
    status: String,
    #[serde(default)]
    error: Option<SubsonicError>,
    #[serde(flatten)]
    body: T,
}

impl<T> SubsonicResponse<T> {
    pub(crate) fn into_result(self) -> Result<T> {
        if self.status.eq_ignore_ascii_case("ok") {
            return Ok(self.body);
        }

        // Only genuine auth codes (40-44) fall back to the credentials
        // message; every other envelope error surfaces its own text (ARC-15).
        let message = match self.error {
            Some(error) => {
                if let Some(message) = error.message.filter(|text| !text.trim().is_empty()) {
                    message
                } else if matches!(error.code, Some(40..=44)) {
                    "Wrong username or password".to_string()
                } else if let Some(code) = error.code {
                    format!("Server rejected the request (code {code})")
                } else {
                    "Server rejected the request".to_string()
                }
            }
            None => "Server rejected the request".to_string(),
        };

        bail!("{message}")
    }
}

#[derive(Debug, Deserialize)]
struct SubsonicError {
    #[serde(default)]
    code: Option<i32>,
    #[serde(default)]
    message: Option<String>,
}

fn normalize_base(server_url: &str) -> String {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

/// Generates a random 32-hex-character Subsonic authentication salt (S-15).
///
/// Subsonic authenticates with `md5(password + salt)`; a predictable salt (the
/// previous implementation used the process nanosecond clock) lets an observer
/// of one request derive the token of a later one. The bytes come from
/// OpenSSL's CSPRNG; a failure to draw them is fatal to the request.
pub(crate) fn generate_salt() -> Result<String> {
    const SALT_BYTES: usize = 16;
    let mut bytes = [0u8; SALT_BYTES];
    openssl::rand::rand_bytes(&mut bytes).context("Could not generate a request salt")?;
    Ok(crate::hex::encode(bytes))
}

pub(crate) fn md5_token(password: &str, salt: &str) -> anyhow::Result<String> {
    crate::digest::md5_token(password, salt)
}
