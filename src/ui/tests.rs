use super::*;

#[cfg(test)]
mod fullscreen_focus_tests {
    use super::FullscreenFocus;

    #[test]
    fn tab_cycles_cover_title_and_lyrics() {
        assert_eq!(FullscreenFocus::Cover.next(), FullscreenFocus::Title);
        assert_eq!(FullscreenFocus::Title.next(), FullscreenFocus::Lyrics);
        assert_eq!(FullscreenFocus::Lyrics.next(), FullscreenFocus::Cover);
        assert_eq!(FullscreenFocus::Cover.prev(), FullscreenFocus::Lyrics);
        assert_eq!(FullscreenFocus::Lyrics.prev(), FullscreenFocus::Title);
        assert_eq!(FullscreenFocus::Title.prev(), FullscreenFocus::Cover);
    }
}

#[cfg(test)]
mod contextual_bar_tests {
    use super::*;
    use crate::ui::widgets::media::cover_source;

    fn page_state(kind: PageKind, starred: bool, cached: bool, downloading: bool) -> PageBarState<'static> {
        PageBarState {
            kind,
            id: "id",
            title: "",
            subtitle: "",
            meta: "",
            rating: 0,
            starred,
            cached,
            downloading,
            radio_seed_kind: None,
        }
    }

    #[test]
    fn contexts_pick_the_right_action_matrix() {
        use TopBarAction::*;
        assert_eq!(topbar_actions_for(SuperiorMode::Home, None), vec![Sync]);
        assert!(topbar_actions_for(SuperiorMode::Playlist, None).is_empty());
        assert_eq!(topbar_actions_for(SuperiorMode::Radio, None), vec![Tool]);

        let neutral = page_state(PageKind::Album, false, false, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&neutral)),
            vec![Tool]
        );
        let favorite = page_state(PageKind::Album, true, false, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&favorite)),
            vec![Favorite, Tool]
        );
        let cached = page_state(PageKind::Album, false, true, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&cached)),
            vec![Download, Tool]
        );
        let both = page_state(PageKind::Album, true, true, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&both)),
            vec![Favorite, Download]
        );

        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&neutral)),
            vec![Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&favorite)),
            vec![Favorite, Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&cached)),
            vec![Download, Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&both)),
            vec![Favorite, Download, Tool]
        );
        assert!(page_tool_visible(&neutral));
        assert!(page_tool_visible(&favorite));
        assert!(page_tool_visible(&cached));
        assert!(!page_tool_visible(&both));
        let downloading = page_state(PageKind::Album, false, true, true);
        assert!(topbar_actions_for(SuperiorMode::Album, Some(&downloading)).contains(&Download));
    }

    #[test]
    fn page_kinds_map_to_contexts() {
        use PageKind;
        assert_eq!(SuperiorMode::from_page(None), SuperiorMode::Home);
        for (kind, mode) in [
            (PageKind::Album, SuperiorMode::Album),
            (PageKind::Artist, SuperiorMode::Artist),
            (PageKind::Playlist, SuperiorMode::Playlist),
            (PageKind::Radio, SuperiorMode::Radio),
        ] {
            let page = PageState::new(PageDescriptor::new(kind, "id"));
            assert_eq!(SuperiorMode::from_page(Some(&page)), mode);
        }
    }

    #[test]
    fn stars_fill_only_the_rating() {
        assert_eq!(AppRoot::star_glyphs(0), [false; 5]);
        assert_eq!(AppRoot::star_glyphs(3), [true, true, true, false, false]);
        assert_eq!(AppRoot::star_glyphs(5), [true; 5]);
        assert_eq!(AppRoot::star_glyphs(9), [true; 5]);
    }

    #[test]
    fn tab_wraps_through_the_action_order() {
        let album = [4, 5, 3];
        assert_eq!(next_tab_slot(&album, None, false), Some(4));
        assert_eq!(next_tab_slot(&album, Some(4), false), Some(5));
        assert_eq!(next_tab_slot(&album, Some(5), false), Some(3));
        assert_eq!(next_tab_slot(&album, Some(3), false), None);
        assert_eq!(next_tab_slot(&album, None, true), Some(3));
        assert_eq!(next_tab_slot(&album, Some(3), true), Some(5));
        assert_eq!(next_tab_slot(&album, Some(5), true), Some(4));
        assert_eq!(next_tab_slot(&album, Some(4), true), None);
        assert_eq!(next_tab_slot(&[], None, false), None);
    }

    #[test]
    fn album_counts_are_pluralized() {
        assert_eq!(crate::bridge::albums_label(0), "0 albums");
        assert_eq!(crate::bridge::albums_label(1), "1 album");
        assert_eq!(crate::bridge::albums_label(4), "4 albums");
    }

    #[test]
    fn remote_covers_are_a_cover_source() {
        assert!(matches!(
            cover_source("https://navidrome.example/rest/getCoverArt.view?id=x"),
            Some(CoverSource::Remote(_))
        ));
        assert!(cover_source("").is_none());
        assert!(remote_cover("http://example.test/a.jpg").is_some());
    }

    #[test]
    fn cached_cover_keeps_its_provider_fallback() {
        let source = CoverSource::Path(std::path::PathBuf::from("/tmp/cover.jpg"))
            .with_remote_fallback("https://provider.test/rest/getCoverArt?id=x");
        match source {
            CoverSource::Fallback { path, remote } => {
                assert_eq!(path, std::path::PathBuf::from("/tmp/cover.jpg"));
                assert_eq!(remote, "https://provider.test/rest/getCoverArt?id=x");
            }
            _ => panic!("expected a cached path with a remote fallback"),
        }

        let source = CoverSource::Remote("https://cache.test/cover.jpg".into())
            .with_remote_fallback("https://provider.test/rest/getCoverArt?id=x");
        assert!(matches!(
            source,
            CoverSource::Remote(url) if url == "https://cache.test/cover.jpg"
        ));
    }

    #[test]
    fn alt_actions_are_thirty_percent_faster() {
        assert!((anim::ALT_MS - anim::EMPHASIZED_MS * 0.7).abs() < f32::EPSILON);
        assert!((anim::ALT_MS - 168.0).abs() < 0.001);
    }
    fn test_search_row(id: &str, title: &str) -> Row {
        Row {
            id: id.to_string(),
            kind: RowKind::Song,
            title: title.to_string(),
            subtitle: String::new(),
            meta: String::new(),
            track: String::new(),
            cover_url: String::new(),
            cover_remote: String::new(),
            song_id: id.to_string(),
            album_id: String::new(),
            starred: false,
            rating: 0,
        }
    }

    fn test_search_state(rows: Vec<Row>) -> SearchState {
        SearchState {
            editor: TextModal::new(),
            rows,
            most_played: false,
            scope: 0,
            scope_cursors: [2, 0, 0, 0],
            selection: state::Selection::default(),
            generation: 1,
            loading: false,
            typing: true,
        }
    }

    #[test]
    fn identical_search_payload_keeps_rows_and_cursor() {
        let rows = vec![test_search_row("a", "A"), test_search_row("b", "B")];
        let mut state = test_search_state(rows.clone());
        assert!(!state.merge_rows(rows.clone(), false));
        assert_eq!(state.rows, rows);
        assert_eq!(state.scope_cursors, [2, 0, 0, 0]);
    }

    #[test]
    fn search_payload_only_replaces_changed_rows() {
        let first = test_search_row("a", "A");
        let second = test_search_row("b", "B");
        let replacement = test_search_row("c", "C");
        let mut state = test_search_state(vec![first.clone(), second]);
        assert!(state.merge_rows(vec![first.clone(), replacement.clone()], false));
        assert_eq!(state.rows, vec![first, replacement]);

        assert!(state.merge_rows(vec![test_search_row("a", "A")], false));
        assert_eq!(state.rows.len(), 1);
        assert!(state.merge_rows(Vec::new(), true));
        assert!(state.most_played);
        assert!(state.rows.is_empty());
    }
}

