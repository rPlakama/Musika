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

//! Local similar-songs expansion (offline `GetSimilarSongs2`).
//!
//! Radio used to be an alias for the server's `getSimilarSongs`: it needed a
//! connection even though the whole candidate catalog lives in `library.db`.
//! This module expands a seed entirely locally:
//!
//! 1. **Similar artists.** A cached `getArtistInfo2.similarArtist` graph (the
//!    same request that populates the artist-page biography) is preferred.
//!    Without it, artists are ranked from local metadata: normalized genre
//!    tokens with IDF weighting, mean-year proximity, playlist co-occurrence
//!    and catalog support (so one-song artists cannot win by sharing one tag).
//! 2. **Song mix.** The seed artist and its similar artists contribute their
//!    most-played songs with Navidrome's fallback weighting (`topCount *
//!    (4 + artistWeight)`, decaying by 4 per song), followed by genre, era and
//!    random top-ups. The result never contains duplicates and is capped.
//!
//! Playback still needs the local library only; the queue builder turns the
//! returned rows into stream/file URLs exactly like every other queue. With
//! `offline = true` every candidate is restricted to downloaded songs before
//! selection, so the page and the queue show only playable rows.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, PoisonError};

use anyhow::{Result, anyhow};
use sqlx::FromRow;

use crate::db::Dbs;
use crate::library::{SimilarArtist, SongRow};

/// How many rows the radio page/queue asks for (the old server call asked 100).
pub(crate) const RADIO_COUNT: usize = 100;
/// Hard cap for callers that pass a larger count, matching Navidrome's limit.
const MAX_COUNT: usize = 500;
/// Similar artists considered per expansion.
const MAX_SIMILAR_ARTISTS: usize = 15;
/// Song count at which an artist reaches full support weight.
const FULL_SUPPORT_SONGS: f64 = 8.0;
/// Navidrome's seed-artist weight in the weighted chooser.
const SEED_ARTIST_WEIGHT: i64 = 10;
/// How many seed genre strings are used for the genre top-up query.
const GENRE_TOPUP_TAGS: usize = 5;
/// Seeds whose similar graph is being fetched, so opening many radios for the
/// same artist does not fire parallel `getArtistInfo2` requests.
static GRAPH_FETCHES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Expands `seed_kind`/`seed_id` into songs similar to it, without network.
///
/// `offline` restricts the candidates to downloaded songs. `rng_seed` makes the
/// weighted pick reproducible (tests) while production passes the clock.
pub(crate) async fn similar_songs(
    dbs: &Dbs,
    seed_kind: &str,
    seed_id: &str,
    count: usize,
    offline: bool,
    rng_seed: u64,
) -> Result<Vec<SongRow>> {
    let count = count.clamp(1, MAX_COUNT);
    let seed = load_seed(dbs, seed_kind, seed_id).await?;
    let seed_artist = seed.artist_id.clone();

    let artists = ordered_similar_artists(dbs, &seed).await?;

    let downloaded = if offline {
        crate::offline::load(dbs, true).await?
    } else {
        None
    };
    let top_count = count.max(20);

    // Navidrome fallback parity: seed artist and similar artists contribute
    // their top songs to one weighted chooser.
    let mut chooser = WeightedChooser::default();
    let mut artist_order = Vec::with_capacity(artists.len() + 1);
    if let Some(seed_artist) = &seed_artist {
        artist_order.push((seed_artist.clone(), SEED_ARTIST_WEIGHT));
    }
    for artist in &artists {
        artist_order.push((artist.clone(), 0));
    }
    for (artist_id, artist_weight) in artist_order {
        let mut songs = artist_songs(dbs, &artist_id, &seed, top_count).await?;
        if let Some(downloaded) = &downloaded {
            songs.retain(|song| downloaded.contains_song(&song.id));
        }
        let mut weight = top_count as i64 * (4 + artist_weight);
        for song in songs {
            if weight > 0 {
                chooser.add(song, weight);
            }
            weight -= 4;
        }
    }

    let mut picked: Vec<SongFeature> = Vec::with_capacity(count);
    let mut picked_ids: HashSet<String> = HashSet::with_capacity(count);
    let mut rng = SplitMix64(rng_seed);
    while picked.len() < count {
        let Some(song) = chooser.pick(&mut rng) else {
            break;
        };
        if picked_ids.insert(song.id.clone()) {
            picked.push(song);
        }
    }

    // Top up a short mix: genre first (the strongest local relation), then
    // era, then anything playable. These are appended in query order so the
    // page and queue stay in step.
    if picked.len() < count {
        let mut genre_rows = genre_songs(dbs, &seed, count - picked.len()).await?;
        retain_offline(&downloaded, &mut genre_rows);
        push_unique(&mut picked, &mut picked_ids, genre_rows, count);
    }
    if picked.len() < count {
        let mut era_rows = era_songs(dbs, &seed, count - picked.len()).await?;
        retain_offline(&downloaded, &mut era_rows);
        push_unique(&mut picked, &mut picked_ids, era_rows, count);
    }
    if picked.len() < count {
        let mut random_rows = random_songs(dbs, &seed, count - picked.len()).await?;
        retain_offline(&downloaded, &mut random_rows);
        push_unique(&mut picked, &mut picked_ids, random_rows, count);
    }

    Ok(picked.into_iter().map(SongFeature::into_row).collect())
}

