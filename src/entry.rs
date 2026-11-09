//! CLI entry for the `musika` binary.

fn load_dotenv() {
    let Ok(text) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(['"', '\'']);
        if !key.is_empty() && std::env::var_os(key).is_none() {
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

pub fn run() {
    // Keep glibc from creating a new arena per decoder thread.
    #[cfg(target_os = "linux")]
    unsafe {
        unsafe extern "C" {
            fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
        }
        const M_ARENA_MAX: std::ffi::c_int = -8;
        let _ = mallopt(M_ARENA_MAX, 1);
    }

    // CLI probe before any database work, so `musika --version` also works
    // on a headless host.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("musika {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--daemon") {
        crate::daemon::run();
    }
    if args.iter().any(|arg| arg == "--ui") {
        eprintln!("musika --ui: the daemon split is not implemented yet");
        std::process::exit(1);
    }

    if !init_backend() {
        std::process::exit(1);
    }
    crate::ui::run();
}

/// Initializes the shared backend: startup instrumentation, native audio
/// warm-up, dotenv, the account data root, and the database set.
pub(crate) fn init_backend() -> bool {
    crate::startup::start();
    crate::startup::trace("main");
    crate::player::warm_native();
    load_dotenv();

    let data_root = match crate::accounts::startup_data_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("accounts: failed to select data root: {err:#}");
            return false;
        }
    };
    crate::db::set_data_root(data_root);
    crate::db::warm_startup();

    let dbs = match crate::app::runtime().block_on(crate::db::init()) {
        Ok(dbs) => dbs,
        Err(err) => {
            eprintln!("database: failed to initialise: {err:#}");
            return false;
        }
    };
    crate::startup::trace("db.init");
    if let Err(err) = crate::app::runtime().block_on(crate::accounts::migrate_legacy(&dbs)) {
        eprintln!("accounts: legacy migration failed: {err:#}");
    }
    crate::app::set_dbs(dbs);
    true
}
