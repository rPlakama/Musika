//! The command registry's behavior contract: these samples pin the canonical
//! form of every command family and the registry<->docs coupling, so the
//! registry, parser and canonical printer can be restructured without drift.

use super::common::*;

/// (input, canonical form) pairs: one per command family. The canonical
/// printer normalizes a few inputs (for example `radio` => `radio current`).
const SAMPLES: &[(&str, &str)] = &[
    ("show", "show"),
    ("random", "random"),
    ("quit", "quit"),
    ("status", "status"),
    ("queue", "queue"),
    ("sync", "sync"),
    ("sync all", "sync all"),
    ("sync repair", "sync repair"),
    ("wipe album", "wipe album"),
    ("wipe lyrics", "wipe lyrics"),
    ("wipe covers", "wipe covers"),
    ("wipe_databases", "wipe_databases"),
    ("logout", "logout"),
    ("add_account", "add_account"),
    ("remove_account", "remove_account"),
    ("remove_account --keep-data", "remove_account --keep-data"),
    ("radio", "radio current"),
    ("radio current", "radio current"),
    ("radio album x1", "radio album x1"),
    ("radio song s1", "radio song s1"),
    ("radio save", "radio save"),
    ("radio save --offline", "radio save --offline"),
    ("playlist create \"My Mix\"", "playlist create My Mix"),
    ("playlist create \"My Mix\" --offline", "playlist create My Mix --offline"),
    ("playlist add a1 b2; P", "playlist add a1 b2; P"),
    ("playlist add selected; P", "playlist add selected; P"),
    ("playlist add queue; P", "playlist add queue; P"),
    ("playlist rm; P", "playlist rm; P"),
    ("playlist rm a1; P", "playlist rm a1; P"),
    ("shuffle queue", "shuffle queue"),
    ("shuffle album", "shuffle album"),
    ("shuffle album x1", "shuffle album x1"),
    ("shuffle artist x1", "shuffle artist x1"),
    ("download", "download current"),
    ("download current", "download current"),
    ("download rm current", "download rm current"),
    ("download artist foo", "download artist foo"),
    ("download album \"Kid A\"", "download album \"Kid A\""),
    ("download track x", "download track x"),
    ("download A B C", "download A album B track C"),
    ("refresh", "refresh"),
    ("top", "top"),
    ("bottom", "bottom"),
    ("move_up", "move_up"),
    ("move_down", "move_down"),
    ("scroll_up", "scroll_up"),
    ("scroll_down", "scroll_down"),
    ("page_up", "page_up"),
    ("page_down", "page_down"),
    ("move_left", "move_left"),
    ("move_right", "move_right"),
    ("prev_place", "prev_place"),
    ("next_place", "next_place"),
    ("home", "home"),
    ("back", "back"),
    ("enter", "enter"),
    ("scope_next", "scope_next"),
    ("scope_prev", "scope_prev"),
    ("order_next", "order_next"),
    ("order_prev", "order_prev"),
    ("visual", "visual"),
    ("visual_all", "visual_all"),
    ("queue_toggle", "queue_toggle"),
    ("queue_remove", "queue_remove"),
    ("yank", "yank"),
    ("yank_row", "yank_row"),
    ("paste_below", "paste_below"),
    ("paste_above", "paste_above"),
    ("delete_row", "delete_row"),
    ("queue_up", "queue_up"),
    ("queue_down", "queue_down"),
    ("repeat_cycle", "repeat_cycle"),
    ("toggle_fullscreen", "toggle_fullscreen"),
    ("lyrics_overlay", "lyrics_overlay"),
    ("pane_cycle", "pane_cycle"),
    ("pane_cycle back", "pane_cycle back"),
    ("search foo", "search foo"),
    ("insert_mode bar", "insert_mode bar"),
    ("search_next", "search_next"),
    ("search_prev", "search_prev"),
    ("search_scope_next", "search_scope_next"),
    ("search_scope_prev", "search_scope_prev"),
    ("line_search", "line_search"),
    ("play_pause", "play_pause"),
    ("play", "play"),
    ("pause", "pause"),
    ("stop", "stop"),
    ("track next", "track next"),
    ("track previous", "track previous"),
    ("mute", "mute"),
    ("volume_up", "volume_up"),
    ("volume_down", "volume_down"),
    ("set_volume 42.5", "set_volume 42"),
    ("seek_back", "seek_back"),
    ("seek_forward", "seek_forward"),
    ("seek 12", "seek 12"),
    ("seek_relative -5", "seek_relative -5"),
    ("play_song s1", "play_song s1"),
    ("play_song_in_album s1", "play_song_in_album s1"),
    ("open_song_album s1", "open_song_album s1"),
    ("download_song s1", "download_song s1"),
    ("quick_hold on", "quick_hold on"),
    ("quick_aim north", "quick_aim north"),
    ("quick_cycle next", "quick_cycle next"),
    ("quick_cancel", "quick_cancel"),
];

