use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::Deserialize;

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

    /// Builds the full URL for any endpoint (`method` is the name without the
    /// `.view` suffix, e.g. `"getArtists"`), including the authentication
    /// parameters and any extra `params`.
    ///
    /// Useful for endpoints that return something other than JSON, such as
    /// `stream` or `getCoverArt`, whose URLs can be handed straight to QML.
    pub fn endpoint_url(&self, method: &str, params: &[(&str, String)]) -> Result<Url> {
        let salt = generate_salt();
        let token = md5_token(&self.password, &salt);

        let mut url = Url::parse(&format!("{}/rest/{method}.view", self.base))
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
        let url = self.endpoint_url(method, params)?;

        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("Could not reach the server")?;

        if !response.status().is_success() {
            bail!("Server returned HTTP {}", response.status());
        }

        let envelope: Envelope<T> = response
            .json()
            .await
            .context("Server did not return a valid Subsonic response")?;

        envelope.response.into_result()
    }

    /// Verifies the credentials against the server (`ping.view`).
    pub async fn ping(&self) -> Result<()> {
        self.call::<serde_json::Value>("ping", &[]).await.map(|_| ())
    }

    /// URL for streaming a track, usable directly as a QML media source.
    #[allow(dead_code)] // Reused by playback/cover caching on later roadmap steps.
    pub fn stream_url(&self, id: &str) -> Result<Url> {
        self.endpoint_url("stream", &[("id", id.to_string())])
    }

    /// URL for a cover art image, usable directly as a QML `Image` source.
    #[allow(dead_code)] // Reused by playback/cover caching on later roadmap steps.
    pub fn cover_art_url(&self, id: &str, size: Option<u32>) -> Result<Url> {
        let mut params = vec![("id", id.to_string())];
        if let Some(size) = size {
            params.push(("size", size.to_string()));
        }
        self.endpoint_url("getCoverArt", &params)
    }
}

/// Convenience wrapper used by the login flow.
pub async fn authenticate(server_url: &str, username: &str, password: &str) -> Result<()> {
    SubsonicClient::new(server_url, username, password).ping().await
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

        let message = self
            .error
            .and_then(|error| error.message)
            .unwrap_or_else(|| "Wrong username or password".to_string());

        bail!("{message}")
    }
}

#[derive(Debug, Deserialize)]
struct SubsonicError {
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
}
