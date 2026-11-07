use super::common::*;
use crate::hex::*;

#[test]
fn round_trips_bytes() {
    let bytes = [0x00, 0x0f, 0xa5, 0xff, b'm', b'd'];
    assert_eq!(decode(&encode(bytes)).unwrap(), bytes);
}

#[test]
fn accepts_uppercase() {
    assert_eq!(decode("DEADBEEF").unwrap(), [0xde, 0xad, 0xbe, 0xef]);
}

#[test]
fn rejects_bad_input() {
    assert!(decode("abc").is_err());
    assert!(decode("zz").is_err());
}
