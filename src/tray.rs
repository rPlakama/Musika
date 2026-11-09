//! System tray for the headless core daemon.
//!
//! Menu actions are turned into the same textual IPC commands external tools
//! use; the daemon's IPC server executes them. Left-click is `show`.

use std::sync::OnceLock;

use ksni::menu::StandardItem;
use ksni::TrayMethods;
use tokio::sync::Mutex;

struct MusikaTray;

impl ksni::Tray for MusikaTray {
    fn id(&self) -> String {
        "musika".to_string()
    }

    fn title(&self) -> String {
        "Musika".to_string()
    }

    fn icon_name(&self) -> String {
        "musika".to_string()
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