#[cfg(test)]
mod reading_tests {
    use crate::ui::app::actions::description_offset;

    #[test]
    fn description_offsets_clamp_to_the_content_range() {
        assert_eq!(description_offset(0.0, 22.0, 100.0), 22.0);
        assert_eq!(description_offset(90.0, 22.0, 100.0), 100.0);
        assert_eq!(description_offset(10.0, -22.0, 100.0), 0.0);
        // A reader whose body fits the panel has no scroll range at all.
        assert_eq!(description_offset(0.0, 22.0, -3.0), 0.0);
        assert_eq!(description_offset(0.0, f32::MAX, 140.0), 140.0);
        assert_eq!(description_offset(42.0, -f32::MAX, 140.0), 0.0);
    }
}

#[cfg(test)]
mod hold_tests {
    use crate::ui::app::shell::hold_message;
    use crate::ui::{HOLD_STALE_NS, HOLD_STEP, HOLD_TOAST_TTL_NS, HoldKind, HoldToast, KeyHold};

    fn message(hold: KeyHold, volume: f64) -> String {
        hold_message(hold, volume).0
    }

    #[test]
    fn a_first_press_starts_a_hold() {
        let hold = KeyHold::record(None, '.', 1_000_000_000);
        assert_eq!(hold.kind, HoldKind::Seek);
        assert_eq!(hold.count, 1);
        assert_eq!(hold.delta, HOLD_STEP);
    }

    #[test]
    fn repeats_fold_into_one_hold() {
        let first = KeyHold::record(None, '.', 1_000_000_000);
        let second = KeyHold::record(Some(first), '.', 1_030_000_000);
        let third = KeyHold::record(Some(second), '.', 1_060_000_000);
        assert_eq!(second.count, 2);
        assert_eq!(third.count, 3);
        assert_eq!(third.delta, 3.0 * HOLD_STEP);
    }

