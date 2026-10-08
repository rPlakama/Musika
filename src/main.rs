use qtbridge::{QApp, qobject};
use std::sync::OnceLock;
use tokio::runtime::Runtime;
mod http_client;
mod sqlite;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

#[derive(Default)]
pub struct Backend {}

#[qobject(Singleton)]
impl Backend {
    #[qslot]

    fn login_in(&self) {
        let mut username = String::new(); // Init usr
        let mut password = String::new(); // Init Passwd
    }
}

fn main() {
    QApp::new()
        .register::<Backend>()
        .load_qml(include_bytes!("qml/Main.qml"))
        .run();
}
