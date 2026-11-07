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