/// The cached provider graph is the quality signal; local metadata fills
/// whatever it does not cover. A stale/missing graph refreshes in the
/// background (online only) so the expansion itself never waits on HTTP.
async fn ordered_similar_artists(dbs: &Dbs, seed: &Seed) -> Result<Vec<String>> {
    let Some(seed_artist) = seed.artist_id.as_deref() else {
        return Ok(Vec::new());
    };
    let mut artists = cached_similar_artist_ids(dbs, seed_artist).await?;
    maybe_refresh_similar_graph(dbs, seed_artist).await;
    for artist in local_similar_artists(dbs, seed).await? {
        if artists.len() >= MAX_SIMILAR_ARTISTS {
            break;
        }
        if !artists.contains(&artist) {
            artists.push(artist);
        }
    }
    Ok(artists)
}

#[cfg(test)]
pub(crate) async fn similar_artists_for_test(
    dbs: &Dbs,
    seed_kind: &str,
    seed_id: &str,
) -> Result<Vec<String>> {
    let seed = load_seed(dbs, seed_kind, seed_id).await?;
    ordered_similar_artists(dbs, &seed).await
}

#[cfg(test)]
pub(crate) async fn playlist_affinity_for_test(
    dbs: &Dbs,
    seed_artist: &str,
) -> Result<HashMap<String, f64>> {
    playlist_affinity(dbs, seed_artist).await
}

#[cfg(test)]
pub(crate) fn genre_tokens_for_test(genre: &str) -> Vec<String> {
    let mut tokens: Vec<String> = genre_tokens(genre).into_iter().collect();
    tokens.sort();
    tokens
}

// ---------------------------------------------------------------------------
// Seed resolution
// ---------------------------------------------------------------------------

/// One local song with the metadata the recommendation engine needs.
#[derive(Debug, Clone, FromRow)]
struct SongFeature {
    id: String,
    album_id: Option<String>,
    artist_id: Option<String>,
    title: String,
    album: Option<String>,
    artist: Option<String>,
    track: Option<i32>,
    duration: Option<i32>,
    suffix: Option<String>,
    cover_art: Option<String>,
    starred: Option<String>,
    genre: Option<String>,
    year: Option<i32>,
    play_count: Option<i64>,
}

impl SongFeature {
    fn into_row(self) -> SongRow {
        SongRow {
            id: self.id,
            title: self.title,
            artist: self.artist,
            album: self.album,
            album_id: self.album_id,
            track: self.track,
            duration: self.duration,
            suffix: self.suffix,
            cover_art: self.cover_art,
            starred: self.starred,
        }
    }
}

/// The part of the seed the expansion needs; the page keeps its own identity
/// header from [`crate::radio`].
struct Seed {
    artist_id: Option<String>,
    /// Raw genre strings (song + album), in preference order.
    genre_texts: Vec<String>,
    year: Option<i32>,
    exclude_song: Option<String>,
    exclude_album: Option<String>,
}

#[derive(Debug, FromRow)]
struct AlbumSeedRow {
    id: String,
    artist_id: Option<String>,
    name: String,
    artist: Option<String>,
    year: Option<i32>,
    genre: Option<String>,
}

