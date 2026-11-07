use super::*;

fn slot(menu: &QuickMenu, direction: Direction) -> &QuickSlot {
    menu.slot(direction)
        .unwrap_or_else(|| panic!("{direction:?} should have a tile"))
}

fn armed(menu: &QuickMenu) -> AltGesture {
    let mut gesture = AltGesture::new();
    assert!(gesture.arm(menu));
    assert!(gesture.armed);
    gesture
}

#[test]
fn direction_opposites_are_involutive() {
    for direction in Direction::ALL {
        assert_ne!(direction.opposite(), direction);
        assert_eq!(direction.opposite().opposite(), direction);
    }
    assert_eq!(Direction::North.opposite(), Direction::South);
    assert_eq!(Direction::South.opposite(), Direction::North);
    assert_eq!(Direction::East.opposite(), Direction::West);
    assert_eq!(Direction::West.opposite(), Direction::East);

    for (index, direction) in Direction::ALL.into_iter().enumerate() {
        assert_eq!(direction.index(), index);
    }
    assert_eq!(
        AltGesture::default(),
        AltGesture::new(),
        "default is a disarmed gesture"
    );
}

#[test]
fn song_quadrants_match_legacy() {
    let menu = QuickMenu::song();
    for direction in Direction::ALL {
        assert!(!menu.is_open(direction), "{direction:?} is live on a song");
        assert!(!menu.is_disabled(direction));
    }

    let north = slot(&menu, Direction::North);
    assert_eq!(north.label, "Add to Queue");
    assert_eq!(north.icon, "baseline-playlist-add.svg");
    assert_eq!(
        north.actions,
        vec![QuickVerb::AddToQueue, QuickVerb::StartRadio]
    );
    assert!(!north.disabled);
    assert_eq!(north.rating, 0);
    assert!(!north.favorite);
    assert_eq!(north.rating_target, None);

    let south = slot(&menu, Direction::South);
    assert_eq!(south.label, "Favorite");
    assert_eq!(south.icon, "heart-add-fill.svg");
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: true }]);
    assert!(south.favorite);
    assert_eq!(south.rating_target, None);

    let west = slot(&menu, Direction::West);
    assert_eq!(west.label, "Go to Artist");
    assert_eq!(west.icon, "person-walking-svgrepo-com.svg");
    assert_eq!(west.actions, vec![QuickVerb::GoToArtist]);

    let east = slot(&menu, Direction::East);
    assert_eq!(east.label, "Go to Album");
    assert_eq!(east.icon, "album.svg");
    assert_eq!(east.actions, vec![QuickVerb::GoToAlbum]);
}

#[test]
fn starred_song_flips_the_heart() {
    let menu = QuickMenu::song_with(true);
    let south = slot(&menu, Direction::South);
    assert_eq!(south.label, "Unfavorite");
    assert_eq!(south.icon, "heart-minus.svg");
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
    assert!(!south.favorite);
}

#[test]
fn album_quadrants_and_download_toggle() {
    let uncached = QuickMenu::album(false);
    for direction in Direction::ALL {
        assert!(uncached.slot(direction).is_some());
    }
    assert_eq!(
        slot(&uncached, Direction::North).actions,
        vec![QuickVerb::AddToQueue, QuickVerb::StartRadio]
    );
    let south = slot(&uncached, Direction::South);
    assert_eq!(south.label, "Favorite");
    assert_eq!(south.icon, "heart-add-fill.svg");
    assert!(south.favorite);
    assert_eq!(south.rating, 0);
    assert_eq!(south.rating_target, Some(QuickKind::Album));

    let east = slot(&uncached, Direction::East);
    assert_eq!(east.label, "Download");
    assert_eq!(east.icon, "outline-download.svg");
    assert_eq!(east.actions, vec![QuickVerb::Download { on: true }]);

    let cached = QuickMenu::album(true);
    let east = slot(&cached, Direction::East);
    assert_eq!(east.label, "Remove Download");
    assert_eq!(east.icon, "baseline-download.svg");
    assert_eq!(east.actions, vec![QuickVerb::Download { on: false }]);
}

