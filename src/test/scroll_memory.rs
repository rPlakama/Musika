use crate::scroll_memory::*;

#[test]
fn remembers_clears_and_serializes_one_entry() {
    let key = format!("scroll-test-{}", std::process::id());
    clear(&key);
    remember(&key, 7, 123.5, 2, "grid");
    assert_eq!(saved_index(&key), 7);
    assert_eq!(saved_layout(&key), "grid");
    let entry: serde_json::Value = serde_json::from_str(&entry_json(&key)).expect("entry JSON");
    assert_eq!(entry["index"], 7);
    assert_eq!(entry["y"], 123.5);
    assert_eq!(entry["scope"], 2);
    assert!(serialize().contains(&key));
    clear(&key);
    assert_eq!(saved_index(&key), -1);
    assert!(entry_json(&key).is_empty());
}

#[test]
fn remember_layout_keeps_the_cursor_and_moves_the_layout() {
    let key = format!("scroll-layout-{}", std::process::id());
    clear(&key);
    remember(&key, 4, 10.0, 0, "list");
    remember_layout(&key, "grid");
    assert_eq!(saved_index(&key), 4);
    assert_eq!(saved_layout(&key), "grid");
    clear(&key);
}

#[test]
fn malformed_hydrate_payload_is_ignored() {
    hydrate_raw("{not json");
    hydrate_raw("[]");
    hydrate_raw("null");
}
