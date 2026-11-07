use super::common::*;
use crate::http_client::*;

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
    let expected = openssl::hash::hash(MessageDigest::md5(), b"secretabc").unwrap();
    assert_eq!(
        md5_token("secret", "abc").unwrap(),
        crate::hex::encode(expected)
    );
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
    let envelope: Envelope<serde_json::Value> =
        serde_json::from_str(r#"{"subsonic-response":{"status":"failed","error":{"code":40}}}"#)
            .unwrap();

    let err = envelope.response.into_result().unwrap_err();
    assert!(err.to_string().contains("Wrong username or password"));
}

#[test]
fn other_errors_are_not_masked_as_auth_failures() {
    let envelope: Envelope<serde_json::Value> =
        serde_json::from_str(r#"{"subsonic-response":{"status":"failed","error":{"code":70}}}"#)
            .unwrap();

    let err = envelope.response.into_result().unwrap_err();
    assert!(err.to_string().contains("code 70"));
    assert!(!err.to_string().contains("username"));
}

/// The download path streams the body to a file instead of collecting it:
/// a multi-chunk body must arrive intact and the byte count must match.
#[tokio::test]
async fn media_download_streams_chunks_to_the_target_file() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let body: Vec<u8> = (0..200_000).map(|index| (index % 251) as u8).collect();
    let served = body.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut request = [0u8; 4096];
        let _ = socket.read(&mut request);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
            served.len()
        );
        socket.write_all(head.as_bytes()).expect("head");
        for chunk in served.chunks(16 * 1024) {
            socket.write_all(chunk).expect("chunk");
        }
        socket.flush().expect("flush");
    });

    let dir = std::env::temp_dir().join(format!("musika-download-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let dest = dir.join("song.part");

    let client = SubsonicClient::new(&format!("http://{addr}"), "alice", "secret");
    let written = client
        .download_media_to("song-1", &dest)
        .await
        .expect("stream the body");
    server.join().expect("server thread");

    assert_eq!(written as usize, body.len());
    assert_eq!(std::fs::read(&dest).expect("read back"), body);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn create_playlist_posts_repeated_song_ids() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut request = [0u8; 4096];
        let n = socket.read(&mut request).expect("request");
        let request = String::from_utf8_lossy(&request[..n]).to_string();
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(head.as_bytes()).expect("head");
        socket.write_all(body.as_bytes()).expect("body");
        socket.flush().expect("flush");
        request
    });

    let client = SubsonicClient::new(&format!("http://{addr}"), "alice", "secret");
    client
        .create_playlist("Saved Radio", &["s1".into(), "s2".into()])
        .await
        .expect("create playlist");
    let request = server.join().expect("server thread");
    assert!(request.contains("POST") || request.contains("GET"));
    assert!(request.contains("/rest/createPlaylist.view"));
    assert!(request.contains("name=Saved+Radio") || request.contains("name=Saved%20Radio"));
    assert!(request.contains("songId=s1"));
    assert!(request.contains("songId=s2"));
}