#[test]
fn album_rating_seed_and_favourite_state() {
    let rated = QuickMenu::album_with(false, true, 4);
    let south = slot(&rated, Direction::South);
    assert_eq!(south.rating, 4);
    assert_eq!(south.rating_target, Some(QuickKind::Album));
    assert_eq!(south.label, "Unfavorite");
    assert_eq!(south.icon, "heart-minus.svg");
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
    assert!(!south.favorite);

    // A stale 6+ payload is clamped to the server's 0..=5 range.
    let capped = QuickMenu::album_with(false, false, 9);
    assert_eq!(slot(&capped, Direction::South).rating, 5);
    assert_eq!(
        slot(&capped, Direction::South).rating_target,
        Some(QuickKind::Album)
    );
}

#[test]
fn artist_quadrants_leave_jumps_open() {
    let menu = QuickMenu::artist();
    assert!(menu.is_open(Direction::West));
    assert!(menu.is_open(Direction::East));
    assert!(menu.west.is_none() && menu.east.is_none());

    let north = slot(&menu, Direction::North);
    assert_eq!(north.label, "Add to Queue");
    assert_eq!(north.icon, "baseline-playlist-add.svg");
    assert_eq!(north.actions, vec![QuickVerb::AddToQueue]);

    let south = slot(&menu, Direction::South);
    assert_eq!(south.label, "Favorite");
    assert!(south.favorite);
    assert_eq!(south.rating, 0);
    assert_eq!(south.rating_target, Some(QuickKind::Artist));

    let rated = QuickMenu::artist_with(true, 5);
    let south = slot(&rated, Direction::South);
    assert_eq!(south.label, "Unfavorite");
    assert_eq!(south.rating, 5);
    assert_eq!(south.rating_target, Some(QuickKind::Artist));
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
}

#[test]
fn radio_tool_has_all_four_container_actions() {
    let menu = QuickMenu::radio_tool();
    for direction in Direction::ALL {
        assert!(!menu.is_open(direction));
    }
    let north = slot(&menu, Direction::North);
    assert_eq!(north.actions, vec![QuickVerb::AddToQueue]);
    let south = slot(&menu, Direction::South);
    assert_eq!(south.actions, vec![QuickVerb::SaveRadio]);
    assert_eq!(south.label, "Save Radio");
    assert_eq!(south.icon, "save-outline.svg");
    let west = slot(&menu, Direction::West);
    assert_eq!(west.actions, vec![QuickVerb::ShuffleRadio]);
    let east = slot(&menu, Direction::East);
    assert_eq!(east.actions, vec![QuickVerb::DownloadRadio]);
    assert_eq!(east.label, "Download Radio");
    assert_eq!(east.icon, "outline-download.svg");
}

#[test]
fn playlist_quadrants_leave_jumps_open() {
    let menu = QuickMenu::playlist();
    assert!(menu.is_open(Direction::West));
    assert!(menu.is_open(Direction::East));
    assert_eq!(
        slot(&menu, Direction::North).actions,
        vec![QuickVerb::PlayPlaylist]
    );
    assert_eq!(slot(&menu, Direction::North).label, "Play Playlist");
    assert_eq!(
        slot(&menu, Direction::North).icon,
        "play-1001-svgrepo-com.svg"
    );
    assert_eq!(
        slot(&menu, Direction::South).actions,
        vec![QuickVerb::AddToQueue]
    );
    assert_eq!(slot(&menu, Direction::South).label, "Add to Queue");
    assert_eq!(slot(&menu, Direction::South).rating_target, None);
}

