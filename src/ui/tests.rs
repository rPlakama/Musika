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

    fn page_state(kind: PageKind, starred: bool, cached: bool, downloading: bool) -> PageBarState {
        PageBarState {
            kind,
            id: "id".to_string(),
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
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
        assert_eq!(
            topbar_actions_for(SuperiorMode::Home, None),
            vec![Sync, Wipe]
        );
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
        assert_eq!(crate::ui::bridge::albums_label(0), "0 albums");
        assert_eq!(crate::ui::bridge::albums_label(1), "1 album");
        assert_eq!(crate::ui::bridge::albums_label(4), "4 albums");
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
            downloaded: false,
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
