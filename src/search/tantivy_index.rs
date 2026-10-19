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

//! ARC-SEARCH: the persistent Tantivy catalog index.
//!
//! Tantivy owns the search corpus (BM25 over title/artist/album, prefix +
//! one-edit fuzzy terms, fast fields for play counts and album track order)
//! and stores the fields the palette payload needs. The index lives under
//! `search-index/` in the config directory, survives restarts, and is
//! rebuilt into a fresh directory after a library sync before being swapped
//! in. As with the other engines, a failure is not fatal: the caller falls
//! back to FTS; the old index keeps serving until the swap.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, STORED, STRING, Schema, TEXT, TantivyDocument, Value,
};
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, Order, ReloadPolicy, Term};

use crate::library::{AlbumRow, ArtistRow};

const SCHEMA_VERSION: u64 = 1;
const INDEX_META: &str = "musika-meta.json";
const WRITER_HEAP: usize = 50_000_000;
/// How many of the best BM25 hits are re-scored for the S-9 play boost.
const SONG_SCAN: usize = 512;
const PLAY_BOOST: f32 = 0.01;
const PLAY_CAP: i64 = 500;

/// The catalog row stored in the index (and returned to the payload).
#[derive(Debug, Clone, FromRow)]
pub struct SongHit {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<i32>,
    pub play_count: i64,
    pub album_id: Option<String>,
    pub cover_art: Option<String>,
    pub track: Option<i32>,
    pub disc_number: Option<i32>,
}

pub struct SearchResults {
    pub songs: Vec<SongHit>,
    pub artists: Vec<ArtistRow>,
    pub albums: Vec<AlbumRow>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IndexMeta {
    version: u64,
    catalog: String,
}

#[derive(Clone)]
struct Fields {
    kind: Field,
    id: Field,
    title: Field,
    artist: Field,
    album: Field,
    album_id: Field,
    duration: Field,
    play_count: Field,
    most_rank: Field,
    cover_art: Field,
    starred: Field,
    track: Field,
    disc: Field,
    year: Field,
    count: Field,
    rating: Field,
}

impl Fields {
    fn from_schema(schema: &Schema) -> Result<Self> {
        let field = |name: &str| {
            schema
                .get_field(name)
                .with_context(|| format!("search index is missing the `{name}` field"))
        };
        Ok(Self {
            kind: field("kind")?,
            id: field("id")?,
            title: field("title")?,
            artist: field("artist")?,
            album: field("album")?,
            album_id: field("album_id")?,
            duration: field("duration")?,
            play_count: field("play_count")?,
            most_rank: field("most_rank")?,
            cover_art: field("cover_art")?,
            starred: field("starred")?,
            track: field("track")?,
            disc: field("disc")?,
            year: field("year")?,
            count: field("count")?,
            rating: field("rating")?,
        })
    }
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let fields = Fields {
        kind: builder.add_text_field("kind", STRING | FAST),
        id: builder.add_text_field("id", STRING | STORED),
        title: builder.add_text_field("title", TEXT | STORED),
        artist: builder.add_text_field("artist", TEXT | STORED),
        album: builder.add_text_field("album", TEXT | STORED),
        album_id: builder.add_text_field("album_id", STRING | STORED | FAST),
        duration: builder.add_u64_field("duration", STORED | FAST),
        play_count: builder.add_u64_field("play_count", STORED | FAST),
        most_rank: builder.add_u64_field("most_rank", FAST),
        cover_art: builder.add_text_field("cover_art", STORED),
        starred: builder.add_text_field("starred", STORED),
        track: builder.add_u64_field("track", STORED | FAST),
        disc: builder.add_u64_field("disc", STORED | FAST),
        year: builder.add_u64_field("year", STORED | FAST),
        count: builder.add_u64_field("count", STORED | FAST),
        rating: builder.add_u64_field("rating", STORED | FAST),
    };
    (builder.build(), fields)
}

/// The index keeps a normalized term dictionary: lowercased and ASCII-folded,
/// so `bjork` matches `Björk` on both the ingest and query sides.
fn register_tokenizers(index: &Index) {
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build();
    index.tokenizers().register("default", analyzer);
}

fn text(doc: &TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn opt_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    let value = text(doc, field);
    (!value.is_empty()).then_some(value)
}

fn number(doc: &TantivyDocument, field: Field) -> u64 {
    doc.get_first(field)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn opt_number(doc: &TantivyDocument, field: Field) -> Option<u64> {
    match number(doc, field) {
        0 => None,
        value => Some(value),
    }
}

fn song_doc(fields: &Fields, song: &SongHit, most_rank: u64) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.kind, "song");
    doc.add_text(fields.id, &song.id);
    doc.add_text(fields.title, &song.title);
    doc.add_text(fields.artist, song.artist.as_deref().unwrap_or(""));
    doc.add_text(fields.album, song.album.as_deref().unwrap_or(""));
    doc.add_text(fields.album_id, song.album_id.as_deref().unwrap_or(""));
    doc.add_text(fields.cover_art, song.cover_art.as_deref().unwrap_or(""));
    doc.add_u64(fields.duration, song.duration.unwrap_or(0).max(0) as u64);
    doc.add_u64(fields.play_count, song.play_count.max(0) as u64);
    doc.add_u64(fields.most_rank, most_rank);
    doc.add_u64(fields.track, song.track.unwrap_or(0).max(0) as u64);
    doc.add_u64(fields.disc, song.disc_number.unwrap_or(0).max(0) as u64);
    doc
}

