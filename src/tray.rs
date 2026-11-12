//! System tray for the headless core daemon.
//!
//! Menu actions are turned into the same textual IPC commands external tools
//! use; the daemon's IPC server executes them. Left-click is `show`.
//!
//! The icon doubles as a playback indicator: the pause glyph while audio is
//! playing, the play glyph while the core is paused or stopped. Both are the
//! transport's own assets rasterized from the runtime asset dir into the panel
//! sizes StatusNotifier hosts ask for; pixmaps are the primary path because a
//! nix-run app cannot rely on the panel's icon-theme search path.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::menu::StandardItem;
use ksni::TrayMethods;
use tokio::sync::Mutex;

/// The glyph shown while the core is playing (the pause action is available).
const TRAY_GLYPH_PLAYING: &str = crate::assets::ICON_PAUSE;

/// The glyph shown while the core is paused or stopped.
const TRAY_GLYPH_PAUSED: &str = crate::assets::ICON_PLAY;

/// Panel sizes a StatusNotifier host may request; it picks the closest one.
const TRAY_ICON_SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

/// The playback state the tray icon reflects. Written by the daemon from the
/// playback snapshot, read by `icon_pixmap` on the SNI service thread.
static PLAYING: AtomicBool = AtomicBool::new(false);

/// Fraction of the panel slot the glyph fills; the rest is breathing room so
/// the tray glyph matches the padding of the system's other tray icons.
const TRAY_GLYPH_FILL: f32 = 0.82;

/// Rasterize the SVG icon into an `edge`-sized square slot. The pixels are
/// converted from tiny-skia's premultiplied RGBA to the ARGB32 network order
/// StatusNotifier expects (straight alpha: `[A, R, G, B]`, like `image`'s
/// `rgba().rotate_right(1)`).
fn rasterize_icon(bytes: &[u8], edge: u32) -> Option<ksni::Icon> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if !longest.is_finite() || longest <= 0.0 {
        return None;
    }
    let edge = edge.max(1);
    let scale = (edge as f32 * TRAY_GLYPH_FILL) / longest;
    let width = ((size.width() * scale).round().max(1.0) as u32).min(edge);
    let height = ((size.height() * scale).round().max(1.0) as u32).min(edge);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(edge, edge)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (edge as f32 - width as f32) * 0.5,
        (edge as f32 - height as f32) * 0.5,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = Vec::with_capacity((edge * edge * 4) as usize);
    for pixel in pixmap.pixels() {
        let alpha = pixel.alpha();
        // tiny-skia stores premultiplied channels; the SNI wire format is
        // straight ARGB32, so undo the alpha multiplication.
        let channel = |value: u8| {
            if alpha == 0 {
                0
            } else {
                ((u16::from(value) * 255 + u16::from(alpha) / 2) / u16::from(alpha)) as u8
            }
        };
        data.extend_from_slice(&[
            alpha,
            channel(pixel.red()),
            channel(pixel.green()),
            channel(pixel.blue()),
        ]);
    }
    Some(ksni::Icon {
        width: edge as i32,
        height: edge as i32,
        data,
    })
}

/// Rasterize one glyph asset at every panel size. A missing or broken asset is
/// not fatal: the tray still registers with whatever the other state loads.
fn load_pixmaps(asset: &str) -> Vec<ksni::Icon> {
    let path = crate::assets::icon(asset);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("tray: cannot read icon {path}: {error}");
            return Vec::new();
        }
    };
    let icons: Vec<ksni::Icon> = TRAY_ICON_SIZES
        .iter()
        .filter_map(|edge| rasterize_icon(&bytes, *edge))
        .collect();
    if icons.is_empty() {
        eprintln!("tray: cannot rasterize icon {path}");
    }
    icons
}

/// The `(playing, paused)` pixmap ladders, rasterized once.
fn tray_pixmaps(playing: bool) -> &'static [ksni::Icon] {
    static PIXMAPS: OnceLock<(Vec<ksni::Icon>, Vec<ksni::Icon>)> = OnceLock::new();
    let (playing_icons, paused_icons) = PIXMAPS.get_or_init(|| {
        (
            load_pixmaps(TRAY_GLYPH_PLAYING),
            load_pixmaps(TRAY_GLYPH_PAUSED),
        )
    });
    if playing { playing_icons } else { paused_icons }
}

