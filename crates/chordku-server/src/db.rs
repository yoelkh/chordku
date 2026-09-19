//! SQLite persistence (bundled, WAL). One connection behind a mutex is plenty for a
//! single-user library; every query is a few microseconds.
//!
//! Chart data is split in three so nothing the user did is ever lost:
//!   analyses   immutable machine output, versioned by pipeline
//!   edits      the user's corrections, keyed by time range so they survive re-analysis
//!   view_prefs transpose / capo / display choices, per song

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MIGRATIONS: &[&str] = &[
    // 1
    r#"
    CREATE TABLE songs (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        artist      TEXT NOT NULL DEFAULT '',
        duration_ms INTEGER NOT NULL DEFAULT 0,
        src_kind    TEXT NOT NULL,              -- url | file | mic
        src_url     TEXT,
        src_hash    TEXT UNIQUE,                -- blake3 of the source bytes
        extractor   TEXT,
        media_id    TEXT,
        source_file TEXT,                       -- file name inside the song directory
        thumbnail   TEXT,
        fav         INTEGER NOT NULL DEFAULT 0,
        created_at  INTEGER NOT NULL,
        UNIQUE (extractor, media_id)
    );
    CREATE TABLE analyses (
        id           INTEGER PRIMARY KEY,
        song_id      TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        pipeline_ver INTEGER NOT NULL,
        variant      TEXT NOT NULL DEFAULT 'mix', -- mix | remix
        json         TEXT NOT NULL,
        active       INTEGER NOT NULL DEFAULT 1,
        created_at   INTEGER NOT NULL
    );
    CREATE INDEX analyses_song ON analyses(song_id, active);
    CREATE TABLE edits (
        id         INTEGER PRIMARY KEY,
        song_id    TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        t0_ms      INTEGER NOT NULL,
        t1_ms      INTEGER NOT NULL,
        op         TEXT NOT NULL,
        payload    TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL
    );
    CREATE INDEX edits_song ON edits(song_id, t0_ms);
    CREATE TABLE view_prefs (
        song_id TEXT PRIMARY KEY REFERENCES songs(id) ON DELETE CASCADE,
        json    TEXT NOT NULL
    );
    CREATE TABLE stems (
        song_id  TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        name     TEXT NOT NULL,
        file     TEXT NOT NULL,
        bytes    INTEGER NOT NULL,
        channels INTEGER NOT NULL,
        PRIMARY KEY (song_id, name)
    );
    CREATE TABLE jobs (
        id         TEXT PRIMARY KEY,
        song_id    TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        kind       TEXT NOT NULL,               -- analyze | separate
        state      TEXT NOT NULL,               -- queued | running | succeeded | failed | canceled
        stage      TEXT NOT NULL DEFAULT '',
        progress   REAL NOT NULL DEFAULT 0,
        error      TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX jobs_state ON jobs(state);
    CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
    CREATE TABLE song_tags (
        song_id TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (song_id, tag_id)
    );
    CREATE TABLE setlists (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE setlist_items (
        setlist_id INTEGER NOT NULL REFERENCES setlists(id) ON DELETE CASCADE,
        song_id    TEXT NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        position   INTEGER NOT NULL,
        PRIMARY KEY (setlist_id, song_id)
    );
    CREATE TABLE devices (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        token_hash TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL,
        last_seen  INTEGER NOT NULL
    );
    CREATE TABLE settings (k TEXT PRIMARY KEY, v TEXT NOT NULL);
    CREATE VIRTUAL TABLE songs_fts USING fts5(title, artist, content='songs', content_rowid='rowid');
    CREATE TRIGGER songs_ai AFTER INSERT ON songs BEGIN
        INSERT INTO songs_fts(rowid, title, artist) VALUES (new.rowid, new.title, new.artist);
    END;
    CREATE TRIGGER songs_ad AFTER DELETE ON songs BEGIN
        INSERT INTO songs_fts(songs_fts, rowid, title, artist) VALUES ('delete', old.rowid, old.title, old.artist);
    END;
    CREATE TRIGGER songs_au AFTER UPDATE OF title, artist ON songs BEGIN
        INSERT INTO songs_fts(songs_fts, rowid, title, artist) VALUES ('delete', old.rowid, old.title, old.artist);
        INSERT INTO songs_fts(rowid, title, artist) VALUES (new.rowid, new.title, new.artist);
    END;
    "#,
];

#[derive(Clone, Debug, Serialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u32,
    pub src_kind: String,
    pub src_url: Option<String>,
    pub thumbnail: Option<String>,
    pub fav: bool,
    pub created_at: i64,
    /// Key of the active analysis (`{tonic_pc, mode}`), for the library list.
    pub key: Option<Value>,
    pub bpm: Option<f64>,
    pub has_chart: bool,
    pub has_stems: bool,
    pub tags: Vec<String>,
    #[serde(skip)]
    pub source_file: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NewSong {
    pub title: String,
    pub artist: String,
    pub src_kind: String,
    pub src_url: Option<String>,
    pub extractor: Option<String>,
    pub media_id: Option<String>,
    pub thumbnail: Option<String>,
    pub duration_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub id: i64,
    pub t0_ms: u32,
    pub t1_ms: u32,
    pub op: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub song_id: String,
    pub kind: String,
    pub state: String,
    pub stage: String,
    pub progress: f64,
    pub error: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Setlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StemFile {
    pub name: String,
    pub file: String,
    pub bytes: u64,
    pub channels: u8,
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

const SONG_COLUMNS: &str = "s.id, s.title, s.artist, s.duration_ms, s.src_kind, s.src_url, s.thumbnail, s.fav, s.created_at, s.source_file,
    (SELECT json FROM analyses a WHERE a.song_id = s.id AND a.active = 1 ORDER BY a.id DESC LIMIT 1),
    EXISTS (SELECT 1 FROM stems t WHERE t.song_id = s.id AND t.name <> 'mix'),
    (SELECT group_concat(g.name, char(31)) FROM song_tags st JOIN tags g ON g.id = st.tag_id WHERE st.song_id = s.id)";

fn song_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Song> {
    let analysis: Option<String> = row.get(10)?;
    let parsed: Option<Value> = analysis.as_deref().and_then(|j| serde_json::from_str(j).ok());
    let tags: Option<String> = row.get(12)?;
    Ok(Song {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        duration_ms: row.get(3)?,
        src_kind: row.get(4)?,
        src_url: row.get(5)?,
        thumbnail: row.get(6)?,
        fav: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        source_file: row.get(9)?,
        key: parsed.as_ref().and_then(|a| a.get("key").cloned()).filter(|k| !k.is_null()),
        bpm: parsed.as_ref().and_then(|a| a.get("bpm").and_then(Value::as_f64)),
        has_chart: parsed.is_some(),
        has_stems: row.get::<_, i64>(11)? != 0,
        tags: {
            let mut list: Vec<String> = tags.map(|t| t.split('\u{1f}').map(str::to_owned).collect()).unwrap_or_default();
            list.sort_by_key(|t| t.to_lowercase());
            list
        },
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql).with_context(|| format!("migration {}", i + 1))?;
            tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
            tx.commit()?;
        }
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.0.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
        f(&mut guard)
    }

    // ---- songs -------------------------------------------------------------------------

    pub fn insert_song(&self, id: &str, s: &NewSong) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO songs (id, title, artist, duration_ms, src_kind, src_url, extractor, media_id, thumbnail, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![id, s.title, s.artist, s.duration_ms, s.src_kind, s.src_url, s.extractor, s.media_id, s.thumbnail, now()],
            )?;
            Ok(())
        })
    }

    pub fn song(&self, id: &str) -> Result<Option<Song>> {
        self.with(|c| {
            Ok(c.query_row(&format!("SELECT {SONG_COLUMNS} FROM songs s WHERE s.id = ?1"), [id], song_from_row).optional()?)
        })
    }

    pub fn find_by_media(&self, extractor: &str, media_id: &str) -> Result<Option<String>> {
        self.with(|c| {
            Ok(c.query_row("SELECT id FROM songs WHERE extractor = ?1 AND media_id = ?2", [extractor, media_id], |r| r.get(0))
                .optional()?)
        })
    }

    pub fn find_by_hash(&self, hash: &str) -> Result<Option<String>> {
        self.with(|c| Ok(c.query_row("SELECT id FROM songs WHERE src_hash = ?1", [hash], |r| r.get(0)).optional()?))
    }

    /// Library listing, newest first. `query` is matched as FTS prefixes on title / artist.
    pub fn songs(&self, query: Option<&str>, tag: Option<&str>, fav_only: bool) -> Result<Vec<Song>> {
        self.with(|c| {
            let mut sql = format!("SELECT {SONG_COLUMNS} FROM songs s WHERE 1=1");
            let mut args: Vec<String> = Vec::new();
            if let Some(q) = query.map(fts_query).filter(|q| !q.is_empty()) {
                args.push(q);
                sql.push_str(&format!(" AND s.rowid IN (SELECT rowid FROM songs_fts WHERE songs_fts MATCH ?{})", args.len()));
            }
            if let Some(t) = tag.filter(|t| !t.is_empty()) {
                args.push(t.to_owned());
                sql.push_str(&format!(
                    " AND s.id IN (SELECT st.song_id FROM song_tags st JOIN tags g ON g.id = st.tag_id WHERE g.name = ?{})",
                    args.len()
                ));
            }
            if fav_only {
                sql.push_str(" AND s.fav = 1");
            }
            sql.push_str(" ORDER BY s.created_at DESC, s.id DESC");
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), song_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn update_song_meta(&self, id: &str, title: Option<&str>, artist: Option<&str>, fav: Option<bool>) -> Result<()> {
        self.with(|c| {
            if let Some(t) = title {
                c.execute("UPDATE songs SET title = ?2 WHERE id = ?1", params![id, t])?;
            }
            if let Some(a) = artist {
                c.execute("UPDATE songs SET artist = ?2 WHERE id = ?1", params![id, a])?;
            }
            if let Some(f) = fav {
                c.execute("UPDATE songs SET fav = ?2 WHERE id = ?1", params![id, f as i64])?;
            }
            Ok(())
        })
    }

    pub fn set_song_source(&self, id: &str, file: &str, hash: &str, duration_ms: u32) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE songs SET source_file = ?2, src_hash = ?3, duration_ms = ?4 WHERE id = ?1",
                params![id, file, hash, duration_ms],
            )?;
            Ok(())
        })
    }

    pub fn delete_song(&self, id: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM songs WHERE id = ?1", [id])? > 0))
    }

    pub fn set_tags(&self, song_id: &str, tags: &[String]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM song_tags WHERE song_id = ?1", [song_id])?;
            for name in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
                tx.execute(
                    "INSERT OR IGNORE INTO song_tags (song_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
                    [song_id, name],
                )?;
            }
            tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM song_tags)", [])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn all_tags(&self) -> Result<Vec<String>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
        })
    }

    // ---- analyses, edits, prefs --------------------------------------------------------

    pub fn save_analysis(&self, song_id: &str, pipeline_ver: u32, variant: &str, json: &str) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE analyses SET active = 0 WHERE song_id = ?1", [song_id])?;
            tx.execute(
                "INSERT INTO analyses (song_id, pipeline_ver, variant, json, active, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                params![song_id, pipeline_ver, variant, json, now()],
            )?;
            // Keep the two most recent analyses per song.
            tx.execute(
                "DELETE FROM analyses WHERE song_id = ?1 AND id NOT IN (SELECT id FROM analyses WHERE song_id = ?1 ORDER BY id DESC LIMIT 2)",
                [song_id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn active_analysis(&self, song_id: &str) -> Result<Option<String>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT json FROM analyses WHERE song_id = ?1 AND active = 1 ORDER BY id DESC LIMIT 1",
                [song_id],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    pub fn edits(&self, song_id: &str) -> Result<Vec<Edit>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT id, t0_ms, t1_ms, op, payload FROM edits WHERE song_id = ?1 ORDER BY id")?;
            let rows = stmt.query_map([song_id], |r| {
                let payload: String = r.get(4)?;
                Ok(Edit {
                    id: r.get(0)?,
                    t0_ms: r.get(1)?,
                    t1_ms: r.get(2)?,
                    op: r.get(3)?,
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn add_edit(&self, song_id: &str, e: &Edit) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO edits (song_id, t0_ms, t1_ms, op, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![song_id, e.t0_ms, e.t1_ms, e.op, e.payload.to_string(), now()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn delete_edit(&self, song_id: &str, edit_id: i64) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM edits WHERE song_id = ?1 AND id = ?2", params![song_id, edit_id])? > 0))
    }

    pub fn clear_edits(&self, song_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM edits WHERE song_id = ?1", [song_id])?;
            Ok(())
        })
    }

    pub fn prefs(&self, song_id: &str) -> Result<Value> {
        self.with(|c| {
            let json: Option<String> =
                c.query_row("SELECT json FROM view_prefs WHERE song_id = ?1", [song_id], |r| r.get(0)).optional()?;
            Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_else(|| serde_json::json!({})))
        })
    }

    pub fn set_prefs(&self, song_id: &str, prefs: &Value) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO view_prefs (song_id, json) VALUES (?1, ?2) ON CONFLICT(song_id) DO UPDATE SET json = excluded.json",
                params![song_id, prefs.to_string()],
            )?;
            Ok(())
        })
    }

    // ---- stems -------------------------------------------------------------------------

    pub fn stems(&self, song_id: &str) -> Result<Vec<StemFile>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT name, file, bytes, channels FROM stems WHERE song_id = ?1 ORDER BY name")?;
            let rows = stmt.query_map([song_id], |r| {
                Ok(StemFile { name: r.get(0)?, file: r.get(1)?, bytes: r.get::<_, i64>(2)? as u64, channels: r.get(3)? })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn replace_stems(&self, song_id: &str, stems: &[StemFile]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM stems WHERE song_id = ?1", [song_id])?;
            for s in stems {
                tx.execute(
                    "INSERT INTO stems (song_id, name, file, bytes, channels) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![song_id, s.name, s.file, s.bytes as i64, s.channels],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    // ---- jobs --------------------------------------------------------------------------

    pub fn insert_job(&self, id: &str, song_id: &str, kind: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO jobs (id, song_id, kind, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'queued', ?4, ?4)",
                params![id, song_id, kind, now()],
            )?;
            Ok(())
        })
    }

    pub fn update_job(&self, id: &str, state: &str, stage: &str, progress: f64, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE jobs SET state = ?2, stage = ?3, progress = ?4, error = ?5, updated_at = ?6 WHERE id = ?1",
                params![id, state, stage, progress, error, now()],
            )?;
            Ok(())
        })
    }

    pub fn job(&self, id: &str) -> Result<Option<Job>> {
        self.with(|c| Ok(c.query_row(&format!("{JOB_SELECT} WHERE id = ?1"), [id], job_from_row).optional()?))
    }

    /// Jobs that are not finished, plus anything that finished in the last minute so a
    /// client that reconnects still learns the outcome.
    pub fn recent_jobs(&self) -> Result<Vec<Job>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!(
                "{JOB_SELECT} WHERE state IN ('queued', 'running') OR updated_at >= ?1 ORDER BY created_at"
            ))?;
            let rows = stmt.query_map([now() - 60], job_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// After a crash or restart nothing is actually running any more.
    pub fn requeue_interrupted_jobs(&self) -> Result<Vec<Job>> {
        self.with(|c| {
            c.execute("UPDATE jobs SET state = 'queued', stage = '', progress = 0 WHERE state = 'running'", [])?;
            let mut stmt = c.prepare(&format!("{JOB_SELECT} WHERE state = 'queued' ORDER BY created_at"))?;
            let rows = stmt.query_map([], job_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn active_job_for(&self, song_id: &str, kind: &str) -> Result<Option<String>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id FROM jobs WHERE song_id = ?1 AND kind = ?2 AND state IN ('queued', 'running') LIMIT 1",
                [song_id, kind],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    // ---- setlists ----------------------------------------------------------------------

    pub fn setlists(&self) -> Result<Vec<Setlist>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT l.id, l.name, (SELECT COUNT(*) FROM setlist_items i WHERE i.setlist_id = l.id) FROM setlists l ORDER BY l.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |r| Ok(Setlist { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn create_setlist(&self, name: &str) -> Result<i64> {
        self.with(|c| {
            c.execute("INSERT INTO setlists (name, created_at) VALUES (?1, ?2)", params![name, now()])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn rename_setlist(&self, id: i64, name: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("UPDATE setlists SET name = ?2 WHERE id = ?1", params![id, name])? > 0))
    }

    pub fn delete_setlist(&self, id: i64) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM setlists WHERE id = ?1", [id])? > 0))
    }

    pub fn setlist_songs(&self, id: i64) -> Result<Vec<Song>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {SONG_COLUMNS} FROM setlist_items i JOIN songs s ON s.id = i.song_id WHERE i.setlist_id = ?1 ORDER BY i.position"
            ))?;
            let rows = stmt.query_map([id], song_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Replace the content and order of a setlist. Unknown song ids are ignored.
    pub fn set_setlist_songs(&self, id: i64, song_ids: &[String]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM setlist_items WHERE setlist_id = ?1", [id])?;
            for (position, song_id) in song_ids.iter().enumerate() {
                tx.execute(
                    "INSERT OR IGNORE INTO setlist_items (setlist_id, song_id, position)
                     SELECT ?1, id, ?3 FROM songs WHERE id = ?2",
                    params![id, song_id, position as i64],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn append_to_setlist(&self, id: i64, song_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO setlist_items (setlist_id, song_id, position)
                 SELECT l.id, s.id, COALESCE((SELECT MAX(position) + 1 FROM setlist_items WHERE setlist_id = l.id), 0)
                 FROM setlists l, songs s WHERE l.id = ?1 AND s.id = ?2",
                params![id, song_id],
            )?;
            Ok(())
        })
    }

    // ---- paired devices (LAN mode) -----------------------------------------------------

    pub fn add_device(&self, id: &str, name: &str, token_hash: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO devices (id, name, token_hash, created_at, last_seen) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, token_hash, now()],
            )?;
            Ok(())
        })
    }

    /// True if the hash belongs to a paired device; refreshes its last-seen time.
    pub fn touch_device(&self, token_hash: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("UPDATE devices SET last_seen = ?2 WHERE token_hash = ?1", params![token_hash, now()])? > 0))
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT id, name, created_at, last_seen FROM devices ORDER BY created_at DESC")?;
            let rows = stmt.query_map([], |r| {
                Ok(Device { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, last_seen: r.get(3)? })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn delete_device(&self, id: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM devices WHERE id = ?1", [id])? > 0))
    }

    // ---- settings ----------------------------------------------------------------------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| Ok(c.query_row("SELECT v FROM settings WHERE k = ?1", [key], |r| r.get(0)).optional()?))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO settings (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [key, value],
            )?;
            Ok(())
        })
    }
}

