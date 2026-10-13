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

const KEY_LEN: usize = 32;
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
            let bytes = hex::decode(contents.trim())
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

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        write_private(&path, hex::encode(key).as_bytes())?;

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

        Ok(hex::encode(blob))
    }

    /// Decrypts a value produced by [`SecretKey::encrypt`].
    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        let blob = hex::decode(encoded).context("stored secret is not valid hex")?;
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

pub fn encrypt_password(password: &str) -> Result<String> {
    SecretKey::load()?.encrypt(password)
}

pub fn decrypt_password(encoded: &str) -> Result<String> {
    SecretKey::load()?.decrypt(encoded)
}

fn key_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MUSIKA_KEY_FILE") {
        return Ok(PathBuf::from(path));
    }

    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("musika")
        .join("key"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> SecretKey {
        SecretKey::from_bytes([7u8; KEY_LEN])
    }

    #[test]
    fn round_trip() {
        let key = test_key();
        let encoded = key.encrypt("hunter2").unwrap();

        assert!(hex::decode(&encoded).is_ok());
        assert_eq!(key.decrypt(&encoded).unwrap(), "hunter2");
    }

    #[test]
    fn wrong_key_fails() {
        let encoded = test_key().encrypt("hunter2").unwrap();
        let other = SecretKey::from_bytes([9u8; KEY_LEN]);

        assert!(other.decrypt(&encoded).is_err());
    }

    #[test]
    fn encryption_is_randomised() {
        let key = test_key();

        assert_ne!(key.encrypt("same").unwrap(), key.encrypt("same").unwrap());
    }
}
