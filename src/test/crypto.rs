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
