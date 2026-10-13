use qtbridge::QApp;
use sqlx::SqlitePool;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

mod api;
mod colors;
mod covers;
mod crypto;
mod db;
mod http_client;
mod index;
mod ipc;
mod keybind;
mod library;
mod lyrics;
mod palette;
mod player;
mod qml;
mod search;
mod sqlite;

static DBS: OnceLock<db::Dbs> = OnceLock::new();

/// The five databases opened at start-up.
pub fn dbs() -> Option<&'static db::Dbs> {
    DBS.get()
}

/// Convenience accessor for the credentials database.
pub fn pool() -> Option<&'static SqlitePool> {
    dbs().map(|dbs| &dbs.credentials)
}

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

fn main() {
    dotenvy::dotenv().ok();

    let dbs = runtime()
        .block_on(db::init())
        .expect("failed to initialise the databases");

    if DBS.set(dbs).is_err() {
        eprintln!("databases were already initialised");
    }

    let mut app = QApp::new();
    qtbridge::include_bytes_qml!("qml", "app");

    app.register::<qml::Backend>()
        .register::<palette::Palette>()
        .load_qml_from_file("qrc:/app/qml/Main.qml")
        .run();
}
