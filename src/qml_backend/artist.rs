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

use super::support::client;

pub(super) enum ArtistUserAction {
    Star(bool),
    Rate(u8),
}

pub(super) async fn apply_artist_user(
    dbs: &crate::db::Dbs,
    artist_id: &str,
    action: ArtistUserAction,
) -> anyhow::Result<()> {
    let client = client(dbs).await?;
    match action {
        ArtistUserAction::Star(on) => {
            if on {
                client.star(&[], &[], &[artist_id]).await?;
            } else {
                client.unstar(&[], &[], &[artist_id]).await?;
            }
            crate::artist_user::set_starred(&dbs.artist_user, artist_id, on).await?;
        }
        ArtistUserAction::Rate(rating) => {
            client.set_rating(artist_id, rating).await?;
            crate::artist_user::set_rating(&dbs.artist_user, artist_id, rating).await?;
        }
    }
    Ok(())
}