/// Index the songs with their Most-played rank precomputed (play count
/// descending, title ascending), so the empty-query surface is a fast-field
/// range read instead of a scan over ties.
fn add_songs(writer: &IndexWriter, fields: &Fields, songs: &[SongHit]) -> Result<()> {
    let mut order: Vec<usize> = (0..songs.len()).collect();
    order.sort_by(|&a, &b| {
        songs[b]
            .play_count
            .cmp(&songs[a].play_count)
            .then_with(|| songs[a].title.cmp(&songs[b].title))
    });
    let mut ranks = vec![0_u64; songs.len()];
    for (rank, &index) in order.iter().enumerate() {
        ranks[index] = rank as u64;
    }
    for (song, rank) in songs.iter().zip(ranks) {
        writer.add_document(song_doc(fields, song, rank))?;
    }
    Ok(())
}

fn artist_doc(fields: &Fields, artist: &ArtistRow) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.kind, "artist");
    doc.add_text(fields.id, &artist.id);
    doc.add_text(fields.title, &artist.name);
    doc.add_text(fields.cover_art, artist.cover_art.as_deref().unwrap_or(""));
    doc.add_text(fields.starred, artist.starred.as_deref().unwrap_or(""));
    doc.add_u64(fields.count, artist.album_count.max(0) as u64);
    doc
}

fn album_doc(fields: &Fields, album: &AlbumRow) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.kind, "album");
    doc.add_text(fields.id, &album.id);
    doc.add_text(fields.title, &album.title);
    doc.add_text(fields.artist, album.artist.as_deref().unwrap_or(""));
    doc.add_text(fields.cover_art, album.cover_art.as_deref().unwrap_or(""));
    doc.add_text(fields.starred, album.starred.as_deref().unwrap_or(""));
    doc.add_u64(fields.year, album.year.unwrap_or(0).max(0) as u64);
    doc.add_u64(fields.count, album.song_count.max(0) as u64);
    doc.add_u64(fields.rating, album.rating.clamp(0, 5) as u64);
    doc
}

/// The in-memory handle to the on-disk index. Queries clone the `Arc`; the
/// reader is immutable, so a rebuild can swap the whole handle atomically
/// while in-flight searches keep the old snapshot.
pub struct TantivyIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl TantivyIndex {
    fn from_index(index: Index) -> Result<Self> {
        register_tokenizers(&index);
        let fields = Fields::from_schema(&index.schema())?;
        let reader: IndexReader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    fn open(path: &Path, catalog: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path.join(INDEX_META))
            .with_context(|| format!("no search index metadata at {}", path.display()))?;
        let meta: IndexMeta = serde_json::from_str(&raw)?;
        if meta.version != SCHEMA_VERSION || meta.catalog != catalog {
            anyhow::bail!("search index is stale");
        }
        let index = Index::open_in_dir(path)?;
        Self::from_index(index)
    }

