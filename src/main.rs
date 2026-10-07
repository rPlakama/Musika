use qtbridge::{QApp, qobject};
use std::sync::OnceLock;
use tokio::runtime::Runtime;
mod http_client;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

#[derive(Default)]
pub struct Backend {}

#[qobject(Singleton)]
impl Backend {
    #[qslot]
    fn say_hello(&self) {
        println!("Hello World!");

        runtime().spawn(http_client::http_call());
    }
}

fn main() {
    QApp::new()
        .register::<Backend>()
        .load_qml(include_bytes!("qml/Main.qml"))
        .run();
}