async fn load_seed(dbs: &Dbs, kind: &str, id: &str) -> Result<Seed> {
    match kind {
        "song" => {
            let song = sqlx::query_as::<_, SongFeature>(SONG_COLUMNS)
                .bind(id)
                .fetch_optional(&dbs.library)
                .await?
                .ok_or_else(|| anyhow!("this song is not in the local library"))?;
            let album = match song.album_id.as_deref() {
                Some(album_id) => album_row(dbs, album_id).await?,
                None => None,
            };
            let mut genre_texts = Vec::new();
            if let Some(genre) = song.genre.clone() {
                genre_texts.push(genre);
            }
            if let Some(album) = &album
                && let Some(genre) = album.genre.clone()
                && !genre_texts.contains(&genre)
            {
                genre_texts.push(genre);
            }
            Ok(Seed {
                artist_id: song.artist_id.clone().or_else(|| {
                    album.as_ref().and_then(|album| album.artist_id.clone())
                }),
                genre_texts,
                year: song
                    .year
                    .or_else(|| album.as_ref().and_then(|album| album.year)),
                exclude_song: Some(song.id),
                exclude_album: song.album_id,
            })
        }
        "album" => {
            let album = album_row(dbs, id)
                .await?
                .ok_or_else(|| anyhow!("this album is not in the local library"))?;
            let mut genre_texts = Vec::new();
            if let Some(genre) = album.genre.clone() {
                genre_texts.push(genre);
            }
            for genre in album_genres(dbs, &album.id).await? {
                if !genre_texts.contains(&genre) {
                    genre_texts.push(genre);
                }
            }
            Ok(Seed {
                artist_id: album.artist_id,
                genre_texts,
                year: album.year,
                exclude_song: None,
                exclude_album: Some(album.id),
            })
        }
        "artist" => {
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM songs WHERE artist_id = ?1",
            )
            .bind(id)
            .fetch_one(&dbs.library)
            .await?;
            if exists == 0 {
                return Err(anyhow!("this artist is not in the local library"));
            }
            let mut genre_texts = Vec::new();
            for (genre, count) in artist_genres(dbs, id).await? {
                if count > 0 {
                    genre_texts.push(genre);
                }
            }
            let year = artist_mean_year(dbs, id).await?;
            Ok(Seed {
                artist_id: Some(id.to_string()),
                genre_texts,
                year,
                exclude_song: None,
                exclude_album: None,
            })
        }
        other => Err(anyhow!("unknown radio seed kind: {other}")),
    }
}

const SONG_SELECT: &str = "SELECT id, album_id, artist_id, title, album, artist, track, duration,
        suffix, cover_art, starred, genre, year, play_count FROM songs";
const SONG_COLUMNS: &str = "SELECT id, album_id, artist_id, title, album, artist, track, duration,
        suffix, cover_art, starred, genre, year, play_count FROM songs WHERE id = ?1";

async fn album_row(dbs: &Dbs, id: &str) -> Result<Option<AlbumSeedRow>> {
    let row = sqlx::query_as::<_, AlbumSeedRow>(
        "SELECT id, artist_id, name, artist, year, genre FROM albums WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&dbs.library)
    .await?;
    Ok(row)
}

async fn album_genres(dbs: &Dbs, album_id: &str) -> Result<Vec<String>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT genre FROM songs
         WHERE album_id = ?1 AND genre IS NOT NULL AND genre <> ''",
    )
    .bind(album_id)
    .fetch_all(&dbs.library)
    .await?;
    Ok(rows)
}

async fn artist_genres(dbs: &Dbs, artist_id: &str) -> Result<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT genre, COUNT(*) FROM songs
         WHERE artist_id = ?1 AND genre IS NOT NULL AND genre <> ''
         GROUP BY genre ORDER BY COUNT(*) DESC",
    )
    .bind(artist_id)
    .fetch_all(&dbs.library)
    .await?;
    Ok(rows)
}

async fn artist_mean_year(dbs: &Dbs, artist_id: &str) -> Result<Option<i32>> {
    let mean: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(year) FROM songs WHERE artist_id = ?1 AND year > 0",
    )
    .bind(artist_id)
    .fetch_one(&dbs.library)
    .await?;
    Ok(mean.map(|value| value.round() as i32))
}

// ---------------------------------------------------------------------------
// Similar artists
// ---------------------------------------------------------------------------

