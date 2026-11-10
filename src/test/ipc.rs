use super::common::*;
use crate::ipc::*;
use std::io::Write;
use std::time::{Duration, Instant};

fn wait_connect(path: &Path) -> UnixStream {
    let start = Instant::now();
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(err) => {
                assert!(
                    start.elapsed() < Duration::from_secs(2),
                    "ipc socket never came up: {err}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// FIX-9: a client that connects and stays silent must not block a
/// second client's command.
#[test]
fn a_silent_client_does_not_block_later_commands() {
    let dir = std::env::temp_dir().join(format!("musika-ipc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("test.sock");

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    serve(path.clone(), move |line| {
        let _ = tx.send(line);
        None
    });

    let silent = wait_connect(&path);
    let mut second = wait_connect(&path);
    second.write_all(b"ping\n").expect("write command");

    let received = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the second client's command ran");
    assert_eq!(received, "ping");

    drop(silent);
    drop(second);
    let _ = std::fs::remove_dir_all(dir);
}

/// Read-only query commands answer on the same connection; fire-and-forget
/// commands stay silent so `printf ... | nc -U` keeps working.
#[test]
fn query_commands_answer_and_plain_commands_stay_silent() {
    let dir = std::env::temp_dir().join(format!("musika-ipc-query-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("query.sock");

    serve(path.clone(), move |line| {
        (line == "status").then(|| "{\"state\":\"playing\"}".to_string())
    });

    let mut client = wait_connect(&path);
    client.write_all(b"play_pause\n").expect("write command");
    client
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    let mut buf = [0u8; 32];
    let silent = std::io::Read::read(&mut client, &mut buf);
    assert!(
        matches!(silent, Err(_) | Ok(0)),
        "a plain command must not write a reply"
    );

    let mut client = wait_connect(&path);
    client.write_all(b"status\n").expect("write query");
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(client), &mut reply)
        .expect("read reply");
    assert_eq!(reply.trim(), "{\"state\":\"playing\"}");

    let _ = std::fs::remove_dir_all(dir);
}

/// The peer-credential check accepts our own process and refuses nothing else
/// on a same-user pair (the refusal path needs a second uid, so only the
/// positive case is asserted here).
#[test]
fn same_uid_accepts_own_connections() {
    let (left, right) = UnixStream::pair().expect("socket pair");
    assert!(same_uid(&left));
    assert!(same_uid(&right));
}