    /// Prefix/fuzzy search over one kind. `raw` is already punctuation-clean.
    pub fn search(&self, raw: &str, limit: usize) -> Result<SearchResults> {
        let terms: Vec<&str> = raw
            .split_whitespace()
            .filter(|term| !term.is_empty())
            .collect();
        if terms.is_empty() || limit == 0 {
            return Ok(SearchResults {
                songs: Vec::new(),
                artists: Vec::new(),
                albums: Vec::new(),
            });
        }

        let scan = limit.saturating_mul(16).clamp(64, SONG_SCAN);
        let mut songs: Vec<(f32, f32, SongHit)> = self
            .run("song", &terms, scan)?
            .into_iter()
            .filter_map(|(score, doc)| {
                let song = self.song_from_doc(&doc)?;
                let effective = score + song.play_count.clamp(0, PLAY_CAP) as f32 * PLAY_BOOST;
                Some((effective, score, song))
            })
            .collect();
        songs.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| b.1.total_cmp(&a.1))
                .then_with(|| a.2.title.cmp(&b.2.title))
        });

        let artists = self
            .run("artist", &terms, limit)?
            .into_iter()
            .filter_map(|(_, doc)| self.artist_from_doc(&doc))
            .collect();
        let albums = self
            .run("album", &terms, limit)?
            .into_iter()
            .filter_map(|(_, doc)| self.album_from_doc(&doc))
            .collect();

        Ok(SearchResults {
            songs: songs
                .into_iter()
                .take(limit)
                .map(|(_, _, song)| song)
                .collect(),
            artists,
            albums,
        })
    }

    fn query(&self, kind: &str, terms: &[&str]) -> Result<Box<dyn Query>> {
        let kind_query = TermQuery::new(
            Term::from_field_text(self.fields.kind, kind),
            IndexRecordOption::Basic,
        );
        let mut must: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, Box::new(kind_query))];

        let mut analyzer = self
            .index
            .tokenizers()
            .get("default")
            .context("the default tokenizer is not registered")?;
        for raw in terms {
            let Some(token) = analyze(&mut analyzer, raw).into_iter().next() else {
                continue;
            };
            let mut should: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for (field, boost) in [
                (self.fields.title, 3.0_f32),
                (self.fields.artist, 2.0),
                (self.fields.album, 1.0),
            ] {
                let term = Term::from_field_text(field, &token);
                let query: Box<dyn Query> = if token.chars().count() >= 4 {
                    Box::new(FuzzyTermQuery::new_prefix(term, 1, true))
                } else {
                    Box::new(FuzzyTermQuery::new_prefix(term, 0, false))
                };
                should.push((Occur::Should, Box::new(BoostQuery::new(query, boost))));
            }
            must.push((Occur::Must, Box::new(BooleanQuery::new(should))));
        }
        Ok(Box::new(BooleanQuery::new(must)))
    }

    fn run(&self, kind: &str, terms: &[&str], limit: usize) -> Result<Vec<(f32, TantivyDocument)>> {
        let query = self.query(kind, terms)?;
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut docs = Vec::with_capacity(top.len());
        for (score, address) in top {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(address) {
                docs.push((score, doc));
            }
        }
        Ok(docs)
    }

    /// The empty-query surface: the precomputed Most-played rank (play count
    /// descending, title ascending) read straight from a fast field.
    pub fn most_played(&self, limit: usize) -> Result<Vec<SongHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let query = TermQuery::new(
            Term::from_field_text(self.fields.kind, "song"),
            IndexRecordOption::Basic,
        );
        let searcher = self.reader.searcher();
        let top = searcher.search(
            &query,
            &TopDocs::with_limit(limit).order_by_fast_field::<u64>("most_rank", Order::Asc),
        )?;
        Ok(top
            .into_iter()
            .filter_map(|(_, address)| {
                let doc = searcher.doc::<TantivyDocument>(address).ok()?;
                self.song_from_doc(&doc)
            })
            .take(limit)
            .collect())
    }

    /// S-8: an album's tracks in disc/track order.
    pub fn album_tracks(&self, album_id: &str, limit: usize) -> Result<Vec<SongHit>> {
        if album_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let query = BooleanQuery::new(vec![
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.kind, "song"),
                    IndexRecordOption::Basic,
                )),
            ),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.album_id, album_id),
                    IndexRecordOption::Basic,
                )),
            ),
        ]);
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(512).order_by_score())?;
        let mut tracks: Vec<SongHit> = top
            .into_iter()
            .filter_map(|(_, address)| {
                let doc = searcher.doc::<TantivyDocument>(address).ok()?;
                self.song_from_doc(&doc)
            })
            .collect();
        tracks.sort_by(|a, b| {
            a.disc_number
                .unwrap_or(0)
                .cmp(&b.disc_number.unwrap_or(0))
                .then_with(|| a.track.unwrap_or(0).cmp(&b.track.unwrap_or(0)))
                .then_with(|| a.title.cmp(&b.title))
        });
        tracks.truncate(limit);
        Ok(tracks)
    }

    fn song_from_doc(&self, doc: &TantivyDocument) -> Option<SongHit> {
        let id = text(doc, self.fields.id);
        if id.is_empty() {
            return None;
        }
        Some(SongHit {
            id,
            title: text(doc, self.fields.title),
            artist: opt_text(doc, self.fields.artist),
            album: opt_text(doc, self.fields.album),
            duration: opt_number(doc, self.fields.duration)
                .map(|value| value.min(i32::MAX as u64) as i32),
            play_count: number(doc, self.fields.play_count) as i64,
            album_id: opt_text(doc, self.fields.album_id),
            cover_art: opt_text(doc, self.fields.cover_art),
            track: opt_number(doc, self.fields.track)
                .map(|value| value.min(i32::MAX as u64) as i32),
            disc_number: opt_number(doc, self.fields.disc)
                .map(|value| value.min(i32::MAX as u64) as i32),
        })
    }

    fn artist_from_doc(&self, doc: &TantivyDocument) -> Option<ArtistRow> {
        let id = text(doc, self.fields.id);
        if id.is_empty() {
            return None;
        }
        Some(ArtistRow {
            id,
            name: text(doc, self.fields.title),
            album_count: number(doc, self.fields.count) as i64,
            cover_art: opt_text(doc, self.fields.cover_art),
            starred: opt_text(doc, self.fields.starred),
        })
    }

    fn album_from_doc(&self, doc: &TantivyDocument) -> Option<AlbumRow> {
        let id = text(doc, self.fields.id);
        if id.is_empty() {
            return None;
        }
        Some(AlbumRow {
            id,
            title: text(doc, self.fields.title),
            artist: opt_text(doc, self.fields.artist),
            year: opt_number(doc, self.fields.year).map(|value| value.min(i32::MAX as u64) as i32),
            song_count: number(doc, self.fields.count) as i64,
            cover_art: opt_text(doc, self.fields.cover_art),
            starred: opt_text(doc, self.fields.starred),
            rating: number(doc, self.fields.rating) as i64,
        })
    }
}

