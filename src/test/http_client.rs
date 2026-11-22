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
    // The subsonic `c` flag is the client identification servers display.
    assert_eq!(query.get("c").unwrap(), "MUSIKA");
    assert_eq!(CLIENT_NAME, "MUSIKA");
    assert_eq!(query.get("f").unwrap(), "json");
    assert_eq!(query.get("musicFolderId").unwrap(), "1");
    assert!(query.contains_key("t") && query.contains_key("s"));
}

#[test]
fn row_cover_fallback_requests_the_row_size_variant() {
    let client = SubsonicClient::new("navidrome.example.com/", "alice", "secret");
    let url = crate::covers::row_cover_url(&client, "al-1");
    let parsed = reqwest::Url::parse(&url).expect("row cover url");
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(query.get("id").unwrap(), "al-1");
    assert_eq!(
        query.get("size").unwrap(),
        "320",
        "rows should stream the row-sized variant, not the original"
    );
    assert_eq!(parsed.path(), "/rest/getCoverArt.view");

    // The hero fallback keeps the sizeless provider URL on purpose.
    let hero = crate::covers::provider_cover_url(&client, "al-1");
    let hero = reqwest::Url::parse(&hero).expect("hero cover url");
    assert!(!hero.query_pairs().any(|(key, _)| key == "size"));
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
        _ = socket.read(&mut request);
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
    _ = std::fs::remove_dir_all(dir);
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

#[tokio::test]
async fn update_playlist_posts_repeated_song_id_to_add() {
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
        .update_playlist_add("pl1", &["s1".into(), "s2".into()])
        .await
        .expect("update playlist");
    let request = server.join().expect("server thread");
    assert!(request.contains("/rest/updatePlaylist.view"));
    assert!(request.contains("playlistId=pl1"));
    assert!(request.contains("songIdToAdd=s1"));
    assert!(request.contains("songIdToAdd=s2"));
}

#[tokio::test]
async fn update_playlist_posts_repeated_song_index_to_remove() {
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
        .update_playlist_remove_indexes("pl1", &[0, 2])
        .await
        .expect("remove from playlist");
    let request = server.join().expect("server thread");
    assert!(request.contains("/rest/updatePlaylist.view"));
    assert!(request.contains("playlistId=pl1"));
    assert!(request.contains("songIndexToRemove=0"));
    assert!(request.contains("songIndexToRemove=2"));
    assert!(!request.contains("songIdToRemove"));
    assert!(!request.contains("songIdToAdd"));
}

// --- compression negotiation ------------------------------------------------

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// A one-shot HTTP/1.1 server: accepts one loopback connection, captures the
/// request head and answers with `response`. Returns the base URL the client
/// should use and a handle carrying the captured request text.
fn serve_once(response: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
    let (base, handle) = serve_sequence(vec![response]);
    let mapped = std::thread::spawn(move || {
        handle
            .join()
            .expect("loopback server thread")
            .into_iter()
            .next()
            .unwrap_or_default()
    });
    (base, mapped)
}

/// A scripted loopback server: answers the next `responses.len()` connections
/// with the given raw responses in order (used to exercise retry/backoff).
fn serve_sequence(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback server");
    let address = listener.local_addr().expect("loopback address");
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::with_capacity(responses.len());
        for response in responses {
            let mut socket = accept_within(&listener, Duration::from_secs(10));
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("read timeout");
            requests.push(read_request_head(&mut socket));
            socket.write_all(&response).expect("write response");
            _ = socket.flush();
        }
        requests
    });
    (format!("http://{address}"), handle)
}

fn accept_within(listener: &TcpListener, timeout: Duration) -> TcpStream {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((socket, _)) => return socket,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    panic!("loopback server: no connection within {timeout:?}");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(err) => panic!("loopback server: accept failed: {err}"),
        }
    }
}