#[test]
fn selection_quadrants_and_disabled_go_to_artist() {
    let menu = QuickMenu::selection(QuickKind::Song, false);
    for direction in Direction::ALL {
        assert!(!menu.is_open(direction));
    }
    assert_eq!(
        slot(&menu, Direction::North).actions,
        vec![QuickVerb::AddToQueue]
    );

    let south = slot(&menu, Direction::South);
    assert_eq!(south.label, "Favorite");
    assert_eq!(south.icon, "heart-add-fill.svg");
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: true }]);
    assert!(south.favorite);
    assert_eq!(south.rating_target, None);

    let west = slot(&menu, Direction::West);
    assert!(west.disabled);
    assert!(!menu.is_open(Direction::West));
    assert!(menu.is_disabled(Direction::West));
    assert_eq!(west.label, "Go to Artist");
    assert_eq!(west.icon, "person-walking-svgrepo-com.svg");

    let east = slot(&menu, Direction::East);
    assert_eq!(east.label, "Download");
    assert_eq!(east.icon, "outline-download.svg");
    assert_eq!(east.actions, vec![QuickVerb::Download { on: true }]);

    let all_starred = QuickMenu::selection(QuickKind::Album, true);
    let south = slot(&all_starred, Direction::South);
    assert_eq!(south.label, "Unfavorite");
    assert_eq!(south.icon, "heart-minus.svg");
    assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
    assert!(!south.favorite);

    // The row kind belongs to the host: the menu itself is identical.
    for kind in [
        QuickKind::Song,
        QuickKind::Album,
        QuickKind::Artist,
        QuickKind::Playlist,
        QuickKind::Selection,
    ] {
        assert_eq!(
            QuickMenu::selection(kind, false),
            QuickMenu::selection(QuickKind::Song, false),
            "{kind:?} selection uses the same verb table"
        );
    }
}

#[test]
fn verb_labels_and_icons_are_legacy_file_names() {
    let cases = [
        (
            QuickVerb::AddToQueue,
            "Add to Queue",
            "baseline-playlist-add.svg",
        ),
        (
            QuickVerb::StartRadio,
            "Start Radio",
            "radio-svgrepo-com.svg",
        ),
        (
            QuickVerb::Favorite { on: true },
            "Favorite",
            "heart-add-fill.svg",
        ),
        (
            QuickVerb::Favorite { on: false },
            "Unfavorite",
            "heart-minus.svg",
        ),
        (QuickVerb::RateAlbum { level: 0 }, "Rate Album", "star.svg"),
        (
            QuickVerb::RateArtist { level: 5 },
            "Rate Artist",
            "star.svg",
        ),
        (
            QuickVerb::GoToArtist,
            "Go to Artist",
            "person-walking-svgrepo-com.svg",
        ),
        (QuickVerb::GoToAlbum, "Go to Album", "album.svg"),
        (
            QuickVerb::Download { on: true },
            "Download",
            "outline-download.svg",
        ),
        (
            QuickVerb::Download { on: false },
            "Remove Download",
            "baseline-download.svg",
        ),
        (
            QuickVerb::PlayPlaylist,
            "Play Playlist",
            "play-1001-svgrepo-com.svg",
        ),
        (QuickVerb::SaveRadio, "Save Radio", "save-outline.svg"),
        (QuickVerb::ShuffleRadio, "Shuffle", "outline-shuffle.svg"),
        (
            QuickVerb::DownloadRadio,
            "Download Radio",
            "outline-download.svg",
        ),
    ];
    for (verb, label, icon) in cases {
        assert_eq!(verb.label(), label, "{verb:?} label");
        assert_eq!(verb.icon(), icon, "{verb:?} icon");
        // legacy UI uses qrc:/app/icons/<file>; the model only keeps the file.
        assert!(!verb.icon().contains("qrc:"), "{verb:?} icon is bare");
    }

    assert_eq!(
        QuickVerb::RateAlbum { level: 2 }.rating_target(),
        Some(QuickKind::Album)
    );
    assert_eq!(
        QuickVerb::RateArtist { level: 2 }.rating_target(),
        Some(QuickKind::Artist)
    );
    assert_eq!(QuickVerb::Favorite { on: true }.rating_target(), None);
}

#[test]
fn slot_helpers_wrap_and_fall_back_to_the_tile_label() {
    let menu = QuickMenu::song();
    let north = slot(&menu, Direction::North);
    assert_eq!(north.active_verb(0), Some(QuickVerb::AddToQueue));
    assert_eq!(north.active_verb(1), Some(QuickVerb::StartRadio));
    assert_eq!(north.active_verb(2), Some(QuickVerb::AddToQueue));
    assert_eq!(north.active_label(1), "Start Radio");
    assert_eq!(north.active_icon(1), "radio-svgrepo-com.svg");
    assert!(!north.is_rating_slot());

    let empty = QuickSlot {
        label: "—".to_string(),
        icon: String::new(),
        actions: Vec::new(),
        ..QuickSlot::default()
    };
    assert_eq!(empty.active_verb(0), None);
    assert_eq!(empty.active_label(0), "—");
    assert_eq!(empty.active_icon(0), "");

    let album = QuickMenu::album(false);
    assert!(slot(&album, Direction::South).is_rating_slot());
}