fn analyze(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut stream = analyzer.token_stream(text);
    while stream.advance() {
        let token = stream.token();
        if !token.text.is_empty() {
            tokens.push(token.text.clone());
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Process-wide index and persistence
// ---------------------------------------------------------------------------

fn slot() -> &'static RwLock<Option<Arc<TantivyIndex>>> {
    static SLOT: std::sync::OnceLock<RwLock<Option<Arc<TantivyIndex>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Serializes index writers: the startup warm and a first search can race
/// before either has installed the handle, and the temp-directory swap must
/// not overlap.
static BUILD: Mutex<()> = Mutex::new(());
/// Bumped by a wipe: a build that started before it must not install its
/// catalog over the cleared state.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Drops the in-memory handle; the on-disk index stays for the next start.
pub fn clear() {
    *slot().write().unwrap_or_else(|error| error.into_inner()) = None;
}

/// Drops the handle and removes the index directory (library/index wipe).
pub fn wipe() {
    EPOCH.fetch_add(1, Ordering::Relaxed);
    clear();
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    if let Ok(dir) = crate::db::search_index_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The current index, opened from disk when its catalog key still matches
/// `library.db`, or rebuilt lazily on first use.
pub async fn ensure(pool: &SqlitePool) -> Result<Arc<TantivyIndex>> {
    if let Some(index) = slot()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        return Ok(index);
    }

    let catalog = crate::library::last_modified(pool).await?.to_string();
    let dir = crate::db::search_index_dir()?;
    let opened = tokio::task::spawn_blocking({
        let dir = dir.clone();
        let catalog = catalog.clone();
        move || TantivyIndex::open(&dir, &catalog)
    })
    .await?;
    if let Ok(index) = opened {
        let index = Arc::new(index);
        *slot().write().unwrap_or_else(|error| error.into_inner()) = Some(Arc::clone(&index));
        return Ok(index);
    }

    rebuild(pool).await?;
    slot()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .context("Tantivy index was not built")
}

/// Rebuilds the whole catalog into a fresh directory, then swaps it in.
pub async fn rebuild(pool: &SqlitePool) -> Result<()> {
    let epoch = EPOCH.load(Ordering::Relaxed);
    let catalog = crate::library::last_modified(pool).await?.to_string();
    let songs: Vec<SongHit> = sqlx::query_as(
        "SELECT s.id, s.title, s.artist, s.album, s.duration,
                COALESCE(s.play_count, 0) AS play_count,
                s.album_id,
                COALESCE(al.cover_art, s.cover_art) AS cover_art,
                s.track, s.disc_number
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id",
    )
    .fetch_all(pool)
    .await?;
    let artists: Vec<ArtistRow> =
        sqlx::query_as("SELECT id, name, album_count, cover_art, starred FROM artists")
            .fetch_all(pool)
            .await?;
    let albums: Vec<AlbumRow> = sqlx::query_as(
        "SELECT id, name AS title, artist, year, song_count, cover_art, starred, rating
         FROM albums",
    )
    .fetch_all(pool)
    .await?;

    let base = crate::db::search_index_dir()?;
    let index = tokio::task::spawn_blocking(move || {
        let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
        build_index(&base, &catalog, epoch, songs, artists, albums)
    })
    .await??;
    *slot().write().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(index));
    Ok(())
}

fn build_index(
    base: &Path,
    catalog: &str,
    epoch: u64,
    songs: Vec<SongHit>,
    artists: Vec<ArtistRow>,
    albums: Vec<AlbumRow>,
) -> Result<TantivyIndex> {
    let temp = base.with_extension("new");
    if temp.exists() {
        std::fs::remove_dir_all(&temp)
            .with_context(|| format!("could not clear {}", temp.display()))?;
    }
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&temp)?;

    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(&temp, schema.clone())?;
    register_tokenizers(&index);

    let mut writer: IndexWriter = index.writer(WRITER_HEAP)?;
    add_songs(&writer, &fields, &songs)?;
    for artist in &artists {
        writer.add_document(artist_doc(&fields, artist))?;
    }
    for album in &albums {
        writer.add_document(album_doc(&fields, album))?;
    }
    writer.commit()?;
    writer.wait_merging_threads()?;

    let meta = serde_json::to_vec(&IndexMeta {
        version: SCHEMA_VERSION,
        catalog: catalog.to_string(),
    })?;
    std::fs::write(temp.join(INDEX_META), meta)?;

    if EPOCH.load(Ordering::Relaxed) != epoch {
        let _ = std::fs::remove_dir_all(&temp);
        anyhow::bail!("search index build was superseded by a wipe");
    }

    if base.exists() {
        std::fs::remove_dir_all(base)
            .with_context(|| format!("could not replace {}", base.display()))?;
    }
    std::fs::rename(&temp, base)
        .with_context(|| format!("could not install {}", base.display()))?;

    TantivyIndex::open(base, catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, title: &str, artist: &str, album: &str, plays: i64) -> SongHit {
        SongHit {
            id: id.to_string(),
            title: title.to_string(),
            artist: Some(artist.to_string()),
            album: Some(album.to_string()),
            duration: Some(200),
            play_count: plays,
            album_id: Some(format!("{album}-id")),
            cover_art: None,
            track: Some(1),
            disc_number: Some(1),
        }
    }

    fn artist(id: &str, name: &str) -> ArtistRow {
        ArtistRow {
            id: id.to_string(),
            name: name.to_string(),
            album_count: 1,
            cover_art: None,
            starred: None,
        }
    }

    fn album(id: &str, title: &str, artist: &str) -> AlbumRow {
        AlbumRow {
            id: id.to_string(),
            title: title.to_string(),
            artist: Some(artist.to_string()),
            year: Some(2000),
            song_count: 1,
            cover_art: None,
            starred: None,
            rating: 0,
        }
    }

    fn test_index(
        songs: Vec<SongHit>,
        artists: Vec<ArtistRow>,
        albums: Vec<AlbumRow>,
    ) -> TantivyIndex {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index);
        let mut writer: IndexWriter = index.writer(15_000_000).unwrap();
        add_songs(&writer, &fields, &songs).unwrap();
        for artist in &artists {
            writer.add_document(artist_doc(&fields, artist)).unwrap();
        }
        for album in &albums {
            writer.add_document(album_doc(&fields, album)).unwrap();
        }
        writer.commit().unwrap();
        drop(writer);
        TantivyIndex::from_index(index).unwrap()
    }

    #[test]
    fn prefix_and_typo_match_artist_and_album() {
        let index = test_index(
            vec![song(
                "1",
                "Everything In Its Right Place",
                "Radiohead",
                "Kid A",
                0,
            )],
            vec![artist("a1", "Radiohead")],
            vec![album("al1", "Kid A", "Radiohead")],
        );

        let hits = index.search("radio", 10).unwrap();
        assert_eq!(hits.songs.len(), 1);
        assert_eq!(hits.artists.len(), 1);
        assert_eq!(hits.albums.len(), 1);

        let hits = index.search("radiohd", 10).unwrap();
        assert_eq!(hits.songs.len(), 1, "one edit is accepted");
        assert!(index.search("rdo", 10).unwrap().songs.is_empty());
    }

    #[test]
    fn multi_token_and_diacritics_fold() {
        let index = test_index(
            vec![song("1", "Hello", "Björk", "Homogenic", 0)],
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(index.search("bjork", 10).unwrap().songs.len(), 1);
        assert_eq!(index.search("hello bjork", 10).unwrap().songs.len(), 1);
        assert!(index.search("hello missing", 10).unwrap().songs.is_empty());
    }

    #[test]
    fn play_count_boosts_equal_text_scores() {
        let index = test_index(
            vec![
                song("quiet", "Hello", "A", "X", 0),
                song("loud", "Hello", "A", "X", 40),
            ],
            Vec::new(),
            Vec::new(),
        );

        let hits = index.search("hello", 10).unwrap();
        let ids: Vec<&str> = hits.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["loud", "quiet"]);
    }

    #[test]
    fn most_played_orders_by_count_then_title() {
        let index = test_index(
            vec![
                song("1", "Zebra", "A", "X", 0),
                song("2", "Alpha", "A", "X", 9),
                song("3", "Beta", "A", "X", 4),
                song("4", "Another", "A", "X", 9),
            ],
            Vec::new(),
            Vec::new(),
        );

        let songs = index.most_played(10).unwrap();
        let ids: Vec<&str> = songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["2", "4", "3", "1"]);
    }

    #[test]
    fn album_tracks_follow_disc_and_track() {
        let mut second = song("b", "Second", "A", "X", 0);
        second.track = Some(2);
        let mut first = song("a", "First", "A", "X", 0);
        first.track = Some(1);
        let mut disc2 = song("c", "Disc Two", "A", "X", 0);
        disc2.disc_number = Some(2);
        disc2.track = Some(1);
        let index = test_index(vec![second, first, disc2], Vec::new(), Vec::new());

        let tracks = index.album_tracks("X-id", 10).unwrap();
        let ids: Vec<&str> = tracks.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn persisted_index_reopens_with_its_catalog_key() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("musika-tantivy-{}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let index = build_index(
            &base,
            "42",
            EPOCH.load(Ordering::Relaxed),
            vec![song("1", "Hello", "A", "X", 0)],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(index.search("hello", 10).unwrap().songs.len(), 1);
        drop(index);

        let reopened = TantivyIndex::open(&base, "42").unwrap();
        assert_eq!(reopened.search("hello", 10).unwrap().songs.len(), 1);
        assert!(
            TantivyIndex::open(&base, "43").is_err(),
            "a changed catalog key must force a rebuild"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// `cargo test --release search::tantivy_index::tests::bench_100k_songs -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark, run explicitly"]
    fn bench_100k_songs() {
        use std::time::Instant;

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "musika-tantivy-bench-{}-{stamp}",
            std::process::id()
        ));
        let songs: Vec<SongHit> = (0..100_000)
            .map(|i| {
                song(
                    &format!("s{i}"),
                    &format!("Track {i}"),
                    "Radiohead",
                    "Kid A",
                    i % 100,
                )
            })
            .collect();

        let start = Instant::now();
        let index = build_index(
            &base,
            "bench",
            EPOCH.load(Ordering::Relaxed),
            songs,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        eprintln!("index build + open (100k): {:?}", start.elapsed());

        for query in ["rdo", "track", "radio", "zzz"] {
            let start = Instant::now();
            let hits = index.search(query, 20).unwrap();
            eprintln!("{query}: {:?} ({} hits)", start.elapsed(), hits.songs.len());
        }
        let start = Instant::now();
        let hits = index.most_played(20).unwrap();
        eprintln!("most_played: {:?} ({} hits)", start.elapsed(), hits.len());

        let _ = std::fs::remove_dir_all(&base);
    }
}
