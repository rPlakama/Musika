use sqlx::sqlite::SqlitePool;

// First lest receive the writtings for user/password/serverURL

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db = SqlitePool::connect(&std::env::var("DATABASE_URL")?).await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    // Insert
    let username = "placeholder".to_string(); // <-- Receive it from main? Perhaps, or something else.
    let password = "placeholder".to_string(); // <-- same as above
    let server_url = "placeholder".to_string(); // Yeah

    let rec = sqlx::query!(
        "INSERT INTO users (username, password, serverURL) VALUES (?1, ?2, ?3) RETURNING id",
        username,
        password,
        server_url,
    )
    .fetch_one(&db)
    .await?;

    println!("Inserted user id {}", rec.id);

    // Read it back
    let user = sqlx::query!(
        "SELECT id, username, password FROM users WHERE id = ?1",
        rec.id
    )
    .fetch_one(&db)
    .await?;

    println!("Fetched: {} <{}>", user.username, user.password);

    Ok(())
}