#[test]
fn menu_slot_accessors_and_overrides() {
    let mut menu = QuickMenu::playlist();
    assert!(menu.is_open(Direction::West));
    assert!(!menu.is_disabled(Direction::West));
    assert!(menu.slot_mut(Direction::North).is_some());
    menu.slot_mut(Direction::North).unwrap().disabled = true;
    assert!(menu.is_disabled(Direction::North));

    menu.set_slot(Direction::North, None);
    assert!(menu.is_open(Direction::North));
    assert!(menu.north.is_none());

    let mut menu = QuickMenu::default();
    let north = single_slot(QuickVerb::PlayPlaylist);
    menu.set_slot(Direction::North, Some(north.clone()));
    assert_eq!(menu.slot(Direction::North), Some(&north));
}

#[test]
fn opposite_aim_passes_through_neutral() {
    let menu = QuickMenu::song();
    let mut gesture = armed(&menu);
    assert_eq!(gesture.aimed, None);

    assert!(gesture.aim(Direction::North, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::North));

    // North + j = neutral; the next j continues to south.
    assert!(!gesture.aim(Direction::South, &menu, false));
    assert_eq!(gesture.aimed, None);
    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::South));

    // Re-aiming the same quadrant stays there.
    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::South));

    for (from, to) in [
        (Direction::North, Direction::South),
        (Direction::South, Direction::North),
        (Direction::East, Direction::West),
        (Direction::West, Direction::East),
    ] {
        let mut gesture = armed(&menu);
        assert!(gesture.aim(from, &menu, false));
        assert!(!gesture.aim(to, &menu, false), "{from:?} -> {to:?}");
        assert_eq!(gesture.aimed, None);
    }
}

#[test]
fn open_slots_cannot_be_aimed() {
    let menu = QuickMenu::artist();
    let mut gesture = armed(&menu);

    assert!(gesture.aim(Direction::North, &menu, false));
    assert!(!gesture.aim(Direction::West, &menu, false));
    assert_eq!(gesture.aimed, None, "open west lands on neutral");

    assert!(!gesture.aim(Direction::East, &menu, false));
    assert_eq!(gesture.aimed, None, "open east lands on neutral");

    // Open south is still a live tile on an artist.
    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::South));

    let playlist = QuickMenu::playlist();
    let mut gesture = armed(&playlist);
    assert!(!gesture.aim(Direction::West, &playlist, false));
    assert!(!gesture.aim(Direction::East, &playlist, false));
    assert_eq!(gesture.aimed, None);
}

#[test]
fn disabled_slots_cannot_be_aimed() {
    let menu = QuickMenu::selection(QuickKind::Song, false);
    let mut gesture = armed(&menu);

    assert!(gesture.aim(Direction::North, &menu, false));
    assert!(!gesture.aim(Direction::West, &menu, false));
    assert_eq!(gesture.aimed, None, "disabled west lands on neutral");

    assert!(!gesture.aim(Direction::West, &menu, false));
    assert_eq!(gesture.aimed, None);

    // A host veto disables an otherwise live direction for this press.
    assert!(!gesture.aim(Direction::South, &menu, true));
    assert_eq!(gesture.aimed, None);
    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::South));
}

#[test]
fn aim_and_cycle_are_ignored_before_arm_or_after_cancel() {
    let menu = QuickMenu::song();
    let mut gesture = AltGesture::new();
    assert!(!gesture.aim(Direction::North, &menu, false));
    assert!(!gesture.cycle(1, &menu));
    assert_eq!(gesture.confirm_plan(&menu), None);
    assert_eq!(gesture.aimed, None);

    assert!(gesture.arm(&menu));
    assert!(gesture.aim(Direction::North, &menu, false));
    assert!(gesture.cycle(1, &menu));
    gesture.cancel();
    assert!(!gesture.armed);
    assert_eq!(gesture.aimed, None);
    assert!(!gesture.aim(Direction::North, &menu, false));
    assert!(!gesture.cycle(1, &menu));
    assert_eq!(gesture.confirm_plan(&menu), None);
}