/// The cached provider graph for `artist_id`, filtered to artists that still
/// exist locally and capped at [`MAX_SIMILAR_ARTISTS`].
async fn cached_similar_artist_ids(dbs: &Dbs, artist_id: &str) -> Result<Vec<String>> {
    let Some(row) = crate::library::artist_info(&dbs.library, artist_id).await? else {
        return Ok(Vec::new());
    };
    let graph: Vec<SimilarArtist> = crate::library::similar_artists(&row);
    let ids: Vec<String> = graph
        .into_iter()
        .map(|artist| artist.id)
        .filter(|id| !id.is_empty() && id != artist_id)
        .take(MAX_SIMILAR_ARTISTS)
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = sqlx::QueryBuilder::new("SELECT id FROM artists WHERE id IN (");
    crate::cache::push_in_list(&mut builder, &ids);
    let existing: Vec<String> = builder.build_query_scalar().fetch_all(&dbs.library).await?;
    let existing: HashSet<String> = existing.into_iter().collect();
    Ok(ids.into_iter().filter(|id| existing.contains(id)).collect())
}

/// Spawns a best-effort provider graph refresh when online and the cached row
/// is missing or older than the metadata TTL. The expansion never awaits it.
async fn maybe_refresh_similar_graph(dbs: &Dbs, artist_id: &str) {
    if !crate::session::online() {
        return;
    }
    let fresh = match crate::library::artist_info(&dbs.library, artist_id).await {
        Ok(Some(row)) => row
            .similar_fetched_at
            .parse::<i64>()
            .is_ok_and(|at| crate::consts::now_secs() - at < crate::consts::INFO_TTL_SECS),
        Ok(None) => false,
        Err(_) => false,
    };
    if fresh {
        return;
    }
    let artist_id = artist_id.to_string();
    if !GRAPH_FETCHES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(artist_id.clone())
    {
        return;
    }
    let dbs = dbs.clone();
    crate::app::runtime().spawn(async move {
        let result = refresh_similar_graph(&dbs, &artist_id).await;
        if let Err(err) = result {
            crate::debug_log!("GPUI: radio similar graph refresh failed: {err:#}");
        }
        GRAPH_FETCHES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&artist_id);
    });
}

/// Fetches and persists the provider's similar-artist list for one artist.
pub(crate) async fn refresh_similar_graph(dbs: &Dbs, artist_id: &str) -> Result<()> {
    let client = crate::session::client(dbs).await?;
    let similar = match client
        .get_artist_info2(artist_id, Some(MAX_SIMILAR_ARTISTS as u32), Some(false))
        .await
    {
        Ok(info) => info
            .similar_artist
            .into_iter()
            .map(|artist| SimilarArtist {
                id: artist.id,
                name: artist.name,
            })
            .collect::<Vec<_>>(),
        Err(err) => return Err(err),
    };
    crate::library::upsert_artist_similar(&dbs.library, artist_id, &similar).await?;
    Ok(())
}

