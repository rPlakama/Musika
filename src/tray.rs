//! System tray for the headless core daemon.
//!
//! Menu actions are turned into the same textual IPC commands external tools
//! use; the daemon's IPC server executes them. Left-click is `show`.
//!
//! The tray glyph is the runtime SVG asset [`TRAY_ICON`] rasterized at the
//! panel sizes StatusNotifier hosts ask for. The pixmaps are the primary path
//! (a nix-run app cannot rely on the host's icon-theme search path); the
//! package additionally installs the same artwork as the `musika-tray` theme
//! icon for hosts or panel configs that look it up by name.

use std::sync::OnceLock;

use ksni::menu::StandardItem;
use ksni::TrayMethods;
use tokio::sync::Mutex;

/// The tray glyph asset under the runtime asset dir.
const TRAY_ICON: &str = "musika-tray-icon.svg";

/// Panel sizes a StatusNotifier host may request; it picks the closest one.
const TRAY_ICON_SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

/// The runtime asset root: `MUSIKA_ASSET_DIR` (the packaged share dir) or the
/// checkout's `src/` for a plain cargo build. Mirrors the UI's resolver so the
/// tray finds the same icons.
fn asset_root() -> &'static str {
    static ROOT: OnceLock<String> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var("MUSIKA_ASSET_DIR")
            .ok()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| format!("{}/src", env!("CARGO_MANIFEST_DIR")))
    })
}

/// Rasterize the SVG icon at `edge` pixels. The pixels are converted from
/// tiny-skia's premultiplied RGBA to the ARGB32 network order StatusNotifier
/// expects (straight alpha: `[A, R, G, B]`, like `image`'s `rgba().rotate_right(1)`).
fn rasterize_icon(bytes: &[u8], edge: u32) -> Option<ksni::Icon> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if !longest.is_finite() || longest <= 0.0 {
        return None;
    }
    let scale = edge as f32 / longest;
    let width = ((size.width() * scale).round().max(1.0)) as u32;
    let height = ((size.height() * scale).round().max(1.0)) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut data = Vec::with_capacity((width * height * 4) as usize);
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
        width: width as i32,
        height: height as i32,
        data,
    })
}

/// The tray pixmaps, rasterized once. An unreadable asset is not fatal: the
/// themed `icon_name` still resolves and the tray otherwise works.
fn tray_pixmaps() -> &'static [ksni::Icon] {
    static PIXMAPS: OnceLock<Vec<ksni::Icon>> = OnceLock::new();
    PIXMAPS.get_or_init(|| {
        let path = format!("{}/icons/{}", asset_root(), TRAY_ICON);
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
    })
}

struct MusikaTray;

impl ksni::Tray for MusikaTray {
    fn id(&self) -> String {
        "musika".to_string()
    }

    fn title(&self) -> String {
        "Musika".to_string()
    }

    /// Rasterized from the runtime `musika-tray-icon.svg`; `icon_pixmap` is
    /// the portable path (the themed `musika-tray` name only resolves when the
    /// package is installed into the host's icon search path).
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_pixmaps().to_vec()
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

    #[test]
    fn tray_glyph_rasterizes_at_every_panel_size() {
        let icons = tray_pixmaps();
        assert_eq!(
            icons.len(),
            TRAY_ICON_SIZES.len(),
            "the icon asset must exist and parse"
        );
        for (icon, edge) in icons.iter().zip(TRAY_ICON_SIZES) {
            assert_eq!((icon.width, icon.height), (edge as i32, edge as i32));
            assert_eq!(
                icon.data.len() as i32,
                icon.width * icon.height * 4,
                "ARGB32 byte count at {edge}px"
            );
            // White ink on transparency: the record ring is opaque somewhere,
            // and the corner (outside the ring) stays fully transparent.
            assert!(
                icon.data
                    .chunks_exact(4)
                    .any(|pixel| pixel[0] == 255 && pixel[1..] == [255, 255, 255]),
                "the icon is blank at {edge}px"
            );
            assert_eq!(
                &icon.data[..4],
                &[0, 0, 0, 0],
                "transparent corner at {edge}px"
            );
        }
    }
}
