use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

pub async fn init_pool() -> Result<SqlitePool> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:app.db".to_string());

    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

pub async fn save_credentials(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    server_url: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM users WHERE username = ?1 AND serverURL = ?2")
        .bind(username)
        .bind(server_url)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO users (username, password, serverURL) VALUES (?1, ?2, ?3)")
        .bind(username)
        .bind(password)
        .bind(server_url)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}