/// Ranks local artists by metadata similarity to the seed artist:
/// `0.60 * genre(tokens, IDF) + 0.20 * era + 0.20 * playlist co-occurrence`,
/// multiplied by a catalog-support factor so one-song artists cannot win on a
/// single shared tag.
async fn local_similar_artists(dbs: &Dbs, seed: &Seed) -> Result<Vec<String>> {
    let Some(seed_artist) = seed.artist_id.as_deref() else {
        return Ok(Vec::new());
    };
    let profiles = artist_profiles(dbs).await?;
    let Some(seed_profile) = profiles.get(seed_artist) else {
        return Ok(Vec::new());
    };
    let playlist = playlist_affinity(dbs, seed_artist).await?;
    let max_playlist = playlist.values().copied().fold(0.0_f64, f64::max);
    let seed_norm = seed_profile.genre_norm();

    let mut ranked: Vec<(f64, String)> = Vec::new();
    for (artist_id, profile) in &profiles {
        if artist_id == seed_artist {
            continue;
        }
        let genre = if seed_norm > 0.0 {
            profile.genre_dot(seed_profile) / (seed_norm * profile.genre_norm()).max(f64::MIN_POSITIVE)
        } else {
            0.0
        };
        let era = match (seed.year, profile.mean_year) {
            (Some(left), Some(right)) => {
                (1.0 - (f64::from(left - right).abs()) / 15.0).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        let playlist_score = match (playlist.get(artist_id), max_playlist) {
            (Some(count), max) if max > 0.0 => count / max,
            _ => 0.0,
        };
        let support = (profile.song_count / FULL_SUPPORT_SONGS).min(1.0);
        let mut weights = 0.0;
        let mut score = 0.0;
        if seed_norm > 0.0 {
            weights += 0.60;
            score += 0.60 * genre;
        }
        if seed.year.is_some() && profile.mean_year.is_some() {
            weights += 0.20;
            score += 0.20 * era;
        }
        if max_playlist > 0.0 {
            weights += 0.20;
            score += 0.20 * playlist_score;
        }
        if weights == 0.0 {
            continue;
        }
        let score = (score / weights) * support;
        if score > 0.03 {
            ranked.push((score, artist_id.clone()));
        }
    }
    ranked.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    Ok(ranked
        .into_iter()
        .take(MAX_SIMILAR_ARTISTS)
        .map(|(_, id)| id)
        .collect())
}

/// Per-artist genre token weights (IDF), mean year and catalog size.
struct ArtistProfile {
    genre_weights: HashMap<String, f64>,
    mean_year: Option<i32>,
    song_count: f64,
}

impl ArtistProfile {
    fn genre_norm(&self) -> f64 {
        self.genre_weights
            .values()
            .map(|weight| weight * weight)
            .sum::<f64>()
            .sqrt()
    }

    fn genre_dot(&self, other: &Self) -> f64 {
        let (small, large) = if self.genre_weights.len() <= other.genre_weights.len() {
            (&self.genre_weights, &other.genre_weights)
        } else {
            (&other.genre_weights, &self.genre_weights)
        };
        small
            .iter()
            .filter_map(|(token, weight)| large.get(token).map(|other| weight * other))
            .sum()
    }
}

async fn artist_profiles(dbs: &Dbs) -> Result<HashMap<String, ArtistProfile>> {
    let genre_counts: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT artist_id, genre, COUNT(*) FROM songs
         WHERE artist_id IS NOT NULL AND artist_id <> ''
           AND genre IS NOT NULL AND genre <> ''
         GROUP BY artist_id, genre",
    )
    .fetch_all(&dbs.library)
    .await?;
    let years: Vec<(String, f64, i64)> = sqlx::query_as(
        "SELECT artist_id, AVG(year), COUNT(*) FROM songs
         WHERE artist_id IS NOT NULL AND artist_id <> '' AND year > 0
         GROUP BY artist_id",
    )
    .fetch_all(&dbs.library)
    .await?;

    let mut document_frequency: HashMap<String, f64> = HashMap::new();
    for (_, genre, _) in &genre_counts {
        for token in genre_tokens(genre) {
            *document_frequency.entry(token).or_default() += 1.0;
        }
    }
    let artist_count = years.len().max(1) as f64;
    let mut profiles: HashMap<String, ArtistProfile> = HashMap::new();
    for (artist_id, genre, count) in genre_counts {
        let profile = profiles.entry(artist_id).or_insert_with(|| ArtistProfile {
            genre_weights: HashMap::new(),
            mean_year: None,
            song_count: 0.0,
        });
        let tokens = genre_tokens(&genre);
        if !tokens.is_empty() && count > 0 {
            let split = 1.0 / tokens.len() as f64;
            for token in tokens {
                let df = document_frequency.get(&token).copied().unwrap_or(1.0);
                let idf = ((artist_count + 1.0) / (df + 0.5)).ln();
                *profile.genre_weights.entry(token).or_default() +=
                    count as f64 * split * idf;
            }
        }
    }
    for (artist_id, mean, count) in years {
        let profile = profiles.entry(artist_id).or_insert_with(|| ArtistProfile {
            genre_weights: HashMap::new(),
            mean_year: None,
            song_count: 0.0,
        });
        profile.mean_year = Some(mean.round() as i32);
        profile.song_count = count as f64;
    }
    Ok(profiles)
}

/// Normalizes a provider genre string into lowercase alphanumeric tokens.
///
/// Provider tags are free text (`Rock`, `Rock progressivo`, `Alternative
/// Rock`, `Alternativo & Indie`), so exact-string matching would miss obvious
/// relatives. Tokenizing is enough for the fallback; the cached provider graph
/// remains the authoritative ordering.
fn genre_tokens(genre: &str) -> HashSet<String> {
    genre
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.chars().count() > 1)
        .map(ToOwned::to_owned)
        .collect()
}

