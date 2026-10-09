use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SubsonicEnvelope {
    #[serde(rename = "subsonic-response")]
    subsonic_response: SubsonicResponse,
}

#[derive(Debug, Deserialize)]
struct SubsonicResponse {
    status: String,
    #[serde(default)]
    error: Option<SubsonicError>,
}

#[derive(Debug, Deserialize)]
struct SubsonicError {
    #[serde(default)]
    message: Option<String>,
}

pub async fn authenticate(server_url: &str, username: &str, password: &str) -> Result<()> {
    let base = normalize_base(server_url);
    let salt = generate_salt();
    let token = md5_token(password, &salt);

    let url = reqwest::Url::parse_with_params(
        &format!("{base}/rest/ping.view"),
        &[
            ("u", username),
            ("t", &token),
            ("s", &salt),
            ("v", "1.16.1"),
            ("c", "musika"),
            ("f", "json"),
        ],
    )
    .with_context(|| format!("Invalid server URL: {server_url}"))?;

    let response = reqwest::get(url)
        .await
        .context("Could not reach the server")?;

    if !response.status().is_success() {
        bail!("Server returned HTTP {}", response.status());
    }

    let envelope: SubsonicEnvelope = response
        .json()
        .await
        .context("Server did not return a valid Subsonic response")?;

    if envelope.subsonic_response.status.eq_ignore_ascii_case("ok") {
        return Ok(());
    }

    let message = envelope
        .subsonic_response
        .error
        .and_then(|error| error.message)
        .unwrap_or_else(|| "Wrong username or password".to_string());

    bail!("{message}")
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