#[test]
fn cycle_wraps_multi_action_slots() {
    let menu = QuickMenu::song();
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::North, &menu, false));

    assert_eq!(gesture.active_label(&menu), Some("Add to Queue"));
    assert_eq!(
        gesture.active_icon(&menu),
        Some("baseline-playlist-add.svg")
    );

    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 1);
    assert_eq!(gesture.active_verb(&menu), Some(QuickVerb::StartRadio));
    assert_eq!(gesture.active_label(&menu), Some("Start Radio"));
    assert_eq!(gesture.active_icon(&menu), Some("radio-svgrepo-com.svg"));

    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 0);
    assert_eq!(gesture.active_verb(&menu), Some(QuickVerb::AddToQueue));

    // Shift-L at the first entry wraps backwards to the last.
    assert!(gesture.cycle(-1, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 1);

    // Larger deltas wrap through rem_euclid, never panic.
    assert!(gesture.cycle(5, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 0);
    assert!(gesture.cycle(-5, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 1);

    // A zero delta still counts as a handled press.
    assert!(gesture.cycle(0, &menu));
    assert_eq!(gesture.action_index[Direction::North.index()], 1);

    // The per-quadrant indexes are independent.
    assert_eq!(gesture.action_index[Direction::South.index()], 0);
    assert_eq!(gesture.action_index[Direction::East.index()], 0);
    assert_eq!(gesture.action_index[Direction::West.index()], 0);
}

#[test]
fn cycle_is_a_noop_for_single_action_and_unaimed_slots() {
    let menu = QuickMenu::artist();
    let mut gesture = armed(&menu);
    assert!(!gesture.cycle(1, &menu), "neutral cannot cycle");
    assert!(gesture.aim(Direction::North, &menu, false));
    let before = gesture.action_index;
    assert!(!gesture.cycle(1, &menu));
    assert_eq!(gesture.action_index, before);

    let playlist = QuickMenu::playlist();
    let mut gesture = armed(&playlist);
    assert!(gesture.aim(Direction::North, &playlist, false));
    assert!(!gesture.cycle(1, &playlist));
    assert_eq!(gesture.action_index, [0; 4]);

    // A song's favourite is a single verb and has no rating.
    let song = QuickMenu::song();
    let mut gesture = armed(&song);
    assert!(gesture.aim(Direction::South, &song, false));
    assert!(!gesture.cycle(1, &song));
    assert_eq!(gesture.rating_value, 0);
    assert!(!gesture.rating_touched);
    assert!(!gesture.rating_dirty);
}

#[test]
fn rating_steps_clamp_zero_to_five() {
    let menu = QuickMenu::album_with(false, false, 4);
    let mut gesture = armed(&menu);
    assert_eq!(gesture.rating_value, 4, "arm seeds the row's rating");
    assert!(gesture.aim(Direction::South, &menu, false));

    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.rating_value, 5);
    assert!(gesture.rating_touched);
    assert!(gesture.rating_dirty);

    // Clamped at the top, but the step still marks a commit.
    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.rating_value, 5);
    assert!(gesture.rating_touched && gesture.rating_dirty);

    assert!(gesture.cycle(-1, &menu));
    assert_eq!(gesture.rating_value, 4);
    for _ in 0..7 {
        assert!(gesture.cycle(-1, &menu));
    }
    assert_eq!(gesture.rating_value, 0);

    assert!(gesture.cycle(-1, &menu));
    assert_eq!(gesture.rating_value, 0);
    assert!(gesture.cycle(0, &menu));
    assert_eq!(gesture.rating_value, 0);
    assert!(gesture.rating_dirty);

    // The seed is clamped too.
    let capped = QuickMenu::album_with(false, false, 9);
    assert_eq!(armed(&capped).rating_value, 5);
}