/// Artist co-occurrence in the user's own playlists. Songs of the seed artist
/// and candidate artists that share a playlist count once per playlist; the
/// library's song ids are the join key (`playlist_song.song_id`).
async fn playlist_affinity(dbs: &Dbs, seed_artist: &str) -> Result<HashMap<String, f64>> {
    let seed_songs: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM songs WHERE artist_id = ?1 LIMIT 200",
    )
    .bind(seed_artist)
    .fetch_all(&dbs.library)
    .await?;
    if seed_songs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT DISTINCT playlist_id FROM playlist_song WHERE song_id IN (",
    );
    crate::cache::push_in_list(&mut builder, &seed_songs);
    builder.push(" LIMIT 50");
    let playlists: Vec<String> = match builder.build_query_scalar().fetch_all(&dbs.playlist).await {
        Ok(playlists) => playlists,
        // The playlist cache is disposable: a missing/old schema just removes
        // this optional signal.
        Err(_) => return Ok(HashMap::new()),
    };
    if playlists.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT song_id, COUNT(*) FROM playlist_song WHERE playlist_id IN (",
    );
    crate::cache::push_in_list(&mut builder, &playlists);
    builder.push(" AND song_id IS NOT NULL GROUP BY song_id LIMIT 1000");
    let song_counts: Vec<(String, i64)> = builder
        .build_query_as()
        .fetch_all(&dbs.playlist)
        .await
        .unwrap_or_default();
    if song_counts.is_empty() {
        return Ok(HashMap::new());
    }
    let song_ids: Vec<String> = song_counts.iter().map(|(id, _)| id.clone()).collect();
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT id, artist_id FROM songs WHERE id IN (",
    );
    crate::cache::push_in_list(&mut builder, &song_ids);
    let artists: Vec<(String, Option<String>)> = builder
        .build_query_as()
        .fetch_all(&dbs.library)
        .await
        .unwrap_or_default();
    let song_artists: HashMap<String, String> = artists
        .into_iter()
        .filter_map(|(id, artist)| artist.map(|artist| (id, artist)))
        .collect();
    let mut affinity: HashMap<String, f64> = HashMap::new();
    for (song_id, count) in song_counts {
        if let Some(artist) = song_artists.get(&song_id)
            && artist != seed_artist
        {
            *affinity.entry(artist.clone()).or_default() += count as f64;
        }
    }
    Ok(affinity)
}

// ---------------------------------------------------------------------------
// Candidate songs
// ---------------------------------------------------------------------------

/// Songs of one artist for the weighted chooser, most popular/starred first.
async fn artist_songs(
    dbs: &Dbs,
    artist_id: &str,
    seed: &Seed,
    limit: usize,
) -> Result<Vec<SongFeature>> {
    let mut builder = sqlx::QueryBuilder::new(SONG_SELECT);
    builder.push(" WHERE artist_id = ");
    builder.push_bind(artist_id);
    if let Some(exclude) = &seed.exclude_song {
        builder.push(" AND id <> ");
        builder.push_bind(exclude);
    }
    if let Some(exclude) = &seed.exclude_album {
        builder.push(" AND (album_id IS NULL OR album_id <> ");
        builder.push_bind(exclude);
        builder.push(")");
    }
    builder.push(
        " ORDER BY CASE WHEN starred IS NOT NULL THEN 0 ELSE 1 END,
                   COALESCE(play_count, 0) DESC, RANDOM() LIMIT ",
    );
    builder.push_bind(limit as i64);
    let rows = builder.build_query_as::<SongFeature>().fetch_all(&dbs.library).await?;
    Ok(rows)
}

/// Songs carrying one of the seed's genre strings, for a short mix.
async fn genre_songs(dbs: &Dbs, seed: &Seed, limit: usize) -> Result<Vec<SongFeature>> {
    let mut genres: Vec<String> = seed.genre_texts.clone();
    genres.dedup();
    genres.truncate(GENRE_TOPUP_TAGS);
    if genres.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = sqlx::QueryBuilder::new(SONG_SELECT);
    builder.push(" WHERE genre IN (");
    crate::cache::push_in_list(&mut builder, &genres);
    push_seed_excludes(&mut builder, seed);
    builder.push(" ORDER BY RANDOM() LIMIT ");
    builder.push_bind((limit * 3).max(limit) as i64);
    let rows = builder.build_query_as::<SongFeature>().fetch_all(&dbs.library).await?;
    Ok(rows)
}

