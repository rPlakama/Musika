use qtbridge::QApp;
use std::sync::OnceLock;
use tokio::runtime::Runtime;
mod http_client;
mod qml;
mod sqlite;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

fn main() {
    QApp::new()
        .register::<qml::Backend>() // Wire to qml module;; Theres shall be build a verification if
        // logged is true..
        .load_qml(include_bytes!("qml/Main.qml"))
        .run();
}