#[test]
fn rating_steps_only_on_the_south_favourite_slot() {
    let menu = QuickMenu::artist_with(false, 2);
    let mut gesture = armed(&menu);
    assert_eq!(gesture.rating_value, 2);

    assert!(gesture.aim(Direction::North, &menu, false));
    assert!(!gesture.cycle(1, &menu), "north is a single action");
    assert_eq!(gesture.rating_value, 2);
    assert!(!gesture.rating_touched);
    assert!(!gesture.rating_dirty);

    // South is opposite north: neutral first, then south.
    assert!(!gesture.aim(Direction::South, &menu, false));
    assert!(gesture.aim(Direction::South, &menu, false));
    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.rating_value, 3);
    assert!(gesture.rating_touched);
}

#[test]
fn rating_commit_consumes_the_dirty_flag() {
    let menu = QuickMenu::album_with(false, false, 1);
    let mut gesture = armed(&menu);
    assert_eq!(gesture.commit_rating(&menu), None, "nothing dirty yet");

    assert!(gesture.aim(Direction::South, &menu, false));
    assert!(gesture.cycle(1, &menu));
    assert_eq!(
        gesture.commit_rating(&menu),
        Some(QuickVerb::RateAlbum { level: 2 })
    );
    assert!(!gesture.rating_dirty, "commit clears the flag");
    assert!(gesture.rating_touched, "the hold still remembers the step");
    assert_eq!(gesture.commit_rating(&menu), None, "committed once");
    assert_eq!(
        gesture.confirm_plan(&menu),
        None,
        "a touched south slot does not also toggle the favourite"
    );

    let artist = QuickMenu::artist_with(false, 3);
    let mut gesture = armed(&artist);
    assert!(gesture.aim(Direction::South, &artist, false));
    assert!(gesture.cycle(1, &artist));
    assert_eq!(
        gesture.commit_rating(&artist),
        Some(QuickVerb::RateArtist { level: 4 })
    );

    // A surface without a rating slot never emits a rate verb.
    let song = QuickMenu::song();
    let mut gesture = armed(&song);
    gesture.rating_dirty = true;
    gesture.rating_value = 4;
    assert_eq!(gesture.commit_rating(&song), None);
    assert!(gesture.rating_dirty);
}

#[test]
fn confirm_plan_returns_the_aimed_verb() {
    let menu = QuickMenu::song();
    let mut gesture = armed(&menu);

    assert_eq!(gesture.confirm_plan(&menu), None, "neutral cancels");

    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(
        gesture.confirm_plan(&menu),
        Some(QuickVerb::Favorite { on: true })
    );

    gesture.cancel();
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::North, &menu, false));
    assert!(gesture.cycle(1, &menu));
    assert_eq!(
        gesture.confirm_plan(&menu),
        Some(QuickVerb::StartRadio),
        "confirm sees the cycled action"
    );

    gesture.cancel();
    assert!(!gesture.armed);
    assert_eq!(gesture.confirm_plan(&menu), None);
}

#[test]
fn rating_step_owns_the_favourite_release() {
    let menu = QuickMenu::album_with(false, false, 2);
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::South, &menu, false));
    assert!(gesture.cycle(1, &menu));

    assert_eq!(
        gesture.confirm_plan(&menu),
        Some(QuickVerb::RateAlbum { level: 3 }),
        "the release invokes the rating, not the heart"
    );
    assert!(!gesture.rating_dirty);
    assert_eq!(
        gesture.confirm_plan(&menu),
        None,
        "a clean rating step skips the favourite"
    );
}

#[test]
fn pending_rating_commits_even_when_aiming_away() {
    let menu = QuickMenu::album_with(false, false, 1);
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::South, &menu, false));
    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.rating_value, 2);

    // South -> north is the opposite pass-through.
    assert!(!gesture.aim(Direction::North, &menu, false));
    assert!(gesture.aim(Direction::North, &menu, false));
    assert_eq!(gesture.aimed, Some(Direction::North));

    assert_eq!(
        gesture.confirm_plan(&menu),
        Some(QuickVerb::AddToQueue),
        "north keeps its own verb"
    );
    assert!(gesture.rating_dirty, "the rating is still pending");
    assert_eq!(
        gesture.commit_rating(&menu),
        Some(QuickVerb::RateAlbum { level: 2 }),
        "the caller flushes the rating alongside the aimed verb"
    );
    assert!(!gesture.rating_dirty);
}

