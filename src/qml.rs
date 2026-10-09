use qtbridge::qobject;
use sqlx::SqlitePool;
#[derive(Default)]

pub struct Backend {
    server_url: String,
    username: String,
    password: String,
    pool: Option<SqlitePool>,
}

#[qobject(Singleton)]
impl Backend {
    qproperty!(
        "serverURL",
        Member = server_url,
        Write = set_server_url,
        Notify = server_url_changed
    );
    qproperty!(
        "username",
        Member = username,
        Write = set_username,
        Notify = username_changed
    );
    qproperty!(
        "password",
        Member = password,
        Write = set_password,
        Notify = password_changed
    );

    fn set_server_url(&mut self, v: String) {
        self.server_url = v;
        self.server_url_changed();
    }
    fn set_username(&mut self, v: String) {
        self.username = v;
        self.username_changed();
    }
    fn set_password(&mut self, v: String) {
        self.password = v;
        self.password_changed();
    }

    #[qsignal]
    fn server_url_changed(&mut self);
    #[qsignal]
    fn username_changed(&mut self);
    #[qsignal]
    fn password_changed(&mut self);

    #[qslot]
    fn login(&self) {
        let Some(pool) = self.pool.clone() else {
            eprintln!("Login called before pool was set!");
            return;
        };
        let username = self.username.clone();
        let password = self.password.clone();
        let server_url = self.server_url.clone();
    }
}
