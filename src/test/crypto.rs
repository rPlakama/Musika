use super::common::*;
use crate::crypto::*;

fn test_key() -> SecretKey {
    SecretKey::from_bytes([7u8; KEY_LEN])
}

#[test]
fn round_trip() {
    let key = test_key();
    let encoded = key.encrypt("hunter2").unwrap();

    assert!(crate::hex::decode(&encoded).is_ok());
    assert_eq!(key.decrypt(&encoded).unwrap(), "hunter2");
}

#[test]
fn wrong_key_fails() {
    let encoded = test_key().encrypt("hunter2").unwrap();
    let other = SecretKey::from_bytes([9u8; KEY_LEN]);

    assert!(other.decrypt(&encoded).is_err());
}

#[test]
fn encryption_is_randomised() {
    let key = test_key();

    assert_ne!(key.encrypt("same").unwrap(), key.encrypt("same").unwrap());
}

#[test]
fn concurrent_load_converges_on_single_key_without_corruption() {
    use std::sync::{Arc, Barrier};

    if std::env::var_os("MUSIKA_TEST_CONCURRENT_KEY_CHILD").is_some() {
        let threads = 8;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);
        for idx in 0..threads {
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let key = SecretKey::load().unwrap();
                let msg = format!("payload-{idx}");
                let ct = key.encrypt(&msg).unwrap();
                (key, msg, ct)
            }));
        }
        let results: Vec<(SecretKey, String, String)> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        let reloaded = SecretKey::load().unwrap();
        for (key, _, _) in &results {
            for (_, msg, ct) in &results {
                assert_eq!(key.decrypt(ct).unwrap(), *msg);
                assert_eq!(reloaded.decrypt(ct).unwrap(), *msg);
            }
        }
        return;
    }

    let dir = std::env::temp_dir().join(format!("musika-crypto-race-{}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. In-process concurrent `SecretKey::load_from` on a missing file.
    let inproc_key_path = dir.join("inproc_key");
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::with_capacity(threads);
    for idx in 0..threads {
        let barrier = Arc::clone(&barrier);
        let path = inproc_key_path.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let key = SecretKey::load_from(&path).unwrap();
            let msg = format!("inproc-{idx}");
            let ct = key.encrypt(&msg).unwrap();
            (key, msg, ct)
        }));
    }
    let results: Vec<(SecretKey, String, String)> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    let final_key = SecretKey::load_from(&inproc_key_path).unwrap();
    for (key, _, _) in &results {
        for (_, msg, ct) in &results {
            assert_eq!(key.decrypt(ct).unwrap(), *msg);
            assert_eq!(final_key.decrypt(ct).unwrap(), *msg);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&inproc_key_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // 2. Child subprocess testing `SecretKey::load()` directly via MUSIKA_KEY_FILE.
    let child_key_path = dir.join("child_key");
    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(&exe)
        .env("MUSIKA_KEY_FILE", &child_key_path)
        .env("MUSIKA_TEST_CONCURRENT_KEY_CHILD", "1")
        .args([
            "--exact",
            "test::crypto::concurrent_load_converges_on_single_key_without_corruption",
        ])
        .output()
        .unwrap();
    _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "concurrent SecretKey::load child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