fn read_request_head(socket: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn response_bytes(content_type: &str, encoding: Option<&str>, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(encoding) = encoding {
        response.push_str(&format!("Content-Encoding: {encoding}\r\n"));
    }
    response.push_str("\r\n");
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// A raw response with an arbitrary status line and headers (304 fixtures, 5xx
/// retry fixtures).
fn raw_response(status_line: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn http_temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "musika-http-{name}-{}-{}",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Header value as sent on the wire (`None` when the request did not send it).
fn sent_header(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip fixture");
    encoder.finish().expect("finish gzip fixture")
}

#[tokio::test]
async fn json_calls_advertise_gzip_and_decode_it() {
    const ENVELOPE: &[u8] = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    let (base, server) = serve_once(response_bytes(
        "application/json",
        Some("gzip"),
        &gzip(ENVELOPE),
    ));
    let client = SubsonicClient::new(&base, "alice", "secret");

    let body: serde_json::Value = client
        .call("ping", &[])
        .await
        .expect("a gzip JSON body decodes transparently");
    assert_eq!(
        body.get("version").and_then(|value| value.as_str()),
        Some("1.16.1")
    );

    let request = server.join().expect("loopback server");
    let encoding = sent_header(&request, "accept-encoding")
        .expect("the JSON request must advertise an Accept-Encoding it can decode");
    assert!(
        encoding.to_ascii_lowercase().contains("gzip"),
        "JSON request advertised {encoding:?}, expected gzip"
    );
}

#[tokio::test]
async fn cover_requests_are_identity_and_pass_the_bytes_through() {
    const COVER: &[u8] = b"\xff\xd8\xff\xe0fake-jpeg-cover";
    let (base, server) = serve_once(response_bytes("image/jpeg", None, COVER));
    let client = SubsonicClient::new(&base, "alice", "secret");

    let art = client
        .get_cover_art("al-1", Some(1024), None, None)
        .await
        .expect("cover art fetch");
    assert_eq!(art.bytes, COVER);
    assert_eq!(art.content_type.as_deref(), Some("image/jpeg"));
    assert!(!art.not_modified);

    let request = server.join().expect("loopback server");
    assert_eq!(
        sent_header(&request, "accept-encoding").as_deref(),
        Some("identity"),
        "cover bytes are already compressed; the request must not negotiate an encoding"
    );
}

#[tokio::test]
async fn media_download_requests_are_identity() {
    const AUDIO: &[u8] = b"fake-flac-bytes";
    let (base, server) = serve_once(response_bytes("audio/flac", None, AUDIO));
    let client = SubsonicClient::new(&base, "alice", "secret");
    let dir = std::env::temp_dir().join(format!(
        "musika-http-media-{}-{}",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp download dir");
    let dest = dir.join("track.flac");

    let written = client
        .download_media_to("s1", &dest)
        .await
        .expect("media download");
    assert_eq!(written, AUDIO.len() as u64);
    assert_eq!(std::fs::read(&dest).expect("downloaded media"), AUDIO);
    _ = std::fs::remove_dir_all(&dir);

    let request = server.join().expect("loopback server");
    assert_eq!(
        sent_header(&request, "accept-encoding").as_deref(),
        Some("identity"),
        "media bytes are already compressed; the request must not negotiate an encoding"
    );
}

// --- N-01: HTTP 200 error envelopes are not binary data ---------------------

/// S-15: the Subsonic salt must come from the CSPRNG, not the clock.
#[test]
fn generate_salt_is_random_hex() {
    let first = generate_salt().expect("a salt");
    assert_eq!(first.len(), 32, "salt is 16 random bytes hex-encoded");
    assert!(
        first.chars().all(|ch| ch.is_ascii_hexdigit()),
        "salt must be hexadecimal: {first:?}"
    );
    let second = generate_salt().expect("a second salt");
    assert_ne!(first, second, "salts must not repeat across calls");
}

#[test]
fn subsonic_error_messages_are_extracted_from_json_and_xml() {
    let json = br#"{"subsonic-response":{"status":"failed","error":{"code":10,"message":"Required parameter is missing"}}}"#;
    assert_eq!(
        subsonic_error_message(json).as_deref(),
        Some("Required parameter is missing")
    );

    let xml = br#"<?xml version="1.0" encoding="UTF-8"?><subsonic-response status="failed"><error code="70" message="Art not found"/></subsonic-response>"#;
    assert_eq!(
        subsonic_error_message(xml).as_deref(),
        Some("Art not found")
    );

    let coded = br#"<subsonic-response status="failed"><error code="40"/></subsonic-response>"#;
    assert!(
        subsonic_error_message(coded).is_some_and(|message| message.contains("40")),
        "an XML error without a message must still surface its code"
    );
    assert!(subsonic_error_message(b"plain audio bytes").is_none());
    assert!(subsonic_error_message(b"").is_none());
}

/// N-01: a failed `/rest/download` answers HTTP 200 with a JSON error envelope.
/// Those bytes must never be adopted as an audio blob.
#[tokio::test]
async fn rejects_json_error_body_in_binary_download() {
    const ERROR: &[u8] = br#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":10,"message":"Required parameter is missing"}}}"#;
    let (base, server) = serve_once(response_bytes("application/json", None, ERROR));
    let client = SubsonicClient::new(&base, "alice", "secret");
    let dir = http_temp_dir("json-error-download");
    let dest = dir.join("song.part");

    let error = client
        .download_media_to("s1", &dest)
        .await
        .expect_err("a JSON error envelope must not be cached as audio");
    let message = format!("{error:#}");
    assert!(
        message.contains("Required parameter is missing"),
        "the Subsonic message must surface, got: {message}"
    );
    assert!(
        !dest.exists(),
        "the rejected body must not be written to disk"
    );

    _ = server.join();
    _ = std::fs::remove_dir_all(dir);
}

/// N-01: an XML error envelope on a cover fetch must not become `raw-*.img`.
#[tokio::test]
async fn rejects_xml_error_body_in_cover_fetch() {
    const ERROR: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><subsonic-response status="failed"><error code="70" message="Art not found"/></subsonic-response>"#;
    let (base, server) = serve_once(response_bytes("text/xml", None, ERROR));
    let client = SubsonicClient::new(&base, "alice", "secret");

    let error = client
        .get_cover_art("al-1", None, None, None)
        .await
        .expect_err("an XML error envelope must not be cached as a cover");
    assert!(
        format!("{error:#}").contains("Art not found"),
        "the Subsonic message must surface, got: {error:#}"
    );

    _ = server.join();
}

/// N-02: an empty 200 body is a failed transfer, not a valid blob.
#[tokio::test]
async fn rejects_empty_download_body() {
    let (base, server) = serve_once(response_bytes("audio/flac", None, b""));
    let client = SubsonicClient::new(&base, "alice", "secret");
    let dir = http_temp_dir("empty-download");
    let dest = dir.join("song.part");

    let error = client
        .download_media_to("s1", &dest)
        .await
        .expect_err("a zero-byte transfer must fail");
    assert!(
        format!("{error:#}").contains("empty"),
        "the error should name the empty body, got: {error:#}"
    );
    assert!(!dest.exists(), "an empty `.part` must not be left behind");

    _ = server.join();
    _ = std::fs::remove_dir_all(dir);
}

/// N-02: one transient 5xx must not abort an idempotent GET.
#[tokio::test]
async fn retries_transient_server_errors_for_idempotent_gets() {
    const ENVELOPE: &[u8] = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    let busy = raw_response(
        "503 Service Unavailable",
        &[("Content-Type", "text/html")],
        b"",
    );
    let ok = response_bytes("application/json", None, ENVELOPE);
    let (base, server) = serve_sequence(vec![busy, ok]);
    let client = SubsonicClient::new(&base, "alice", "secret");

    let body: serde_json::Value = client
        .call("ping", &[])
        .await
        .expect("the retried GET succeeds");
    assert_eq!(
        body.get("version").and_then(|value| value.as_str()),
        Some("1.16.1")
    );

    let requests = server.join().expect("loopback server");
    assert_eq!(
        requests.len(),
        2,
        "one retry for one transient server error"
    );
}

/// N-21: a `304` may refresh the validators even though no bytes moved; the
/// refreshed ETag/Last-Modified must be returned (and fall back to the request
/// values when the server sends none).
#[tokio::test]
async fn not_modified_preserves_refreshed_validators() {
    let refreshed = raw_response(
        "304 Not Modified",
        &[
            ("ETag", "\"v2\""),
            ("Last-Modified", "Wed, 21 Oct 2026 07:28:00 GMT"),
        ],
        b"",
    );
    let bare = raw_response("304 Not Modified", &[], b"");
    let (base, server) = serve_sequence(vec![refreshed, bare]);
    let client = SubsonicClient::new(&base, "alice", "secret");

    let art = client
        .get_cover_art("al-1", None, Some("\"v1\""), None)
        .await
        .expect("304 answer");
    assert!(art.not_modified);
    assert_eq!(art.etag.as_deref(), Some("\"v2\""), "refreshed ETag");
    assert_eq!(
        art.last_modified.as_deref(),
        Some("Wed, 21 Oct 2026 07:28:00 GMT"),
        "refreshed Last-Modified"
    );
    assert!(art.bytes.is_empty());

    let cached = client
        .get_cover_art("al-1", None, Some("\"v3\""), None)
        .await
        .expect("bare 304 answer");
    assert_eq!(
        cached.etag.as_deref(),
        Some("\"v3\""),
        "without a refreshed validator the request copy is kept"
    );

    let requests = server.join().expect("loopback server");
    assert_eq!(
        sent_header(&requests[0], "if-none-match").as_deref(),
        Some("\"v1\"")
    );
    assert_eq!(
        sent_header(&requests[1], "if-none-match").as_deref(),
        Some("\"v3\"")
    );
}
