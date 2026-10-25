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

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use openssl::rand::rand_bytes;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};

pub(crate) const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// A 256-bit key used to encrypt secrets before they are written to the
/// database.
///
/// The key lives in `~/.config/musika/key` (hex-encoded, mode `0600`) and is
/// generated on first use. The `MUSIKA_KEY_FILE` environment variable can
/// override the location.
pub struct SecretKey([u8; KEY_LEN]);

impl SecretKey {
    pub fn load() -> Result<Self> {
        let path = key_path()?;

        if path.exists() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read key file {}", path.display()))?;
            let bytes = crate::hex::decode(contents.trim())
                .with_context(|| format!("key file {} is not valid hex", path.display()))?;
            let key: [u8; KEY_LEN] = bytes.try_into().map_err(|_| {
                anyhow!(
                    "key file {} must contain exactly {KEY_LEN} bytes",
                    path.display()
                )
            })?;
            return Ok(Self(key));
        }

        let mut key = [0u8; KEY_LEN];
        rand_bytes(&mut key).context("failed to generate a random key")?;

        crate::db::ensure_parent_dir(&path)?;
        write_private(&path, crate::hex::encode(key).as_bytes())?;

        Ok(Self(key))
    }

    #[cfg(test)]
    pub fn from_bytes(key: [u8; KEY_LEN]) -> Self {
        Self(key)
    }

    /// Encrypts `plaintext` with AES-256-GCM and returns
    /// `hex(nonce || tag || ciphertext)`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut nonce = [0u8; NONCE_LEN];
        rand_bytes(&mut nonce).context("failed to generate a nonce")?;

        let mut tag = [0u8; TAG_LEN];
        let ciphertext = encrypt_aead(
            Cipher::aes_256_gcm(),
            &self.0,
            Some(&nonce),
            &[],
            plaintext.as_bytes(),
            &mut tag,
        )
        .context("failed to encrypt secret")?;

        let mut blob = Vec::with_capacity(NONCE_LEN + TAG_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&tag);
        blob.extend_from_slice(&ciphertext);

        Ok(crate::hex::encode(blob))
    }

    /// Decrypts a value produced by [`SecretKey::encrypt`].
    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        let blob = crate::hex::decode(encoded).context("stored secret is not valid hex")?;
        if blob.len() < NONCE_LEN + TAG_LEN {
            return Err(anyhow!("stored secret is truncated"));
        }

        let (nonce, rest) = blob.split_at(NONCE_LEN);
        let (tag, ciphertext) = rest.split_at(TAG_LEN);

        let plaintext = decrypt_aead(
            Cipher::aes_256_gcm(),
            &self.0,
            Some(nonce),
            &[],
            ciphertext,
            tag,
        )
        .context("failed to decrypt secret")?;

        String::from_utf8(plaintext).context("decrypted secret is not valid UTF-8")
    }
}

/// Process-wide key: the file is read once per run (ARC-9).
static SECRET_KEY: std::sync::OnceLock<SecretKey> = std::sync::OnceLock::new();

/// Test seam: pin the process key so credential round-trips never touch the
/// real config dir (the nix sandbox's `$HOME` is read-only).
#[cfg(test)]
pub fn use_test_key() {
    let _ = SECRET_KEY.set(SecretKey([7u8; KEY_LEN]));
}

fn secret_key() -> Result<&'static SecretKey> {
    if let Some(key) = SECRET_KEY.get() {
        return Ok(key);
    }
    let key = SecretKey::load()?;
    let _ = SECRET_KEY.set(key);
    SECRET_KEY
        .get()
        .ok_or_else(|| anyhow::anyhow!("secret key not initialised"))
}

pub fn encrypt_password(password: &str) -> Result<String> {
    secret_key()?.encrypt(password)
}

pub fn decrypt_password(encoded: &str) -> Result<String> {
    secret_key()?.decrypt(encoded)
}

fn key_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MUSIKA_KEY_FILE") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    // Honor XDG_CONFIG_HOME through the shared config-dir helper (ARC-9).
    Ok(crate::db::config_dir()?.join("key"))
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("failed to create key file {}", path.display()))?;
        file.write_all(data)
            .with_context(|| format!("failed to write key file {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(path, data)
            .with_context(|| format!("failed to write key file {}", path.display()))?;
    }

    Ok(())
}
