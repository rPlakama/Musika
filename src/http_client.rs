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

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use reqwest::Url;
use serde::Deserialize;
use serde::de::DeserializeOwned;

const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "musika";

/// A reusable client for the [Subsonic API](https://www.subsonic.org/pages/api.jsp).
///
/// It owns the server URL and credentials and takes care of the shared
/// authentication query parameters (`u`, `t`, `s`, `v`, `c`, `f`) on every
/// call, so endpoint wrappers only have to name their method and extra params.
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
            http: reqwest::Client::new(),
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
    /// `stream` or `getCoverArt`, whose URLs can be handed straight to QML.
    pub fn endpoint_url(&self, method: &str, params: &[(&str, String)]) -> Result<Url> {
        self.resource_url(&format!("{method}.view"), params)
    }

    /// Builds a URL for an arbitrary `/rest/<resource>` path, including the
    /// authentication parameters. Used for endpoints whose file name is not
    /// `<method>.view` (for example `hls.m3u8`).
    pub fn resource_url(&self, resource: &str, params: &[(&str, String)]) -> Result<Url> {
        let salt = generate_salt();
        let token = md5_token(&self.password, &salt);

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
        self.send_checked(self.http.get(url)).await
    }

    async fn send_checked(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request.send().await.context("Could not reach the server")?;
        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_MODIFIED {
            bail!("Server returned HTTP {status}");
        }
        Ok(response)
    }

    /// Fetches the original media file (`download`) as raw bytes (CACHE-1/8).
    pub async fn download_media(&self, id: &str) -> Result<Vec<u8>> {
        let url = self.endpoint_url("download", &[("id", id.to_string())])?;
        let response = self.get_checked(url).await?;
        let bytes = response
            .bytes()
            .await
            .context("Could not read the media body")?;
        Ok(bytes.to_vec())
    }

    /// Fetches cover art, optionally conditionally.
    ///
    /// When `etag`/`last_modified` are supplied they are sent as
    /// `If-None-Match` / `If-Modified-Since`; the server may then answer
    /// `304 Not Modified`, in which case [`CoverArt::not_modified`] is `true`
    /// and no bytes are transferred.
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

        let mut request = self.http.get(url);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        }

        let response = self.send_checked(request).await?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(CoverArt {
                not_modified: true,
                etag: etag.map(str::to_string),
                last_modified: last_modified.map(str::to_string),
                content_type: None,
                bytes: Vec::new(),
            });
        }

        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        let etag = header(reqwest::header::ETAG);
        let last_modified = header(reqwest::header::LAST_MODIFIED);
        let content_type = header(reqwest::header::CONTENT_TYPE);
        let bytes = response
            .bytes()
            .await
            .context("Could not read cover art body")?
            .to_vec();

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
pub struct CoverArt {
    pub not_modified: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
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
struct Envelope<T> {
    #[serde(rename = "subsonic-response")]
    response: SubsonicResponse<T>,
}

#[derive(Debug, Deserialize)]
struct SubsonicResponse<T> {
    status: String,
    #[serde(default)]
    error: Option<SubsonicError>,
    #[serde(flatten)]
    body: T,
}

impl<T> SubsonicResponse<T> {
    fn into_result(self) -> Result<T> {
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

fn generate_salt() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

fn md5_token(password: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_has_auth_and_extra_params() {
        let client = SubsonicClient::new("navidrome.example.com/", "alice", "secret");
        let url = client
            .endpoint_url("getArtists", &[("musicFolderId", "1".to_string())])
            .unwrap();

        assert_eq!(url.scheme(), "https");
        assert_eq!(url.path(), "/rest/getArtists.view");

        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("u").unwrap(), "alice");
        assert_eq!(query.get("v").unwrap(), API_VERSION);
        assert_eq!(query.get("c").unwrap(), CLIENT_NAME);
        assert_eq!(query.get("f").unwrap(), "json");
        assert_eq!(query.get("musicFolderId").unwrap(), "1");
        assert!(query.contains_key("t") && query.contains_key("s"));
    }

    #[test]
    fn token_is_md5_of_password_plus_salt() {
        let mut expected = Md5::new();
        expected.update(b"secretabc");
        assert_eq!(md5_token("secret", "abc"), hex::encode(expected.finalize()));
    }

    #[test]
    fn ok_envelope_yields_body() {
        let envelope: Envelope<serde_json::Value> =
            serde_json::from_str(r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#)
                .unwrap();
        assert!(envelope.response.into_result().is_ok());
    }

    #[test]
    fn failed_envelope_surfaces_server_message() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("Wrong username or password"));
    }

    #[test]
    fn auth_code_without_message_still_maps_to_credentials_error() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":40}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("Wrong username or password"));
    }

    #[test]
    fn other_errors_are_not_masked_as_auth_failures() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":70}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("code 70"));
        assert!(!err.to_string().contains("username"));
    }
}
