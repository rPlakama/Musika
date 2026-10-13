// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

use crate::crypto;

pub struct Credentials {
    pub username: String,
    pub password: String,
    pub server_url: String,
}

pub async fn save_credentials(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    server_url: &str,
) -> Result<()> {
    let encrypted_password = crypto::encrypt_password(password)?;

    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM users WHERE username = ?1 AND serverURL = ?2")
        .bind(username)
        .bind(server_url)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO users (username, password, serverURL) VALUES (?1, ?2, ?3)")
        .bind(username)
        .bind(encrypted_password)
        .bind(server_url)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn load_credentials(pool: &SqlitePool) -> Result<Option<Credentials>> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT username, password, serverURL FROM users ORDER BY id DESC LIMIT 1")
            .fetch_optional(pool)
            .await?;

    match row {
        Some((username, encrypted_password, server_url)) => {
            let password = crypto::decrypt_password(&encrypted_password)?;
            Ok(Some(Credentials {
                username,
                password,
                server_url,
            }))
        }
        None => Ok(None),
    }
}