const JOB_SELECT: &str = "SELECT id, song_id, kind, state, stage, progress, error, updated_at FROM jobs";

fn job_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: r.get(0)?,
        song_id: r.get(1)?,
        kind: r.get(2)?,
        state: r.get(3)?,
        stage: r.get(4)?,
        progress: r.get(5)?,
        error: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

/// Turn free text into a safe FTS5 prefix query: `"foo"* "bar"*`.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(db: &Db, id: &str, title: &str, artist: &str) {
        db.insert_song(id, &NewSong { title: title.into(), artist: artist.into(), src_kind: "file".into(), ..Default::default() })
            .unwrap();
    }

    #[test]
    fn library_search_tags_and_cascade() {
        let db = Db::memory().unwrap();
        song(&db, "a", "Laskar Pelangi", "Nidji");
        song(&db, "b", "Sempurna", "Andra and The Backbone");
        db.set_tags("a", &["pop".into(), "film".into()]).unwrap();

        assert_eq!(db.songs(Some("lask"), None, false).unwrap().len(), 1);
        assert_eq!(db.songs(Some("andra back"), None, false).unwrap()[0].id, "b");
        assert_eq!(db.songs(Some("\"); DROP TABLE songs; --"), None, false).unwrap().len(), 0);
        assert_eq!(db.songs(None, Some("film"), false).unwrap()[0].id, "a");
        assert_eq!(db.songs(None, None, false).unwrap().len(), 2);
        assert_eq!(db.song("a").unwrap().unwrap().tags, vec!["film", "pop"]);

        db.save_analysis("a", 1, "mix", r#"{"key":{"tonic_pc":7,"mode":"major"},"bpm":92.5}"#).unwrap();
        let a = db.song("a").unwrap().unwrap();
        assert!(a.has_chart && a.bpm == Some(92.5) && a.key.unwrap()["tonic_pc"] == 7);

        let id = db
            .add_edit("a", &Edit { id: 0, t0_ms: 1000, t1_ms: 3000, op: "set_chord".into(), payload: serde_json::json!({"class": 1}) })
            .unwrap();
        assert_eq!(db.edits("a").unwrap()[0].id, id);
        db.set_prefs("a", &serde_json::json!({"transpose": 2})).unwrap();
        assert_eq!(db.prefs("a").unwrap()["transpose"], 2);

        assert!(db.delete_song("a").unwrap());
        assert!(db.edits("a").unwrap().is_empty());
        assert!(db.all_tags().unwrap().is_empty() || db.songs(None, Some("film"), false).unwrap().is_empty());
    }

    #[test]
    fn duplicate_sources_are_detected() {
        let db = Db::memory().unwrap();
        db.insert_song(
            "a",
            &NewSong { title: "x".into(), src_kind: "url".into(), extractor: Some("youtube".into()), media_id: Some("abc".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(db.find_by_media("youtube", "abc").unwrap().as_deref(), Some("a"));
        db.set_song_source("a", "source.webm", "hash1", 1000).unwrap();
        assert_eq!(db.find_by_hash("hash1").unwrap().as_deref(), Some("a"));
    }

    #[test]
    fn interrupted_jobs_are_requeued() {
        let db = Db::memory().unwrap();
        song(&db, "a", "t", "");
        db.insert_job("j1", "a", "analyze").unwrap();
        db.update_job("j1", "running", "chords", 0.5, None).unwrap();
        let jobs = db.requeue_interrupted_jobs().unwrap();
        assert_eq!((jobs.len(), jobs[0].state.as_str(), jobs[0].progress), (1, "queued", 0.0));
        assert_eq!(db.active_job_for("a", "analyze").unwrap().as_deref(), Some("j1"));
    }
}