    #[test]
    fn alternating_the_pair_keeps_counting() {
        // Spamming `,` and `.` must fold into one bubble instead of restarting,
        // and the pair nets out while the count still adds up.
        let hold = ['.', ',', '.', ',', '.', '.']
            .into_iter()
            .enumerate()
            .fold(None, |previous, (step, key)| {
                Some(KeyHold::record(
                    previous,
                    key,
                    1_000_000_000 + step as u64 * 40_000_000,
                ))
            })
            .expect("fold");
        assert_eq!(hold.kind, HoldKind::Seek);
        assert_eq!(hold.count, 6);
        // four `.` (+20 s) against two `,` (-10 s).
        assert_eq!(hold.delta, 2.0 * HOLD_STEP);
        assert_eq!(message(hold, 0.0), "Seek forward x6 (+10 s)");
    }

    #[test]
    fn a_net_zero_spam_still_reports_its_count() {
        let hold = ['.', ',']
            .into_iter()
            .enumerate()
            .fold(None, |previous, (step, key)| {
                Some(KeyHold::record(
                    previous,
                    key,
                    1_000_000_000 + step as u64 * 40_000_000,
                ))
            })
            .expect("fold");
        assert_eq!(hold.count, 2);
        assert_eq!(hold.delta, 0.0);
        assert_eq!(message(hold, 0.0), "Seek x2 (0 s)");
    }

    #[test]
    fn the_other_family_restarts_the_hold() {
        let hold = KeyHold::record(None, ',', 1_000_000_000);
        let next = KeyHold::record(Some(hold), '+', 1_030_000_000);
        assert_eq!(next.kind, HoldKind::Volume);
        assert_eq!(next.count, 1);
        assert_eq!(next.delta, HOLD_STEP);
    }

    #[test]
    fn a_stale_hold_restarts_instead_of_jumping() {
        // A key-up that never arrived must not turn the next tap into a big jump.
        let hold = KeyHold::record(None, '.', 1_000_000_000);
        let stale = KeyHold::record(Some(hold), '.', 1_000_000_000 + HOLD_STALE_NS);
        assert_eq!(stale.count, 1);
        let fresh = KeyHold::record(Some(hold), '.', 1_000_000_000 + HOLD_STALE_NS - 1);
        assert_eq!(fresh.count, 2);
    }

    #[test]
    fn the_first_auto_repeat_after_the_delay_still_counts() {
        // X11/Wayland wait ~660 ms before the first repeat; it must increment
        // the hold rather than restart it.
        let first = KeyHold::record(None, '.', 1_000_000_000);
        let repeat = KeyHold::record(Some(first), '.', 1_000_000_000 + 660_000_000);
        assert_eq!(repeat.count, 2);
    }

    #[test]
    fn the_bubble_outlives_the_auto_repeat_delay_but_not_forever() {
        // The TTL must cover the press -> first repeat gap (~660 ms) or the
        // bubble blinks, and must still expire after a lost key-up.
        let hold = HoldToast::new("Seek forward (+5 s)".to_string(), None, 1_000_000_000);
        assert!(!hold.expired(1_000_000_000));
        assert!(!hold.expired(1_000_000_000 + 660_000_000));
        assert!(hold.expired(1_000_000_000 + HOLD_TOAST_TTL_NS));
    }

    #[test]
    fn the_bubble_counts_repeats_and_the_distance() {
        let press = |key: char, count: u32| {
            let mut hold = KeyHold::record(None, key, 1_000_000_000);
            for step in 1..count {
                hold = KeyHold::record(Some(hold), key, 1_000_000_000 + u64::from(step) * 30_000_000);
            }
            hold
        };
        assert_eq!(message(press('.', 1), 0.0), "Seek forward (+5 s)");
        assert_eq!(message(press('.', 3), 0.0), "Seek forward x3 (+15 s)");
        assert_eq!(message(press(',', 2), 0.0), "Seek back x2 (-10 s)");
    }

    #[test]
    fn the_bubble_reports_the_clamped_level_and_its_chevron() {
        let press = |key: char, count: u32| {
            let mut hold = KeyHold::record(None, key, 1_000_000_000);
            for step in 1..count {
                hold = KeyHold::record(Some(hold), key, 1_000_000_000 + u64::from(step) * 30_000_000);
            }
            hold
        };
        assert_eq!(message(press('+', 1), 90.0), "Volume up (95%)");
        assert_eq!(message(press('+', 2), 90.0), "Volume up x2 (100%)");
        assert_eq!(message(press('-', 3), 10.0), "Volume down x3 (0%)");
        let (_, icon) = hold_message(press('+', 1), 90.0);
        assert_eq!(icon, Some("up-fill.svg"));
        let (_, icon) = hold_message(press('-', 1), 90.0);
        assert_eq!(icon, Some("down-fill.svg"));
        let (_, icon) = hold_message(press('.', 1), 90.0);
        assert_eq!(icon, None);
    }
}

#[cfg(test)]
mod ipc_toast_tests {
    use super::*;

    #[test]
    fn ipc_echo_names_the_canonical_command() {
        assert_eq!(AppRoot::ipc_toast_text("volume_up"), "IPC · volume_up");
        assert_eq!(
            AppRoot::ipc_toast_text("playlist add selected; \"Mix\""),
            "IPC · playlist add selected; \"Mix\""
        );
    }
}