#[test]
fn cancel_and_rearm_reset_the_transient_state() {
    let menu = QuickMenu::album_with(false, false, 3);
    let mut gesture = AltGesture::with_sticky(true);
    assert!(gesture.arm(&menu));
    assert!(gesture.aim(Direction::South, &menu, false));
    assert!(gesture.cycle(1, &menu));
    assert_eq!(gesture.action_index, [0; 4]);

    gesture.cancel();
    assert!(!gesture.armed);
    assert_eq!(gesture.aimed, None);
    assert!(!gesture.rating_touched);
    assert!(!gesture.rating_dirty);
    assert!(!gesture.sticky);
    assert!(!gesture.dismissed);

    // Re-arming reseeds the rating and every action index.
    assert!(gesture.arm(&menu));
    assert_eq!(gesture.rating_value, 3);
    assert_eq!(gesture.action_index, [0; 4]);
}

#[test]
fn sticky_boards_survive_alt_release() {
    let menu = QuickMenu::song();
    let mut gesture = AltGesture::with_sticky(true);
    assert!(gesture.arm(&menu));
    assert!(gesture.aim(Direction::South, &menu, false));

    assert!(
        !gesture.release_hold(),
        "sticky menus fire on Enter, not on the Alt release"
    );
    assert!(gesture.armed);
    assert_eq!(gesture.aimed, Some(Direction::South));
    assert!(!gesture.dismissed);

    // Alt release clears the dismissed latch even for a sticky board.
    gesture.dismiss();
    assert!(gesture.dismissed);
    assert!(!gesture.armed);
    assert!(!gesture.release_hold());
    assert!(!gesture.dismissed);

    // Enter on a sticky board confirms explicitly.
    gesture.sticky = true;
    assert!(gesture.arm(&menu));
    assert!(gesture.aim(Direction::South, &menu, false));
    assert_eq!(
        gesture.confirm_plan(&menu),
        Some(QuickVerb::Favorite { on: true })
    );
    gesture.cancel();
    assert!(!gesture.armed);
}

#[test]
fn non_sticky_release_asks_the_host_to_confirm() {
    let menu = QuickMenu::song();
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::North, &menu, false));

    assert!(gesture.release_hold());
    assert!(
        gesture.armed,
        "release_hold leaves confirm/cancel to the host"
    );
    assert_eq!(gesture.confirm_plan(&menu), Some(QuickVerb::AddToQueue));
    gesture.cancel();
    assert!(!gesture.armed);
}

#[test]
fn dismiss_retires_the_hold_until_release() {
    let menu = QuickMenu::song();
    let mut gesture = armed(&menu);
    assert!(gesture.aim(Direction::North, &menu, false));

    gesture.dismiss();
    assert!(gesture.dismissed);
    assert!(!gesture.armed);
    assert_eq!(gesture.aimed, None);

    assert!(!gesture.arm(&menu), "a dismissed hold cannot re-arm");
    assert!(!gesture.release_hold(), "nothing to confirm after Esc");
    assert!(!gesture.dismissed, "the Alt release clears the latch");
    assert!(gesture.arm(&menu), "the next hold may arm again");

    // A new hold that is dismissed can be re-armed after a release.
    gesture.dismiss();
    assert!(!gesture.arm(&menu));
    gesture.clear_dismissed();
    assert!(gesture.arm(&menu));
}

#[test]
fn page_tool_menus_offer_shuffle_for_album_and_artist() {
    let album = QuickMenu::album_tool(false, false, 0);
    assert_eq!(
        album.west.expect("album shuffle").actions,
        vec![QuickVerb::ShuffleAlbum]
    );
    assert_eq!(
        album.east.expect("download").actions,
        vec![QuickVerb::Download { on: true }]
    );

    let artist = QuickMenu::artist_tool(false, false, 0);
    assert_eq!(
        artist.west.expect("artist shuffle").actions,
        vec![QuickVerb::ShuffleArtist]
    );
    assert_eq!(
        artist.east.expect("artist download").actions,
        vec![QuickVerb::Download { on: true }]
    );
}