/// Tell the tray whether the core is playing. The daemon calls this from its
/// playback snapshot pump; the icon only repaints when the state flips.
pub fn set_playing(playing: bool) {
    if PLAYING.swap(playing, Ordering::Relaxed) == playing {
        return;
    }
    // ksni diffs the tray properties inside `Handle::update` and emits
    // `NewIcon` when the pixmaps changed, which is how panels learn the glyph
    // flipped between play and pause.
    crate::app::runtime().spawn(async {
        let handle = handle_slot().lock().await.clone();
        if let Some(handle) = handle {
            let _ = handle.update(|_tray| ()).await;
        }
    });
}

struct MusikaTray;

impl ksni::Tray for MusikaTray {
    fn id(&self) -> String {
        "musika".to_string()
    }

    fn title(&self) -> String {
        "Musika".to_string()
    }

    /// Pause bars while playing, play triangle otherwise.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_pixmaps(PLAYING.load(Ordering::Relaxed)).to_vec()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = crate::ipc::send_line("show");
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let item = |label: &str, command: &'static str| {
            StandardItem {
                label: label.to_string(),
                activate: Box::new(move |_| {
                    let _ = crate::ipc::send_line(command);
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Shuffle", "shuffle queue"),
            item("Radio", "radio current"),
            item("Random", "random"),
            item("Quit", "quit"),
        ]
    }
}

fn handle_slot() -> &'static Mutex<Option<ksni::Handle<MusikaTray>>> {
    static HANDLE: OnceLock<Mutex<Option<ksni::Handle<MusikaTray>>>> = OnceLock::new();
    HANDLE.get_or_init(|| Mutex::const_new(None))
}

/// Starts the SNI service once. A missing tray host is not fatal; the daemon
/// still runs and a later `musika` launch can show the UI.
pub fn start() {
    crate::app::runtime().spawn(async {
        let tray = MusikaTray.assume_sni_available(true);
        match tray.spawn().await {
            Ok(handle) => {
                *handle_slot().lock().await = Some(handle);
                eprintln!("tray: StatusNotifierItem registered");
            }
            Err(error) => eprintln!("tray: could not start: {error}"),
        }
    });
}

pub async fn shutdown() {
    if let Some(handle) = handle_slot().lock().await.take() {
        handle.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray as _;

    fn assert_ladder(icons: &[ksni::Icon], label: &str) {
        assert_eq!(
            icons.len(),
            TRAY_ICON_SIZES.len(),
            "the {label} icon asset must exist and parse"
        );
        for (icon, edge) in icons.iter().zip(TRAY_ICON_SIZES) {
            assert_eq!((icon.width, icon.height), (edge as i32, edge as i32));
            assert_eq!(
                icon.data.len() as i32,
                icon.width * icon.height * 4,
                "ARGB32 byte count for {label} at {edge}px"
            );
            // White ink on transparency: the glyph is opaque somewhere, and
            // the corner inside the viewBox padding stays fully transparent.
            assert!(
                icon.data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[0] == 255 && pixel[1..] == [255, 255, 255]),
                "the {label} icon is blank at {edge}px"
            );
            assert_eq!(
                &icon.data[..4],
                &[0, 0, 0, 0],
                "transparent corner for {label} at {edge}px"
            );
        }
    }

    fn center_alpha(icon: &ksni::Icon) -> u8 {
        let x = icon.width as usize / 2;
        let y = icon.height as usize / 2;
        icon.data[(y * icon.width as usize + x) * 4]
    }

    #[test]
    fn tray_glyph_rasterizes_at_every_panel_size() {
        assert_ladder(tray_pixmaps(false), "play");
        assert_ladder(tray_pixmaps(true), "pause");
    }

    #[test]
    fn tray_icon_follows_the_playback_state() {
        // The pause bars leave a transparent gap at the glyph centre; the play
        // triangle covers it. That difference is what the panel repaints on,
        // so it also proves the two assets are not the same file.
        let playing = tray_pixmaps(true);
        let paused = tray_pixmaps(false);
        let largest = TRAY_ICON_SIZES.len() - 1;
        assert_eq!(center_alpha(&playing[largest]), 0, "pause bars have a gap");
        assert_eq!(center_alpha(&paused[largest]), 255, "the triangle is solid");

        set_playing(true);
        assert_eq!(MusikaTray.icon_pixmap()[largest].data, playing[largest].data);
        set_playing(false);
        assert_eq!(MusikaTray.icon_pixmap()[largest].data, paused[largest].data);
    }
}