/// Songs from the seed's decade (widened to the surrounding four years), for
/// the last metadata top-up.
async fn era_songs(dbs: &Dbs, seed: &Seed, limit: usize) -> Result<Vec<SongFeature>> {
    let Some(year) = seed.year else {
        return Ok(Vec::new());
    };
    let mut builder = sqlx::QueryBuilder::new(SONG_SELECT);
    builder.push(" WHERE year BETWEEN ");
    builder.push_bind(year - 4);
    builder.push(" AND ");
    builder.push_bind(year + 4);
    push_seed_excludes(&mut builder, seed);
    builder.push(" ORDER BY RANDOM() LIMIT ");
    builder.push_bind((limit * 3).max(limit) as i64);
    let rows = builder.build_query_as::<SongFeature>().fetch_all(&dbs.library).await?;
    Ok(rows)
}

/// Any other local song, so a radio on a tiny/metadata-less library is not
/// empty.
async fn random_songs(dbs: &Dbs, seed: &Seed, limit: usize) -> Result<Vec<SongFeature>> {
    let mut builder = sqlx::QueryBuilder::new(SONG_SELECT);
    builder.push(" WHERE 1 = 1");
    push_seed_excludes(&mut builder, seed);
    builder.push(" ORDER BY RANDOM() LIMIT ");
    builder.push_bind((limit * 3).max(limit) as i64);
    let rows = builder.build_query_as::<SongFeature>().fetch_all(&dbs.library).await?;
    Ok(rows)
}

fn push_seed_excludes(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, seed: &Seed) {
    if let Some(exclude) = &seed.exclude_song {
        builder.push(" AND id <> ");
        builder.push_bind(exclude);
    }
    if let Some(exclude) = &seed.exclude_album {
        builder.push(" AND (album_id IS NULL OR album_id <> ");
        builder.push_bind(exclude);
        builder.push(")");
    }
}

fn retain_offline(
    downloaded: &Option<std::sync::Arc<crate::offline::Filter>>,
    rows: &mut Vec<SongFeature>,
) {
    if let Some(downloaded) = downloaded {
        rows.retain(|song| downloaded.contains_song(&song.id));
    }
}

fn push_unique(
    picked: &mut Vec<SongFeature>,
    picked_ids: &mut HashSet<String>,
    rows: Vec<SongFeature>,
    count: usize,
) {
    for row in rows {
        if picked.len() >= count {
            break;
        }
        if picked_ids.insert(row.id.clone()) {
            picked.push(row);
        }
    }
}

// ---------------------------------------------------------------------------
// Weighted chooser
// ---------------------------------------------------------------------------

/// Weighted random selection without replacement, matching Navidrome's
/// fallback chooser: each entry is drawn proportional to its weight and
/// removed once picked.
#[derive(Default)]
struct WeightedChooser {
    entries: Vec<(i64, SongFeature)>,
    total: i64,
}

impl WeightedChooser {
    fn add(&mut self, song: SongFeature, weight: i64) {
        if weight <= 0 {
            return;
        }
        self.total += weight;
        self.entries.push((weight, song));
    }

    fn pick(&mut self, rng: &mut SplitMix64) -> Option<SongFeature> {
        if self.total <= 0 || self.entries.is_empty() {
            return None;
        }
        let mut needle = rng.below(self.total as u64) as i64;
        let mut index = 0;
        for (position, (weight, _)) in self.entries.iter().enumerate() {
            if needle < *weight {
                index = position;
                break;
            }
            needle -= weight;
        }
        self.total -= self.entries[index].0;
        Some(self.entries.swap_remove(index).1)
    }
}

/// Seeded SplitMix64; the same generator the queue shuffle uses, kept local
/// so recommendation tests can pin the pick.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, len: u64) -> u64 {
        if len == 0 {
            return 0;
        }
        ((u128::from(self.next()) * u128::from(len)) >> 64) as u64
    }
}