#[test]
fn samples_parse_and_reprint_canonically() {
    for (sample, expected) in SAMPLES {
        let parsed = crate::ipc_command::parse(sample)
            .unwrap_or_else(|err| panic!("sample {sample:?} must parse: {err:?}"));
        let canonical = crate::ipc_command::canonical_string(&parsed);
        assert_eq!(&canonical, expected, "canonical form of {sample:?}");
        let reparsed = crate::ipc_command::parse(&canonical)
            .unwrap_or_else(|err| panic!("canonical {canonical:?} must reparse: {err:?}"));
        assert_eq!(
            crate::ipc_command::canonical_string(&reparsed),
            canonical,
            "canonical form must be stable for {sample:?}"
        );
    }
}

#[test]
fn parse_errors_keep_their_messages() {
    let cases: &[(&str, &str)] = &[
        ("", "empty command"),
        ("nope", "unknown command: nope"),
        ("mpris_bogus:1", "unknown command: mpris_bogus"),
        ("mpris_volume:101", "volume must be in 0..=100"),
        ("track", "missing argument"),
        ("track sideways", "unknown track command: sideways"),
        ("track next extra", "too many arguments"),
        ("wipe", "missing argument"),
        ("wipe x", "unknown wipe scope: x"),
        ("sync nope", "usage: sync [all|repair]"),
        ("sync all extra", "usage: sync [all|repair]"),
        ("remove_account x", "usage: remove_account [--keep-data]"),
        ("pane_cycle x", "usage: pane_cycle [back]"),
        ("set_volume 101", "volume must be in 0..=100"),
        ("set_volume x", "invalid volume: x"),
        ("seek x", "invalid seconds: x"),
        ("seek_relative x", "invalid seconds: x"),
    ];
    for (input, expected) in cases {
        let err = crate::ipc_command::parse(input)
            .err()
            .unwrap_or_else(|| panic!("{input:?} must fail"));
        assert_eq!(&err.0, expected, "error message for {input:?}");
    }
    // Bare commands keep ignoring trailing words, as the hand-written arms did.
    assert!(crate::ipc_command::parse("play now please").is_ok());
    // `quick_hold` treats any non-`on` value as off, like the old arm did.
    let off = crate::ipc_command::parse("quick_hold maybe").unwrap();
    assert_eq!(crate::ipc_command::canonical_string(&off), "quick_hold off");
}

#[test]
fn documented_commands_are_registered() {
    let docs = std::fs::read_to_string("docs/Commands.md").expect("read docs/Commands.md");
    let mut documented: Vec<String> = Vec::new();
    for line in docs.lines() {
        let Some(rest) = line.strip_prefix("| `") else {
            continue;
        };
        let Some((name, _)) = rest.split_once('`') else {
            continue;
        };
        if name.is_empty() || name.contains(' ') {
            continue;
        }
        documented.push(name.to_string());
    }
    documented.sort();
    documented.dedup();
    assert!(!documented.is_empty(), "the docs table must be found");
    for name in &documented {
        assert!(
            crate::ipc_command::COMMANDS.iter().any(|spec| spec.name == name),
            "documented command {name:?} is not in the registry"
        );
    }
}
