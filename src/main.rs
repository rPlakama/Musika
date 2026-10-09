use qtbridge::QApp;
use sqlx::SqlitePool;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

mod crypto;
mod http_client;
mod qml;
mod sqlite;

static POOL: OnceLock<SqlitePool> = OnceLock::new();

pub fn pool() -> Option<&'static SqlitePool> {
    POOL.get()
}

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

fn main() {
    dotenvy::dotenv().ok();

    let pool = runtime()
        .block_on(sqlite::init_pool())
        .expect("failed to initialise the database");

    if POOL.set(pool).is_err() {
        eprintln!("database pool was already initialised");
    }

    QApp::new()
        .register::<qml::Backend>()
        .load_qml(include_bytes!("qml/Main.qml"))
        .run();
}
