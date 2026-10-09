//! The knowledge library's own SQLite file: sources, their items (one per
//! chunk), a full-text index over the items, and the entity graph the
//! extraction builds. After Kiro Crew's `knowledge/store.py`, without the
//! embedding column: search is keyword + graph, fused by reciprocal rank.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::chunk::Chunk;
use crate::extract::Extraction;
use crate::fts;

const SCHEMA_VERSION: i64 = 6;

/// Upgrades from each version to the next; `MIGRATIONS[v - 1]` takes v to v + 1.
const MIGRATIONS: &[&str] = &[
    // 1 -> 2: vectors from a remote embedding API, and which space they are in.
    "ALTER TABLE items ADD COLUMN embedding BLOB;
     ALTER TABLE items ADD COLUMN embedding_sig TEXT NOT NULL DEFAULT '';",
    // 2 -> 3: finding whether a space has vectors without scanning every item.
    "CREATE INDEX IF NOT EXISTS items_by_sig ON items(embedding_sig);",
    // 3 -> 4: the same passage in two documents is one passage (filled in on open).
    "ALTER TABLE items ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
     CREATE INDEX IF NOT EXISTS items_by_hash ON items(content_hash);",
    // 4 -> 5: libraries. Every document there is goes to the General one
    // (made on open, see `settle_libraries`), the column's default.
    "CREATE TABLE IF NOT EXISTS libraries (
         id          INTEGER PRIMARY KEY,
         name        TEXT NOT NULL,
         name_key    TEXT NOT NULL UNIQUE,
         created_at  INTEGER NOT NULL
     );
     ALTER TABLE sources ADD COLUMN library_id INTEGER NOT NULL DEFAULT 1;
     CREATE INDEX IF NOT EXISTS sources_by_library ON sources(library_id);",
    // 5 -> 6: a library's colour and tags, as a track's and a design's.
    "ALTER TABLE libraries ADD COLUMN color TEXT NOT NULL DEFAULT '';
     ALTER TABLE libraries ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';",
];

// `sources.library_id` names a row of `libraries` without a REFERENCES
// clause: SQLite cannot add such a column to a table that has rows (the
// migration above), and a new library and a migrated one should be the same.
// The library methods keep it right instead.
const SCHEMA: &str = r#"
CREATE TABLE libraries (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    name_key    TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    color       TEXT NOT NULL DEFAULT '',
    tags        TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE sources (
    id             TEXT PRIMARY KEY,
    source_type    TEXT NOT NULL,
    uri            TEXT NOT NULL UNIQUE,
    status         TEXT NOT NULL,
    error          TEXT NOT NULL DEFAULT '',
    content_hash   TEXT NOT NULL DEFAULT '',
    mtime_ms       INTEGER NOT NULL DEFAULT 0,
    size           INTEGER NOT NULL DEFAULT 0,
    last_synced    INTEGER,
    topic          TEXT NOT NULL DEFAULT '',
    themes         TEXT NOT NULL DEFAULT '[]',
    extracted      INTEGER NOT NULL DEFAULT 0,
    done           INTEGER NOT NULL DEFAULT 0,
    total          INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    library_id     INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX sources_by_library ON sources(library_id);
CREATE TABLE items (
    id           INTEGER PRIMARY KEY,
    source_id    TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    title        TEXT NOT NULL,
    content      TEXT NOT NULL,
    summary      TEXT NOT NULL DEFAULT '',
    category     TEXT NOT NULL DEFAULT 'document',
    tags         TEXT NOT NULL DEFAULT '[]',
    section      TEXT,
    line_start   INTEGER NOT NULL,
    line_end     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    embedding    BLOB,
    embedding_sig TEXT NOT NULL DEFAULT '',
    content_hash TEXT NOT NULL DEFAULT ''
);
CREATE INDEX items_by_source ON items(source_id, chunk_index);
CREATE INDEX items_by_sig ON items(embedding_sig);
CREATE INDEX items_by_hash ON items(content_hash);
CREATE VIRTUAL TABLE items_fts USING fts5(title, content, tags);
CREATE TABLE entities (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    name_key     TEXT NOT NULL UNIQUE,
    kind         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT ''
);
CREATE TABLE mentions (
    item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    PRIMARY KEY (item_id, entity_id)
);
CREATE INDEX mentions_by_entity ON mentions(entity_id);
CREATE TABLE relations (
    id           INTEGER PRIMARY KEY,
    source       INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    target       INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    item_id      INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE
);
CREATE INDEX relations_by_source ON relations(source);
CREATE INDEX relations_by_target ON relations(target);
"#;

/// Reciprocal rank fusion constant.
const RRF_K: f64 = 60.0;
/// Results below this fused score are noise (Kiro Crew's cut-off).
pub const MIN_SCORE: f64 = 0.012;

pub mod status {
    /// Added, waiting its turn.
    pub const PENDING: &str = "pending";
    /// Being chunked and extracted now.
    pub const INDEXING: &str = "indexing";
    pub const SYNCED: &str = "synced";
    /// The file is gone.
    pub const MISSING: &str = "missing";
    /// The same text is already in another source.
    pub const DUPLICATE: &str = "duplicate";
    pub const ERROR: &str = "error";
}

/// The library made once, on a new library file (and the one every document
/// already there went to when libraries came). It is a library like any other
/// after that: renamed, and deleted when empty, and not made again on open.
/// It is made again only to hold a document that would have none (see
/// [`KnowledgeDb::default_library`] and `settle_libraries`).
pub const GENERAL: i64 = 1;
/// Its name as stored; the UI shows it in the person's language until renamed.
pub const GENERAL_NAME: &str = "General";
/// Longest library name, in characters.
pub const MAX_LIBRARY_NAME: usize = 80;

/// A library: a named set of documents. Search, the conductor's tools and
/// `@kb` see every library; the knowledge page shows one at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    /// Documents in it.
    pub sources: i64,
    /// `#rrggbb`, or "" for none: the bar on its row, as a track's and a design's.
    pub color: String,
    /// Names from the app's one tag vocabulary, as a track's and a design's.
    pub tags: Vec<String>,
}

/// A document in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// The artifact that stands for it in lists.
    pub id: String,
    /// `local_file`.
    pub source_type: String,
    /// Absolute path.
    pub uri: String,
    pub status: String,
    pub error: String,
    pub content_hash: String,
    pub mtime_ms: i64,
    pub size: i64,
    pub last_synced: Option<i64>,
    pub topic: String,
    pub themes: Vec<String>,
    /// Whether an LLM extracted its items (false: titles are first lines).
    pub extracted: bool,
    /// Chunks done and total while indexing.
    pub done: i64,
    pub total: i64,
    pub items: i64,
    pub created_at: i64,
    /// The library it is in (one only).
    pub library_id: i64,
}

/// One chunk of a document, as listed and searched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub source_id: String,
    pub chunk_index: i64,
    pub title: String,
    pub content: String,
    pub summary: String,
    pub category: String,
    pub tags: Vec<String>,
    pub section: Option<String>,
    pub line_start: i64,
    pub line_end: i64,
    pub created_at: i64,
}

/// A search result.
/// A source that shares passages with another (see [`KnowledgeDb::overlaps`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Overlap {
    pub source: String,
    pub other: String,
    /// Passages of `source` also in `other`.
    pub shared: u32,
    /// That, as a share of `source`'s passages.
    pub percent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub item: Item,
    pub score: f64,
    /// Which searches found it: `keyword`, `graph`, or both joined by `+`.
    pub match_type: String,
    pub source_uri: String,
    /// The name of the library its document is in.
    pub library: String,
}

/// An item waiting for its vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToEmbed {
    pub id: i64,
    /// Tells the item apart from a later one given the same id.
    pub created_at: i64,
    pub text: String,
}

/// A chunk and what was extracted from it, ready to store.
#[derive(Debug, Clone)]
pub struct NewItem {
    pub chunk: Chunk,
    pub extraction: Option<Extraction>,
    pub tags: Vec<String>,
}

/// The file state a sync stored items for.
#[derive(Debug, Clone, Default)]
pub struct FileState {
    pub hash: String,
    pub mtime_ms: i64,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub description: String,
    /// Items that mention it.
    pub mentions: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: i64,
    pub target: i64,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// The entities a query reaches, as the graph leg of [`KnowledgeDb::search`]
/// reaches them: the ones it names, and what lies within two relations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryEntities {
    /// Named by the query.
    pub seeds: Vec<i64>,
    /// The seeds and everything within two relations of them.
    pub related: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub sources: i64,
    pub items: i64,
    pub entities: i64,
    pub relations: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

const SOURCE_SELECT: &str = "SELECT id, source_type, uri, status, error, content_hash, mtime_ms, size, last_synced, topic, themes,
    extracted, done, total, (SELECT COUNT(*) FROM items i WHERE i.source_id = s.id), created_at, library_id FROM sources s";

fn row_to_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        id: r.get(0)?,
        source_type: r.get(1)?,
        uri: r.get(2)?,
        status: r.get(3)?,
        error: r.get(4)?,
        content_hash: r.get(5)?,
        mtime_ms: r.get(6)?,
        size: r.get(7)?,
        last_synced: r.get(8)?,
        topic: r.get(9)?,
        themes: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or_default(),
        extracted: r.get::<_, i64>(11)? != 0,
        done: r.get(12)?,
        total: r.get(13)?,
        items: r.get(14)?,
        created_at: r.get(15)?,
        library_id: r.get(16)?,
    })
}

const LIBRARY_SELECT: &str =
    "SELECT l.id, l.name, l.created_at, (SELECT COUNT(*) FROM sources s WHERE s.library_id = l.id), l.color, l.tags FROM libraries l";

fn row_to_library(r: &rusqlite::Row<'_>) -> rusqlite::Result<Library> {
    Ok(Library {
        id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        sources: r.get(3)?,
        color: r.get(4)?,
        tags: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
    })
}

/// A library's name as kept: spaces at the ends dropped and runs of them
/// made one; refused when empty or longer than [`MAX_LIBRARY_NAME`].
pub fn library_name(name: &str) -> anyhow::Result<String> {
    let clean = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        anyhow::bail!("a library needs a name");
    }
    if clean.chars().count() > MAX_LIBRARY_NAME {
        anyhow::bail!("a library's name is at most {MAX_LIBRARY_NAME} characters");
    }
    Ok(clean)
}

/// Two names that differ only in case are one name.
fn name_key(name: &str) -> String {
    name.to_lowercase()
}

const ITEM_SELECT: &str = "SELECT id, source_id, chunk_index, title, content, summary, category, tags, section, line_start, line_end, created_at FROM items";

/// [`ITEM_SELECT`] with each item's source joined as `s`, items as `i`.
const ITEM_SELECT_BY_SOURCE: &str = "SELECT i.id, i.source_id, i.chunk_index, i.title, i.content, i.summary, i.category, i.tags, i.section,
    i.line_start, i.line_end, i.created_at FROM items i JOIN sources s ON s.id = i.source_id";

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        source_id: r.get(1)?,
        chunk_index: r.get(2)?,
        title: r.get(3)?,
        content: r.get(4)?,
        summary: r.get(5)?,
        category: r.get(6)?,
        tags: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        section: r.get(8)?,
        line_start: r.get(9)?,
        line_end: r.get(10)?,
        created_at: r.get(11)?,
    })
}

/// The library. All access is serialized.
pub struct KnowledgeDb {
    conn: Mutex<Connection>,
}

impl KnowledgeDb {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        Self::init(Connection::open(path)?, Some(path))
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    /// `path` is the file's, for the copy kept before an upgrade.
    fn init(mut conn: Connection, path: Option<&Path>) -> anyhow::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        let version: Option<i64> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok());
        match version {
            None => {
                let tx = conn.transaction()?;
                tx.execute_batch(SCHEMA)?;
                tx.execute("INSERT INTO meta(key, value) VALUES ('schema_version', ?1)", params![SCHEMA_VERSION.to_string()])?;
                tx.commit()?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) if (1..SCHEMA_VERSION).contains(&v) => {
                if let Some(path) = path {
                    backup_before_upgrade(&conn, path, v);
                }
                for step in v..SCHEMA_VERSION {
                    let tx = conn.transaction()?;
                    tx.execute_batch(MIGRATIONS[(step - 1) as usize])?;
                    tx.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", params![(step + 1).to_string()])?;
                    tx.commit()?;
                }
            }
            Some(v) => anyhow::bail!("knowledge schema version {v} is not supported by this build ({SCHEMA_VERSION})"),
        }
        settle_libraries(&conn)?;
        backfill_hashes(&mut conn)?;
        // A sync cut short by a quit resumes from the start next time.
        conn.execute(
            "UPDATE sources SET status = ?1 WHERE status = ?2",
            params![status::PENDING, status::INDEXING],
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Register a document in the default library ([`Self::default_library`]).
    /// Errors when the id or the path is taken.
    pub fn add_source(&self, id: &str, source_type: &str, uri: &str) -> anyhow::Result<Source> {
        let library = self.default_library()?;
        self.add_source_in(id, source_type, uri, library)
    }

    /// Where a document goes when no library is named: the first library
    /// (General first while it exists, then the oldest). With none at all,
    /// General is made again for it, so a document always has a library.
    pub fn default_library(&self) -> anyhow::Result<i64> {
        let conn = self.conn.lock();
        let first: Option<i64> = conn
            .query_row(&format!("SELECT id FROM libraries ORDER BY id != {GENERAL}, created_at, id LIMIT 1"), [], |r| r.get(0))
            .optional()?;
        if let Some(id) = first {
            return Ok(id);
        }
        make_general(&conn)?;
        Ok(GENERAL)
    }

    /// Register a document in a library. Errors when the id or the path is
    /// taken (a file is in one library only), or there is no such library.
    pub fn add_source_in(&self, id: &str, source_type: &str, uri: &str, library: i64) -> anyhow::Result<Source> {
        {
            let conn = self.conn.lock();
            require_library(&conn, library)?;
            conn.execute(
                "INSERT INTO sources(id, source_type, uri, status, created_at, library_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, source_type, uri, status::PENDING, now_ms(), library],
            )?;
        }
        self.source(id)?.ok_or_else(|| anyhow::anyhow!("source {id} vanished"))
    }

    /// Move a document to another library, with everything extracted from it.
    pub fn move_source(&self, id: &str, library: i64) -> anyhow::Result<Source> {
        {
            let conn = self.conn.lock();
            require_library(&conn, library)?;
            if conn.execute("UPDATE sources SET library_id = ?2 WHERE id = ?1", params![id, library])? == 0 {
                anyhow::bail!("no knowledge source {id}");
            }
        }
        self.source(id)?.ok_or_else(|| anyhow::anyhow!("source {id} vanished"))
    }

    /// Every library, General first, then oldest first.
    pub fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{LIBRARY_SELECT} ORDER BY l.id != {GENERAL}, l.created_at, l.id"))?;
        let rows = stmt.query_map([], row_to_library)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn library(&self, id: i64) -> anyhow::Result<Option<Library>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{LIBRARY_SELECT} WHERE l.id = ?1"), params![id], row_to_library).optional()?)
    }

    /// The library of that name (case aside).
    pub fn library_named(&self, name: &str) -> anyhow::Result<Option<Library>> {
        let key = name_key(&library_name(name)?);
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{LIBRARY_SELECT} WHERE l.name_key = ?1"), params![key], row_to_library).optional()?)
    }

    /// A new, empty library. Errors when the name is empty, too long, or taken (case aside).
    pub fn create_library(&self, name: &str) -> anyhow::Result<Library> {
        let name = library_name(name)?;
        let id = {
            let conn = self.conn.lock();
            if name_taken(&conn, &name, None)? {
                anyhow::bail!("there is already a library named {name}");
            }
            conn.execute(
                "INSERT INTO libraries(name, name_key, created_at) VALUES (?1, ?2, ?3)",
                params![name, name_key(&name), now_ms()],
            )?;
            conn.last_insert_rowid()
        };
        self.library(id)?.ok_or_else(|| anyhow::anyhow!("library {id} vanished"))
    }

    /// Rename a library (General too). The same rules as [`Self::create_library`].
    pub fn rename_library(&self, id: i64, name: &str) -> anyhow::Result<Library> {
        let name = library_name(name)?;
        {
            let conn = self.conn.lock();
            require_library(&conn, id)?;
            if name_taken(&conn, &name, Some(id))? {
                anyhow::bail!("there is already a library named {name}");
            }
            conn.execute("UPDATE libraries SET name = ?2, name_key = ?3 WHERE id = ?1", params![id, name, name_key(&name)])?;
        }
        self.library(id)?.ok_or_else(|| anyhow::anyhow!("library {id} vanished"))
    }

    /// Set a library's colour and/or tags (General's too). The values are
    /// taken as given: the app checks them as it checks a track's.
    pub fn set_library_look(&self, id: i64, color: Option<&str>, tags: Option<&[String]>) -> anyhow::Result<Library> {
        {
            let conn = self.conn.lock();
            require_library(&conn, id)?;
            if let Some(c) = color {
                conn.execute("UPDATE libraries SET color = ?2 WHERE id = ?1", params![id, c])?;
            }
            if let Some(t) = tags {
                conn.execute("UPDATE libraries SET tags = ?2 WHERE id = ?1", params![id, serde_json::to_string(t)?])?;
            }
        }
        self.library(id)?.ok_or_else(|| anyhow::anyhow!("library {id} vanished"))
    }

    /// Delete an empty library, General too (the last one as well: then
    /// there are none). A library with documents is refused: they are moved
    /// or removed first, by the person, so nothing goes anywhere unasked.
    pub fn delete_library(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        require_library(&conn, id)?;
        let docs: i64 = conn.query_row("SELECT COUNT(*) FROM sources WHERE library_id = ?1", params![id], |r| r.get(0))?;
        if docs > 0 {
            anyhow::bail!("{docs} documents are still in this library; move or remove them first");
        }
        conn.execute("DELETE FROM libraries WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn source(&self, id: &str) -> anyhow::Result<Option<Source>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{SOURCE_SELECT} WHERE id = ?1"), params![id], row_to_source).optional()?)
    }

    /// The source for a path, compared as given.
    pub fn source_by_uri(&self, uri: &str) -> anyhow::Result<Option<Source>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{SOURCE_SELECT} WHERE uri = ?1"), params![uri], row_to_source).optional()?)
    }

    /// Every source, newest first.
    pub fn sources(&self) -> anyhow::Result<Vec<Source>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{SOURCE_SELECT} ORDER BY created_at DESC, id DESC"))?;
        let rows = stmt.query_map([], row_to_source)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_status(&self, id: &str, status: &str, error: &str) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("UPDATE sources SET status = ?2, error = ?3 WHERE id = ?1", params![id, status, error])?;
        Ok(())
    }

    pub fn set_progress(&self, id: &str, done: usize, total: usize) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("UPDATE sources SET done = ?2, total = ?3 WHERE id = ?1", params![id, done as i64, total as i64])?;
        Ok(())
    }

    /// Remember the file's size and time without re-indexing (it was touched
    /// but its text did not change).
    pub fn touch(&self, id: &str, file: &FileState) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sources SET mtime_ms = ?2, size = ?3, last_synced = ?4, status = ?5, error = '' WHERE id = ?1",
            params![id, file.mtime_ms, file.size, now_ms(), status::SYNCED],
        )?;
        Ok(())
    }

    /// Remember a file's time and size alone (it could not be indexed), so
    /// it is retried only once it changes.
    pub fn remember_file(&self, id: &str, mtime_ms: i64, size: i64) -> anyhow::Result<()> {
        self.conn
            .lock()
            .execute("UPDATE sources SET mtime_ms = ?2, size = ?3 WHERE id = ?1", params![id, mtime_ms, size])?;
        Ok(())
    }

    /// Another source in the same library already holding exactly this text
    /// (the same file in two libraries is two documents); General's, for an
    /// id not in the library.
    pub fn hash_owner(&self, hash: &str, except: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id FROM sources WHERE content_hash = ?1 AND id != ?2 AND status = ?3
                 AND library_id = COALESCE((SELECT library_id FROM sources WHERE id = ?2), 1) LIMIT 1",
                params![hash, except, status::SYNCED],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Replace a source's items with a fresh sync's, and mark it synced.
    pub fn replace_items(&self, id: &str, items: &[NewItem], extracted: bool, file: &FileState) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        clear_items(&tx, id)?;
        let now = now_ms();
        for new in items {
            let x = new.extraction.clone().unwrap_or_default();
            let title = if x.title.is_empty() { crate::extract::first_line_title(&new.chunk.content) } else { x.title.clone() };
            let category = if x.category.is_empty() { crate::extract::DEFAULT_CATEGORY.to_string() } else { x.category.clone() };
            let tags = serde_json::to_string(&new.tags)?;
            tx.execute(
                "INSERT INTO items(source_id, chunk_index, title, content, summary, category, tags, section, line_start, line_end, created_at, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    new.chunk.index as i64,
                    title,
                    new.chunk.content,
                    x.summary,
                    category,
                    tags,
                    new.chunk.section,
                    new.chunk.line_start as i64,
                    new.chunk.line_end as i64,
                    now,
                    passage_hash(&new.chunk.content)
                ],
            )?;
            let item = tx.last_insert_rowid();
            let indexed_tags = format!("{} {}", new.tags.join(" "), new.chunk.section.clone().unwrap_or_default());
            tx.execute(
                "INSERT INTO items_fts(rowid, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
                params![item, fts::segment(&title), fts::segment(&format!("{}\n{}", x.summary, new.chunk.content)), fts::segment(&indexed_tags)],
            )?;
            let mut ids: HashMap<String, i64> = HashMap::new();
            for e in &x.entities {
                let key = e.name.to_lowercase();
                tx.execute(
                    "INSERT INTO entities(name, name_key, kind, description) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(name_key) DO UPDATE SET description = CASE WHEN entities.description = '' THEN excluded.description ELSE entities.description END",
                    params![e.name, key, e.kind, e.description],
                )?;
                let eid: i64 = tx.query_row("SELECT id FROM entities WHERE name_key = ?1", params![key], |r| r.get(0))?;
                tx.execute("INSERT OR IGNORE INTO mentions(item_id, entity_id) VALUES (?1, ?2)", params![item, eid])?;
                ids.insert(key, eid);
            }
            for r in &x.relations {
                let (Some(s), Some(t)) = (ids.get(&r.source.to_lowercase()), ids.get(&r.target.to_lowercase())) else {
                    continue;
                };
                tx.execute(
                    "INSERT INTO relations(source, target, kind, description, item_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![s, t, r.kind, r.description, item],
                )?;
            }
        }
        tx.execute(
            "UPDATE sources SET status = ?2, error = '', content_hash = ?3, mtime_ms = ?4, size = ?5, last_synced = ?6,
             extracted = ?7, done = ?8, total = ?8 WHERE id = ?1",
            params![id, status::SYNCED, file.hash, file.mtime_ms, file.size, now, extracted as i64, items.len() as i64],
        )?;
        prune_entities(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_summary(&self, id: &str, topic: &str, themes: &[String]) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE sources SET topic = ?2, themes = ?3 WHERE id = ?1",
            params![id, topic, serde_json::to_string(themes)?],
        )?;
        Ok(())
    }

    /// Drop a source and everything extracted from it.
    pub fn delete_source(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        clear_items(&tx, id)?;
        tx.execute("DELETE FROM sources WHERE id = ?1", params![id])?;
        prune_entities(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Items, in document order, of one source or of all (newest
    /// documents first), at most `limit`.
    pub fn items(&self, source: Option<&str>, limit: usize) -> anyhow::Result<Vec<Item>> {
        self.items_in(source, None, limit)
    }

    /// [`Self::items`] of one library, or of every one (`None`).
    pub fn items_in(&self, source: Option<&str>, library: Option<i64>, limit: usize) -> anyhow::Result<Vec<Item>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{ITEM_SELECT_BY_SOURCE} WHERE (?1 IS NULL OR i.source_id = ?1) AND (?3 IS NULL OR s.library_id = ?3)
             ORDER BY s.created_at DESC, i.source_id, i.chunk_index LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![source, limit as i64, library], row_to_item)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn hash_of(&self, id: i64) -> Option<String> {
        self.conn.lock().query_row("SELECT content_hash FROM items WHERE id = ?1", params![id], |r| r.get(0)).ok()
    }

    /// Sources that share passages: for each source, the other source it
    /// shares the most with and what share of its own passages that is.
    /// Only shares of at least `min_percent`.
    pub fn overlaps(&self, min_percent: u32) -> anyhow::Result<Vec<Overlap>> {
        let conn = self.conn.lock();
        let mut totals: HashMap<String, i64> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT source_id, COUNT(DISTINCT content_hash) FROM items WHERE content_hash != '' GROUP BY source_id")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (s, n) = row?;
                totals.insert(s, n);
            }
        }
        // Within a library: a document copied into another library is not a duplicate to tidy.
        let mut stmt = conn.prepare(
            "SELECT a.source_id, b.source_id, COUNT(DISTINCT a.content_hash)
             FROM items a JOIN items b ON a.content_hash = b.content_hash AND a.source_id != b.source_id
             JOIN sources sa ON sa.id = a.source_id JOIN sources sb ON sb.id = b.source_id
             WHERE a.content_hash != '' AND sa.library_id = sb.library_id
             GROUP BY a.source_id, b.source_id",
        )?;
        let mut best: HashMap<String, Overlap> = HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
            let (source, other, shared) = row?;
            let total = totals.get(&source).copied().unwrap_or(0).max(1);
            let percent = ((shared * 100) / total) as u32;
            if percent < min_percent {
                continue;
            }
            let better = best.get(&source).is_none_or(|o| percent > o.percent);
            if better {
                best.insert(source.clone(), Overlap { source, other, shared: shared as u32, percent });
            }
        }
        let mut out: Vec<Overlap> = best.into_values().collect();
        out.sort_by(|a, b| b.percent.cmp(&a.percent).then(a.source.cmp(&b.source)));
        Ok(out)
    }

    /// What the graph view's conversation is shown of the library: the
    /// entities picked (name, kind, how often mentioned, description), the
    /// relations among them, and the passages picked, each with its document
    /// and lines, cut at `max_chars`. Ids that no longer exist are skipped;
    /// nothing picked (or nothing left) is the empty string.
    pub fn selection_context(&self, entities: &[i64], items: &[i64], max_chars: usize) -> anyhow::Result<String> {
        let mut found: Vec<GraphNode> = Vec::new();
        let mut rels: Vec<GraphEdge> = Vec::new();
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT e.id, e.name, e.kind, e.description, (SELECT COUNT(*) FROM mentions m WHERE m.entity_id = e.id) FROM entities e WHERE e.id = ?1")?;
            let mut seen = HashSet::new();
            for id in entities {
                if !seen.insert(*id) {
                    continue;
                }
                let row = stmt
                    .query_row(params![id], |r| {
                        Ok(GraphNode {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            kind: r.get(2)?,
                            description: r.get(3)?,
                            mentions: r.get(4)?,
                        })
                    })
                    .optional()?;
                found.extend(row);
            }
            if found.len() > 1 {
                let ids: HashSet<i64> = found.iter().map(|n| n.id).collect();
                let mut stmt = conn.prepare("SELECT DISTINCT source, target, kind FROM relations ORDER BY source, target, kind")?;
                rels = stmt
                    .query_map([], |r| {
                        Ok(GraphEdge {
                            source: r.get(0)?,
                            target: r.get(1)?,
                            kind: r.get(2)?,
                        })
                    })?
                    .filter_map(Result::ok)
                    .filter(|e| e.source != e.target && ids.contains(&e.source) && ids.contains(&e.target))
                    .collect();
            }
        }
        let mut passages: Vec<(Item, String)> = Vec::new();
        let mut seen = HashSet::new();
        for id in items {
            if !seen.insert(*id) {
                continue;
            }
            let Some(item) = self.item(*id)? else { continue };
            let doc = self.source(&item.source_id)?.map(|s| s.uri).unwrap_or_else(|| item.source_id.clone());
            let doc = doc.rsplit(['/', '\\']).next().unwrap_or(&doc).to_string();
            passages.push((item, doc));
        }
        if found.is_empty() && passages.is_empty() {
            return Ok(String::new());
        }
        let name = |id: i64| found.iter().find(|n| n.id == id).map(|n| n.name.as_str()).unwrap_or("?");
        let mut out = String::from("[selection from the knowledge graph]");
        if !found.is_empty() {
            out.push_str(&format!("\nentities ({}):", found.len()));
            for n in &found {
                let desc = n.description.trim();
                out.push_str(&format!(
                    "\n- {} ({}, mentioned {}x){}",
                    n.name,
                    n.kind,
                    n.mentions,
                    if desc.is_empty() { String::new() } else { format!(": {desc}") }
                ));
            }
        }
        if !rels.is_empty() {
            out.push_str(&format!("\nrelations ({}):", rels.len()));
            for e in &rels {
                out.push_str(&format!("\n- {} -{}-> {}", name(e.source), e.kind, name(e.target)));
            }
        }
        if !passages.is_empty() {
            out.push_str(&format!("\npassages ({}):", passages.len()));
            for (item, doc) in &passages {
                let text = item.content.trim();
                let cut: String = text.chars().take(max_chars).collect();
                let more = if cut.len() < text.len() { " [...]" } else { "" };
                out.push_str(&format!("\n### {} — {} · lines {}-{}\n{cut}{more}", item.title.trim(), doc, item.line_start, item.line_end));
            }
        }
        Ok(out)
    }

    pub fn item(&self, id: i64) -> anyhow::Result<Option<Item>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{ITEM_SELECT} WHERE id = ?1"), params![id], row_to_item).optional()?)
    }

    pub fn stats(&self) -> anyhow::Result<Stats> {
        self.stats_in(None)
    }

    /// Counts of one library, or of every one (`None`). Entities are shared
    /// by the libraries; a library counts those its documents mention, and
    /// the relations its documents state.
    pub fn stats_in(&self, library: Option<i64>) -> anyhow::Result<Stats> {
        let conn = self.conn.lock();
        let count = |sql: &str| conn.query_row(sql, params![library], |r| r.get::<_, i64>(0));
        Ok(Stats {
            sources: count("SELECT COUNT(*) FROM sources WHERE ?1 IS NULL OR library_id = ?1")?,
            items: count("SELECT COUNT(*) FROM items i JOIN sources s ON s.id = i.source_id WHERE ?1 IS NULL OR s.library_id = ?1")?,
            entities: count(
                "SELECT COUNT(DISTINCT m.entity_id) FROM mentions m JOIN items i ON i.id = m.item_id
                 JOIN sources s ON s.id = i.source_id WHERE ?1 IS NULL OR s.library_id = ?1",
            )?,
            relations: count(
                "SELECT COUNT(*) FROM relations r JOIN items i ON i.id = r.item_id
                 JOIN sources s ON s.id = i.source_id WHERE ?1 IS NULL OR s.library_id = ?1",
            )?,
        })
    }

    /// The entity graph, the most mentioned entities first, up to `limit`.
    pub fn graph(&self, limit: usize) -> anyhow::Result<Graph> {
        self.graph_in(None, limit)
    }

    /// The entity graph of one library, or of every one (`None`): the
    /// entities its documents mention (counted there), and the relations its
    /// documents state among them.
    pub fn graph_in(&self, library: Option<i64>, limit: usize) -> anyhow::Result<Graph> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT e.id, e.name, e.kind, e.description, COUNT(*) AS n
             FROM entities e JOIN mentions m ON m.entity_id = e.id JOIN items i ON i.id = m.item_id JOIN sources s ON s.id = i.source_id
             WHERE ?1 IS NULL OR s.library_id = ?1
             GROUP BY e.id ORDER BY n DESC, e.id LIMIT ?2",
        )?;
        let nodes: Vec<GraphNode> = stmt
            .query_map(params![library, limit as i64], |r| {
                Ok(GraphNode { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, description: r.get(3)?, mentions: r.get(4)? })
            })?
            .collect::<Result<_, _>>()?;
        let shown: HashSet<i64> = nodes.iter().map(|n| n.id).collect();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT r.source, r.target, r.kind FROM relations r JOIN items i ON i.id = r.item_id JOIN sources s ON s.id = i.source_id
             WHERE ?1 IS NULL OR s.library_id = ?1",
        )?;
        let edges = stmt
            .query_map(params![library], |r| Ok(GraphEdge { source: r.get(0)?, target: r.get(1)?, kind: r.get(2)? }))?
            .filter_map(Result::ok)
            .filter(|e| shown.contains(&e.source) && shown.contains(&e.target))
            .collect();
        Ok(Graph { nodes, edges })
    }

    /// Items that mention an entity.
    pub fn entity_items(&self, entity: i64) -> anyhow::Result<Vec<Item>> {
        self.entity_items_in(entity, None)
    }

    /// Items of one library, or of every one (`None`), that mention an entity.
    pub fn entity_items_in(&self, entity: i64, library: Option<i64>) -> anyhow::Result<Vec<Item>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{ITEM_SELECT_BY_SOURCE} WHERE i.id IN (SELECT item_id FROM mentions WHERE entity_id = ?1) AND (?2 IS NULL OR s.library_id = ?2)
             ORDER BY i.source_id, i.chunk_index"
        ))?;
        let rows = stmt.query_map(params![entity, library], row_to_item)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keyword, graph and (given the query's vector and the space it is
    /// in) vector search, fused by reciprocal rank with the vector leg
    /// counting double, as Kiro Crew weighs it. The keyword search's best
    /// match is always kept, so up to `limit + 1` come back. Results under
    /// [`MIN_SCORE`] are dropped.
    pub fn search(&self, query: &str, limit: usize, source: Option<&str>, vector: Option<(&[f32], &str)>) -> anyhow::Result<Vec<Hit>> {
        self.search_in(query, limit, source, None, vector)
    }

    /// [`Self::search`] in one library, or in every one (`None`).
    pub fn search_in(&self, query: &str, limit: usize, source: Option<&str>, library: Option<i64>, vector: Option<(&[f32], &str)>) -> anyhow::Result<Vec<Hit>> {
        let limit = limit.max(1);
        let keyword = self.keyword_search(query, 20, source, library)?;
        let graph = self.graph_search(query, 20, source, library)?;
        let semantic = match vector {
            Some((v, sig)) => self.vector_search(v, sig, 20, source, library)?,
            None => Vec::new(),
        };
        let mut scores: BTreeMap<i64, (f64, Vec<&str>)> = BTreeMap::new();
        for (leg, name, weight) in [(&keyword, "keyword", 1.0), (&graph, "graph", 1.0), (&semantic, "vector", VECTOR_WEIGHT)] {
            for (rank, id) in leg.iter().enumerate() {
                let e = scores.entry(*id).or_insert((0.0, Vec::new()));
                e.0 += weight / (RRF_K + rank as f64 + 1.0);
                e.1.push(name);
            }
        }
        let mut ranked: Vec<(i64, f64, String)> = scores.into_iter().map(|(id, (s, legs))| (id, s, legs.join("+"))).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(b.0.cmp(&a.0)));
        // The same passage in several documents (a copy, another version)
        // takes one place, its best-ranked copy's.
        let mut seen: HashSet<String> = HashSet::new();
        let mut first_copy = |id: i64| -> bool {
            match self.hash_of(id) {
                Some(h) if !h.is_empty() => seen.insert(h),
                _ => true,
            }
        };
        let mut keep: Vec<(i64, f64, String)> = Vec::new();
        for k in &ranked {
            if keep.len() >= limit {
                break;
            }
            if first_copy(k.0) {
                keep.push(k.clone());
            }
        }
        if let Some(top) = keyword.first() {
            if !keep.iter().any(|k| k.0 == *top) {
                if let Some(k) = ranked.iter().find(|k| k.0 == *top) {
                    if first_copy(k.0) {
                        keep.push(k.clone());
                    }
                }
            }
        }
        let mut hits = Vec::new();
        for (id, score, match_type) in keep {
            if score < MIN_SCORE {
                continue;
            }
            let Some(item) = self.item(id)? else { continue };
            let source = self.source(&item.source_id)?;
            let library = match &source {
                Some(s) => self.library(s.library_id)?.map(|l| l.name).unwrap_or_default(),
                None => String::new(),
            };
            let source_uri = source.map(|s| s.uri).unwrap_or_default();
            hits.push(Hit { item, score, match_type, source_uri, library });
        }
        Ok(hits)
    }

    /// Items closest to `query` by cosine similarity, among those embedded
    /// in the space `sig` names. A brute-force scan: fine for the tens of
    /// thousands of items a personal library holds.
    fn vector_search(&self, query: &[f32], sig: &str, limit: usize, source: Option<&str>, library: Option<i64>) -> anyhow::Result<Vec<i64>> {
        let qn = norm(query);
        if qn == 0.0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, embedding FROM items WHERE embedding IS NOT NULL AND embedding_sig = ?1 AND (?2 IS NULL OR source_id = ?2)
             AND (?3 IS NULL OR source_id IN (SELECT id FROM sources WHERE library_id = ?3))",
        )?;
        let mut scored: Vec<(i64, f32)> = stmt
            .query_map(params![sig, source, library], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .filter_map(Result::ok)
            .filter_map(|(id, blob)| {
                let v = from_blob(&blob);
                (v.len() == query.len()).then(|| {
                    let dot: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
                    let vn = norm(&v);
                    (id, if vn == 0.0 { 0.0 } else { dot / (vn * qn) })
                })
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(limit).map(|(id, _)| id).collect())
    }

    /// Items after `after` (by id) not yet embedded in the space `sig`
    /// names, nor given up on in it, up to `limit`: the text to embed
    /// (title, summary, content) and the item's stamp for [`Self::set_embeddings`].
    pub fn to_embed(&self, sig: &str, after: i64, limit: usize) -> anyhow::Result<Vec<ToEmbed>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, created_at, title, summary, content FROM items
             WHERE id > ?3 AND (embedding IS NULL OR embedding_sig != ?1) AND embedding_sig != ('failed:' || ?1)
             ORDER BY id LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![sig, limit as i64, after], |r| {
            let text = format!("{}\n{}\n{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?);
            Ok(ToEmbed { id: r.get(0)?, created_at: r.get(1)?, text: text.chars().take(EMBED_CHARS).collect() })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keep vectors for items. An item that is gone, or whose id now names
    /// a newer item (ids of a re-synced source are reused), is skipped.
    pub fn set_embeddings(&self, sig: &str, vectors: &[(&ToEmbed, Vec<f32>)]) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (item, v) in vectors {
            tx.execute(
                "UPDATE items SET embedding = ?3, embedding_sig = ?4 WHERE id = ?1 AND created_at = ?2",
                params![item.id, item.created_at, to_blob(v), sig],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Give up on embedding an item in the space `sig` names (the endpoint
    /// refuses it); it is not retried until the space changes.
    pub fn embed_failed(&self, sig: &str, item: &ToEmbed) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "UPDATE items SET embedding = NULL, embedding_sig = 'failed:' || ?3 WHERE id = ?1 AND created_at = ?2",
            params![item.id, item.created_at, sig],
        )?;
        Ok(())
    }

    /// Try again every item given up on, in any space.
    pub fn retry_refused(&self) -> anyhow::Result<usize> {
        Ok(self.conn.lock().execute("UPDATE items SET embedding_sig = '' WHERE embedding_sig LIKE 'failed:%'", [])?)
    }

    /// Whether any item has a vector in the space `sig` names.
    pub fn has_vectors(&self, sig: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row("SELECT 1 FROM items WHERE embedding_sig = ?1 AND embedding IS NOT NULL LIMIT 1", params![sig], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Items embedded in the space `sig` names, items given up on in it,
    /// and all items.
    pub fn embedded(&self, sig: &str) -> anyhow::Result<(i64, i64, i64)> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FILTER (WHERE embedding IS NOT NULL AND embedding_sig = ?1),
                    COUNT(*) FILTER (WHERE embedding_sig = 'failed:' || ?1), COUNT(*) FROM items",
            params![sig],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    fn keyword_search(&self, query: &str, limit: usize, source: Option<&str>, library: Option<i64>) -> anyhow::Result<Vec<i64>> {
        let Some(expr) = fts::match_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT i.id FROM items_fts f JOIN items i ON i.id = f.rowid
             WHERE items_fts MATCH ?1 AND (?2 IS NULL OR i.source_id = ?2)
             AND (?4 IS NULL OR i.source_id IN (SELECT id FROM sources WHERE library_id = ?4)) ORDER BY f.rank LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![expr, source, limit as i64, library], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Entities named in the query, their neighbours two hops out, and the
    /// items that mention most of them.
    /// The entities a query names, and those within two relations of them:
    /// what the graph view lights up for the search it shows.
    pub fn query_entities(&self, query: &str) -> anyhow::Result<QueryEntities> {
        let conn = self.conn.lock();
        let (seeds, seen) = reach(&conn, query)?;
        let mut seeds: Vec<i64> = seeds.into_iter().collect();
        let mut related: Vec<i64> = seen.into_iter().collect();
        seeds.sort_unstable();
        related.sort_unstable();
        Ok(QueryEntities { seeds, related })
    }

    fn graph_search(&self, query: &str, limit: usize, source: Option<&str>, library: Option<i64>) -> anyhow::Result<Vec<i64>> {
        let conn = self.conn.lock();
        let (seeds, seen) = reach(&conn, query)?;
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let list = seen.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT m.item_id FROM mentions m JOIN items i ON i.id = m.item_id
             WHERE m.entity_id IN ({list}) AND (?1 IS NULL OR i.source_id = ?1)
             AND (?3 IS NULL OR i.source_id IN (SELECT id FROM sources WHERE library_id = ?3))
             GROUP BY m.item_id ORDER BY COUNT(*) DESC, m.item_id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![source, limit as i64, library], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The entities a query names (its seeds), and the seeds with everything
/// within two relations of them. Shared by the graph leg of search and by
/// [`KnowledgeDb::query_entities`], so the view lights what search followed.
fn reach(conn: &Connection, query: &str) -> anyhow::Result<(HashSet<i64>, HashSet<i64>)> {
    let mut seeds: HashSet<i64> = HashSet::new();
    for term in fts::entity_candidates(query) {
        if let Some(id) = conn
            .query_row("SELECT id FROM entities WHERE name_key = ?1", params![term.to_lowercase()], |r| r.get::<_, i64>(0))
            .optional()?
        {
            seeds.insert(id);
        }
    }
    let mut seen: HashSet<i64> = seeds.clone();
    if seeds.is_empty() {
        return Ok((seeds, seen));
    }
    let mut queue: VecDeque<(i64, usize)> = seeds.iter().map(|s| (*s, 0)).collect();
    let mut stmt = conn.prepare("SELECT target FROM relations WHERE source = ?1 UNION SELECT source FROM relations WHERE target = ?1")?;
    while let Some((id, depth)) = queue.pop_front() {
        if depth == 2 {
            continue;
        }
        let next: Vec<i64> = stmt.query_map(params![id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for n in next {
            if seen.insert(n) {
                queue.push_back((n, depth + 1));
            }
        }
    }
    Ok((seeds, seen))
}

/// The vector leg's weight in the fusion (Kiro Crew's `VECTOR_RRF_WEIGHT`).
const VECTOR_WEIGHT: f64 = 2.0;
/// Text embedded per item, in characters.
const EMBED_CHARS: usize = 6000;

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// A passage's identity for finding copies: its words, whitespace and case aside.
fn passage_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let words = content.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    Sha256::digest(words.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Marks in `meta` that General was made for this file, once. Every file
/// from before General could be deleted has it already (it was made on every
/// open), so a file without the mark gets General once and the mark; one
/// with the mark is left with whatever libraries the person kept, none too.
const GENERAL_MADE: &str = "general_made";

/// On every open: General made the first time only (a new file, or one from
/// before libraries or before General could be deleted), and every document
/// in a library that exists (one whose library went missing goes to General,
/// made again for it if need be). Safe to run on every open.
fn settle_libraries(conn: &Connection) -> anyhow::Result<()> {
    let made = conn.query_row("SELECT 1 FROM meta WHERE key = ?1", params![GENERAL_MADE], |_| Ok(())).optional()?.is_some();
    if !made {
        make_general(conn)?;
        conn.execute("INSERT INTO meta(key, value) VALUES (?1, '1')", params![GENERAL_MADE])?;
    }
    let lost: i64 = conn.query_row("SELECT COUNT(*) FROM sources WHERE library_id NOT IN (SELECT id FROM libraries)", [], |r| r.get(0))?;
    if lost > 0 {
        make_general(conn)?;
        conn.execute("UPDATE sources SET library_id = ?1 WHERE library_id NOT IN (SELECT id FROM libraries)", params![GENERAL])?;
    }
    Ok(())
}

/// General, unless it is there. Its id may meanwhile be another library's
/// (SQLite reuses the highest rowid once it is gone); that one is kept.
fn make_general(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO libraries(id, name, name_key, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![GENERAL, GENERAL_NAME, name_key(GENERAL_NAME), now_ms()],
    )?;
    Ok(())
}

/// Before an upgrade, a copy of the file as it was, beside it
/// (`knowledge.db.v4.bak`; `knowledge.db.v4.<ms>.bak` when that name is taken
/// by an earlier upgrade's copy, which is kept). A failed copy is logged,
/// not fatal: each step of the upgrade is a transaction of its own.
fn backup_before_upgrade(conn: &Connection, path: &Path, from: i64) {
    let named = |suffix: String| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        std::path::PathBuf::from(name)
    };
    let mut backup = named(format!(".v{from}.bak"));
    if backup.exists() {
        backup = named(format!(".v{from}.{}.bak", now_ms()));
    }
    match conn.execute("VACUUM INTO ?1", params![backup.to_string_lossy()]) {
        Ok(_) => tracing::info!(backup = %backup.display(), "kept a copy of the knowledge library before upgrading it"),
        Err(err) => tracing::warn!(%err, "could not copy the knowledge library before upgrading it"),
    }
}

fn require_library(conn: &Connection, id: i64) -> anyhow::Result<()> {
    let found = conn.query_row("SELECT 1 FROM libraries WHERE id = ?1", params![id], |_| Ok(())).optional()?;
    if found.is_none() {
        anyhow::bail!("no library {id}");
    }
    Ok(())
}

fn name_taken(conn: &Connection, name: &str, except: Option<i64>) -> anyhow::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM libraries WHERE name_key = ?1 AND (?2 IS NULL OR id != ?2)",
            params![name_key(name), except],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Items from before passages had hashes get theirs.
fn backfill_hashes(conn: &mut Connection) -> anyhow::Result<()> {
    let missing: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, content FROM items WHERE content_hash = ''")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    if missing.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    for (id, content) in &missing {
        tx.execute("UPDATE items SET content_hash = ?2 WHERE id = ?1", params![id, passage_hash(content)])?;
    }
    tx.commit()?;
    Ok(())
}

fn clear_items(tx: &rusqlite::Transaction<'_>, source: &str) -> anyhow::Result<()> {
    tx.execute("DELETE FROM items_fts WHERE rowid IN (SELECT id FROM items WHERE source_id = ?1)", params![source])?;
    tx.execute("DELETE FROM items WHERE source_id = ?1", params![source])?;
    Ok(())
}

/// Entities no item mentions any more go (with their relations).
fn prune_entities(tx: &rusqlite::Transaction<'_>) -> anyhow::Result<()> {
    tx.execute("DELETE FROM entities WHERE id NOT IN (SELECT entity_id FROM mentions)", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{Entity, Relation};

    fn chunk(i: usize, content: &str) -> Chunk {
        Chunk { content: content.to_string(), section: None, index: i, line_start: 1, line_end: 1 }
    }

    fn entity(name: &str) -> Entity {
        Entity { name: name.into(), kind: "technology".into(), description: String::new() }
    }

    fn file(hash: &str) -> FileState {
        FileState { hash: hash.into(), mtime_ms: 1, size: 1 }
    }

    fn seeded() -> KnowledgeDb {
        let db = KnowledgeDb::in_memory().unwrap();
        db.add_source("ar001", "local_file", "/docs/rust.md").unwrap();
        db.add_source("ar002", "local_file", "/docs/store.md").unwrap();
        let rust = Extraction {
            title: "Ownership in Rust".into(),
            entities: vec![entity("Rust"), entity("Borrow checker")],
            relations: vec![Relation { source: "Rust".into(), target: "Borrow checker".into(), kind: "uses".into(), description: String::new() }],
            category: "code_doc".into(),
            summary: "러스트의 소유권과 빌림 검사기.".into(),
        };
        db.replace_items(
            "ar001",
            &[NewItem { chunk: chunk(0, "Every value has one owner. 소유권 규칙을 설명한다."), extraction: Some(rust), tags: vec![] }],
            true,
            &file("h1"),
        )
        .unwrap();
        let store = Extraction {
            title: "SQLite store".into(),
            entities: vec![entity("SQLite"), entity("Borrow checker")],
            relations: vec![],
            category: "design_doc".into(),
            summary: String::new(),
        };
        db.replace_items(
            "ar002",
            &[
                NewItem { chunk: chunk(0, "The store keeps events in SQLite."), extraction: Some(store), tags: vec![] },
                NewItem { chunk: chunk(1, "## Backups\nnightly copies"), extraction: None, tags: vec![] },
            ],
            false,
            &file("h2"),
        )
        .unwrap();
        db
    }

    #[test]
    fn a_copied_passage_is_found_once() {
        let db = seeded();
        // A second version of store.md: the same passage (spacing aside) and one new one.
        db.add_source("ar003", "local_file", "/docs/store-v2.md").unwrap();
        db.replace_items(
            "ar003",
            &[
                NewItem { chunk: chunk(0, "The store  keeps\nevents in SQLite."), extraction: None, tags: vec![] },
                NewItem { chunk: chunk(1, "## Restore\nfrom the nightly copy"), extraction: None, tags: vec![] },
            ],
            false,
            &file("h3"),
        )
        .unwrap();
        let hits = db.search("events SQLite", 5, None, None).unwrap();
        let copies = hits.iter().filter(|h| h.item.content.contains("keeps")).count();
        assert_eq!(copies, 1, "{:?}", hits.iter().map(|h| &h.item.content).collect::<Vec<_>>());

        let overlaps = db.overlaps(30).unwrap();
        let v2 = overlaps.iter().find(|o| o.source == "ar003").expect("v2 shares with store.md");
        assert_eq!((v2.other.as_str(), v2.shared, v2.percent), ("ar002", 1, 50));
        assert!(!overlaps.iter().any(|o| o.source == "ar001"), "rust.md shares nothing");
    }

    #[test]
    fn stores_and_counts() {
        let db = seeded();
        assert_eq!(db.stats().unwrap(), Stats { sources: 2, items: 3, entities: 3, relations: 1 });
        let items = db.items(Some("ar002"), 100).unwrap();
        assert_eq!(items[1].title, "Backups", "no extraction: the first line");
        assert_eq!(db.source("ar002").unwrap().unwrap().items, 2);
        assert_eq!(db.source_by_uri("/docs/rust.md").unwrap().unwrap().status, status::SYNCED);
        assert_eq!(db.hash_owner("h1", "ar009").unwrap().as_deref(), Some("ar001"));
    }

    #[test]
    fn searches_keywords_in_korean_and_english() {
        let db = seeded();
        let hits = db.search("소유권은", 3, None, None).unwrap();
        assert_eq!(hits[0].item.source_id, "ar001");
        assert_eq!(hits[0].source_uri, "/docs/rust.md");
        let hits = db.search("nightly backups", 3, None, None).unwrap();
        assert_eq!(hits[0].item.title, "Backups");
        assert!(db.search("", 3, None, None).unwrap().is_empty());
    }

    #[test]
    fn graph_finds_what_keywords_miss() {
        let db = seeded();
        // "Rust" appears nowhere in the store doc, but both mention the borrow checker.
        let hits = db.search("Rust", 5, None, None).unwrap();
        let sources: Vec<&str> = hits.iter().map(|h| h.item.source_id.as_str()).collect();
        assert!(sources.contains(&"ar001") && sources.contains(&"ar002"), "{sources:?}");
        assert!(hits.iter().any(|h| h.match_type == "graph"));
        assert_eq!(db.search("Rust", 5, Some("ar002"), None).unwrap().len(), 1);
    }

    #[test]
    fn replacing_and_deleting_prunes() {
        let db = seeded();
        db.replace_items("ar001", &[NewItem { chunk: chunk(0, "gone"), extraction: None, tags: vec![] }], false, &file("h3")).unwrap();
        assert_eq!(db.stats().unwrap().entities, 2, "Rust is no longer mentioned");
        assert!(db.search("owner", 3, None, None).unwrap().is_empty(), "old text left the index");
        db.delete_source("ar002").unwrap();
        assert_eq!(db.stats().unwrap(), Stats { sources: 1, items: 1, entities: 0, relations: 0 });
        let g = db.graph(100).unwrap();
        assert!(g.nodes.is_empty());
    }

    #[test]
    fn vectors_find_by_meaning_in_their_own_space() {
        let db = seeded();
        let todo = db.to_embed("s1", 0, 10).unwrap();
        assert_eq!(todo.len(), 3);
        assert!(todo[0].text.starts_with("Ownership in Rust"), "title, summary, content");
        assert_eq!(db.to_embed("s1", todo[0].id, 10).unwrap().len(), 2, "the cursor skips what came before");
        // Pretend the store doc's backup chunk is about "restore".
        let vecs: Vec<(&ToEmbed, Vec<f32>)> =
            todo.iter().map(|t| (t, if t.text.contains("nightly") { vec![0.0, 1.0] } else { vec![1.0, 0.0] })).collect();
        db.set_embeddings("s1", &vecs).unwrap();
        assert_eq!(db.embedded("s1").unwrap(), (3, 0, 3));
        assert!(db.has_vectors("s1").unwrap());
        assert!(db.to_embed("s1", 0, 10).unwrap().is_empty());
        let other = db.to_embed("s2", 0, 10).unwrap();
        assert_eq!(other.len(), 3, "another space starts over");
        db.embed_failed("s2", &other[0]).unwrap();
        assert_eq!(db.to_embed("s2", 0, 10).unwrap().len(), 2, "a refused item is not retried in that space");
        assert_eq!(db.embedded("s2").unwrap(), (0, 1, 3));
        assert_eq!(db.retry_refused().unwrap(), 1);
        assert_eq!(db.to_embed("s2", 0, 10).unwrap().len(), 3, "a retry brings it back");
        db.embed_failed("s2", &other[0]).unwrap();
        // A vector for an item whose id now names a newer one is dropped.
        let stale = ToEmbed { created_at: other[1].created_at - 1, ..other[1].clone() };
        db.set_embeddings("s2", &[(&stale, vec![1.0, 0.0])]).unwrap();
        assert_eq!(db.embedded("s2").unwrap().0, 0);
        let hits = db.search("restore", 3, None, Some((&[0.1, 0.9], "s1"))).unwrap();
        assert_eq!(hits[0].item.title, "Backups");
        assert_eq!(hits[0].match_type, "vector");
        assert!(db.search("restore", 3, None, Some((&[0.1, 0.9], "s2"))).unwrap().is_empty(), "no vectors in that space");
    }

    /// Take `from` out of `text`, insisting it was there (so a changed schema
    /// fails the test rather than quietly building the wrong old one).
    fn cut(text: &str, from: &str) -> String {
        assert!(text.contains(from), "the schema no longer has {from:?}");
        text.replace(from, "")
    }

    /// The schema as it was before a library had a colour and tags (version 5).
    fn schema_v5() -> String {
        cut(SCHEMA, ",\n    color       TEXT NOT NULL DEFAULT '',\n    tags        TEXT NOT NULL DEFAULT '[]'")
    }

    /// The schema as it was before libraries (version 4).
    fn schema_v4() -> String {
        let s = cut(&schema_v5(), "CREATE TABLE libraries (\n    id          INTEGER PRIMARY KEY,\n    name        TEXT NOT NULL,\n    name_key    TEXT NOT NULL UNIQUE,\n    created_at  INTEGER NOT NULL\n);\n");
        let s = cut(&s, ",\n    library_id     INTEGER NOT NULL DEFAULT 1");
        cut(&s, "CREATE INDEX sources_by_library ON sources(library_id);\n")
    }

    fn temp_db(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("kn-{name}-{}.db", std::process::id()));
        for p in [path.clone(), path.with_extension("db.v4.bak"), path.with_extension("db.v5.bak"), path.with_extension("db.v1.bak")] {
            let _ = std::fs::remove_file(p);
        }
        path
    }

    #[test]
    fn migrates_version_four_into_the_general_library() {
        let path = temp_db("migrate-v4");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&schema_v4()).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO meta VALUES ('schema_version', '4');
                 INSERT INTO sources(id, source_type, uri, status, content_hash, created_at) VALUES ('ar001', 'local_file', '/docs/a.md', 'synced', 'h1', 1);
                 INSERT INTO sources(id, source_type, uri, status, content_hash, created_at) VALUES ('ar002', 'local_file', '/docs/b.md', 'synced', 'h2', 2);
                 INSERT INTO items(id, source_id, chunk_index, title, content, line_start, line_end, created_at) VALUES (1, 'ar001', 0, 'Backups', 'nightly copies of the store', 1, 2, 1);
                 INSERT INTO items_fts(rowid, title, content, tags) VALUES (1, 'Backups', 'nightly copies of the store', '');",
            )
            .unwrap();
        }
        let db = KnowledgeDb::open(&path).unwrap();
        let libs = db.libraries().unwrap();
        assert_eq!(libs.len(), 1, "{libs:?}");
        assert_eq!((libs[0].id, libs[0].name.as_str(), libs[0].sources), (GENERAL, GENERAL_NAME, 2), "every document went to General");
        assert!(db.sources().unwrap().iter().all(|s| s.library_id == GENERAL));
        let hits = db.search("nightly", 3, None, None).unwrap();
        assert_eq!((hits.len(), hits[0].library.as_str()), (1, GENERAL_NAME), "found as before, and from General");
        assert_eq!(db.stats_in(Some(GENERAL)).unwrap(), db.stats().unwrap());
        drop(db);

        // The file as it was, kept beside it before the upgrade.
        let backup = path.with_extension("db.v4.bak");
        {
            let old = Connection::open(&backup).unwrap();
            let v: String = old.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
            let libs: i64 = old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'libraries'", [], |r| r.get(0)).unwrap();
            let docs: i64 = old.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0)).unwrap();
            assert_eq!((v.as_str(), libs, docs), ("4", 0, 2), "the copy is the old file, whole");
        }

        // Opening again changes nothing (and makes no second General).
        let db = KnowledgeDb::open(&path).unwrap();
        assert_eq!(db.libraries().unwrap().len(), 1);
        assert_eq!(db.sources().unwrap().len(), 2);
        drop(db);

        // A file put back at version 4 and upgraded again: its own copy, the first one kept.
        let _ = std::fs::remove_file(&path);
        for ext in ["db-wal", "db-shm"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
        std::fs::copy(&backup, &path).unwrap();
        drop(KnowledgeDb::open(&path).unwrap());
        let dir = path.parent().unwrap();
        let stem = path.file_name().unwrap().to_string_lossy().to_string();
        let copies: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&format!("{stem}.v4."))))
            .collect();
        assert_eq!(copies.len(), 2, "{copies:?}");
        for p in copies.into_iter().chain([path.clone()]) {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn migrates_version_five_libraries_to_colours_and_tags() {
        let path = temp_db("migrate-v5");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&schema_v5()).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO meta VALUES ('schema_version', '5');
                 INSERT INTO libraries(id, name, name_key, created_at) VALUES (1, 'General', 'general', 1), (2, 'Work', 'work', 2);
                 INSERT INTO sources(id, source_type, uri, status, created_at, library_id) VALUES ('ar001', 'local_file', '/docs/a.md', 'synced', 1, 2);",
            )
            .unwrap();
            let cols: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('libraries') WHERE name IN ('color', 'tags')", [], |r| r.get(0)).unwrap();
            assert_eq!(cols, 0, "the old schema really lacks them");
        }
        let db = KnowledgeDb::open(&path).unwrap();
        let libs = db.libraries().unwrap();
        assert_eq!(libs.iter().map(|l| (l.name.as_str(), l.sources, l.color.as_str(), l.tags.len())).collect::<Vec<_>>(), vec![("General", 0, "", 0), ("Work", 1, "", 0)], "kept, with no colour and no tags");
        assert_eq!(db.source("ar001").unwrap().unwrap().library_id, 2, "documents stay where they were");
        let work = db.set_library_look(2, Some("#c0392b"), Some(&["docs".to_string(), "q4".to_string()])).unwrap();
        assert_eq!((work.color.as_str(), work.tags.clone()), ("#c0392b", vec!["docs".to_string(), "q4".to_string()]));
        drop(db);
        let backup = path.with_extension("db.v5.bak");
        {
            let old = Connection::open(&backup).unwrap();
            let v: String = old.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
            assert_eq!(v, "5", "the copy kept before the upgrade");
        }
        // Again: nothing changes, the look stays.
        let db = KnowledgeDb::open(&path).unwrap();
        let work = db.library(2).unwrap().unwrap();
        assert_eq!((work.color.as_str(), work.tags.len()), ("#c0392b", 2));
        drop(db);
        for p in [path.clone(), backup] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn a_library_gets_a_colour_and_tags_and_keeps_them_by_parts() {
        let db = seeded();
        let work = db.create_library("Work").unwrap();
        assert_eq!((work.color.as_str(), work.tags.len()), ("", 0), "none at first");
        let w = db.set_library_look(work.id, Some("#1a73e8"), None).unwrap();
        assert_eq!((w.color.as_str(), w.tags.len()), ("#1a73e8", 0));
        let w = db.set_library_look(work.id, None, Some(&["a".to_string()])).unwrap();
        assert_eq!((w.color.as_str(), w.tags.clone()), ("#1a73e8", vec!["a".to_string()]), "the colour stays when only tags change");
        let g = db.set_library_look(GENERAL, Some(""), Some(&[])).unwrap();
        assert_eq!((g.color.as_str(), g.tags.len()), ("", 0), "General too");
        assert!(db.set_library_look(999, Some("#ffffff"), None).is_err());
        let renamed = db.rename_library(work.id, "Work 2").unwrap();
        assert_eq!(renamed.color, "#1a73e8", "a rename keeps the look");
    }

    #[test]
    fn a_document_whose_library_is_gone_goes_back_to_general() {
        let path = temp_db("orphan");
        {
            let db = KnowledgeDb::open(&path).unwrap();
            db.add_source("ar001", "local_file", "/docs/a.md").unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("UPDATE sources SET library_id = 42", []).unwrap();
        }
        let db = KnowledgeDb::open(&path).unwrap();
        assert_eq!(db.source("ar001").unwrap().unwrap().library_id, GENERAL);
        drop(db);
        {
            // General deleted meanwhile: made again to hold the document.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("DELETE FROM libraries; UPDATE sources SET library_id = 42;").unwrap();
        }
        let db = KnowledgeDb::open(&path).unwrap();
        assert_eq!(db.source("ar001").unwrap().unwrap().library_id, GENERAL);
        assert_eq!(db.libraries().unwrap().len(), 1);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn libraries_are_made_renamed_and_deleted_by_the_rules() {
        let db = seeded();
        let general = db.library(GENERAL).unwrap().unwrap();
        assert_eq!((general.name.as_str(), general.sources), (GENERAL_NAME, 2));
        let work = db.create_library("  Work   notes ").unwrap();
        assert_eq!((work.name.as_str(), work.sources), ("Work notes", 0), "spaces tidied");
        assert!(db.create_library("work NOTES").is_err(), "a name differing only in case is taken");
        assert!(db.create_library("general").is_err(), "General's name too");
        assert!(db.create_library("   ").is_err(), "a name is needed");
        assert!(db.create_library(&"x".repeat(MAX_LIBRARY_NAME + 1)).is_err(), "too long");
        assert_eq!(db.libraries().unwrap().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec![GENERAL_NAME, "Work notes"], "General first");
        assert_eq!(db.library_named("WORK notes").unwrap().map(|l| l.id), Some(work.id));

        let work = db.rename_library(work.id, "Work").unwrap();
        assert_eq!(work.name, "Work");
        assert!(db.rename_library(work.id, "General").is_err(), "not onto another's name");
        assert_eq!(db.rename_library(work.id, "WORK").unwrap().name, "WORK", "its own name in another case is fine");
        assert!(db.rename_library(999, "Elsewhere").is_err());

        // A document goes to one library, and moves.
        db.add_source_in("ar010", "local_file", "/docs/w.md", work.id).unwrap();
        assert!(db.add_source_in("ar011", "local_file", "/docs/x.md", 999).is_err(), "no such library");
        assert!(db.add_source_in("ar012", "local_file", "/docs/w.md", GENERAL).is_err(), "a file is in one library only");
        assert_eq!(db.library(work.id).unwrap().unwrap().sources, 1);

        let refused = db.delete_library(work.id).unwrap_err().to_string();
        assert!(refused.contains("1 documents"), "a library with documents is kept: {refused}");
        assert_eq!(db.move_source("ar010", GENERAL).unwrap().library_id, GENERAL);
        assert!(db.move_source("ar010", 999).is_err());
        assert!(db.move_source("nope", GENERAL).is_err());
        db.delete_library(work.id).unwrap();
        assert!(db.library(work.id).unwrap().is_none());
        assert!(db.delete_library(work.id).is_err(), "gone already");
        assert_eq!(db.source("ar010").unwrap().unwrap().library_id, GENERAL, "the moved document stayed");

        // General is refused only for its documents, in the same words as any library.
        let refused = db.delete_library(GENERAL).unwrap_err().to_string();
        assert!(refused.contains("3 documents") && refused.contains("move or remove them first"), "{refused}");
    }

    #[test]
    fn general_is_deleted_like_any_library_and_not_made_again_on_open() {
        let path = temp_db("general-gone");
        {
            let db = KnowledgeDb::open(&path).unwrap();
            assert_eq!(db.libraries().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![GENERAL], "a new file has General");
            let work = db.create_library("Work").unwrap();
            db.delete_library(GENERAL).unwrap();
            assert_eq!(db.libraries().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![work.id]);
            // With no library named, a document goes to the first one there is.
            assert_eq!(db.add_source("ar001", "local_file", "/docs/a.md").unwrap().library_id, work.id);
        }
        {
            let db = KnowledgeDb::open(&path).unwrap();
            assert_eq!(db.libraries().unwrap().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Work"], "not made again on open");
            db.delete_source("ar001").unwrap();
            let work = db.libraries().unwrap()[0].id;
            db.delete_library(work).unwrap();
            assert!(db.libraries().unwrap().is_empty(), "the last one goes too");
        }
        {
            let db = KnowledgeDb::open(&path).unwrap();
            assert!(db.libraries().unwrap().is_empty(), "none, and none made on open");
            // A document with nowhere to go gets General back.
            let doc = db.add_source("ar002", "local_file", "/docs/b.md").unwrap();
            assert_eq!(doc.library_id, GENERAL);
            assert_eq!(db.libraries().unwrap().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec![GENERAL_NAME]);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_from_before_general_could_be_deleted_keeps_it_once() {
        let path = temp_db("general-mark");
        {
            let db = KnowledgeDb::open(&path).unwrap();
            db.create_library("Work").unwrap();
        }
        {
            // As the last build left it: General there, no mark.
            let conn = Connection::open(&path).unwrap();
            conn.execute("DELETE FROM meta WHERE key = ?1", params![GENERAL_MADE]).unwrap();
        }
        {
            let db = KnowledgeDb::open(&path).unwrap();
            assert_eq!(db.libraries().unwrap().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec![GENERAL_NAME, "Work"], "nothing lost, nothing doubled");
            db.delete_library(GENERAL).unwrap();
        }
        let db = KnowledgeDb::open(&path).unwrap();
        assert_eq!(db.libraries().unwrap().iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Work"], "marked: General stays deleted");
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reads_narrow_to_a_library_and_search_sees_them_all() {
        let db = seeded();
        let store = db.create_library("Store").unwrap().id;
        db.move_source("ar002", store).unwrap();
        let names = |g: &Graph| {
            let mut n: Vec<String> = g.nodes.iter().map(|n| format!("{}:{}", n.name, n.mentions)).collect();
            n.sort();
            n
        };
        assert_eq!(names(&db.graph_in(Some(GENERAL), 100).unwrap()), vec!["Borrow checker:1", "Rust:1"]);
        assert_eq!(names(&db.graph_in(Some(store), 100).unwrap()), vec!["Borrow checker:1", "SQLite:1"], "the shared entity, counted in its library");
        assert_eq!(names(&db.graph_in(None, 100).unwrap()), vec!["Borrow checker:2", "Rust:1", "SQLite:1"]);
        assert_eq!(db.graph_in(Some(GENERAL), 100).unwrap().edges.len(), 1);
        assert!(db.graph_in(Some(store), 100).unwrap().edges.is_empty(), "the relation is stated in rust.md only");
        assert_eq!(db.graph(100).unwrap(), db.graph_in(None, 100).unwrap());

        assert_eq!(db.stats_in(Some(store)).unwrap(), Stats { sources: 1, items: 2, entities: 2, relations: 0 });
        assert_eq!(db.stats_in(Some(GENERAL)).unwrap(), Stats { sources: 1, items: 1, entities: 2, relations: 1 });
        assert_eq!(db.stats_in(None).unwrap(), db.stats().unwrap());
        assert_eq!(db.items_in(None, Some(store), 100).unwrap().len(), 2);
        assert_eq!(db.items_in(None, Some(GENERAL), 100).unwrap().len(), 1);
        assert!(db.items_in(Some("ar001"), Some(store), 100).unwrap().is_empty(), "a document outside the library");

        let bc = db.graph(100).unwrap().nodes.iter().find(|n| n.name == "Borrow checker").unwrap().id;
        assert_eq!(db.entity_items_in(bc, Some(store)).unwrap().len(), 1);
        assert_eq!(db.entity_items_in(bc, None).unwrap().len(), 2);

        // Search: every library unless narrowed, each hit naming its library.
        let all = db.search("Rust", 5, None, None).unwrap();
        let mut libs: Vec<&str> = all.iter().map(|h| h.library.as_str()).collect();
        libs.sort();
        assert_eq!(libs, vec![GENERAL_NAME, "Store"], "{all:?}");
        let narrowed = db.search_in("Rust", 5, None, Some(store), None).unwrap();
        assert!(narrowed.iter().all(|h| h.item.source_id == "ar002"), "graph leg, in Store only");
        assert!(db.search_in("소유권은", 5, None, Some(store), None).unwrap().is_empty(), "keyword leg too");
    }

    #[test]
    fn copies_are_told_within_a_library() {
        let db = seeded();
        let other = db.create_library("Other").unwrap().id;
        db.add_source_in("ar009", "local_file", "/docs/rust-copy.md", other).unwrap();
        assert_eq!(db.hash_owner("h1", "ar009").unwrap(), None, "the same text in another library is its own document");
        db.move_source("ar009", GENERAL).unwrap();
        assert_eq!(db.hash_owner("h1", "ar009").unwrap().as_deref(), Some("ar001"), "and a duplicate in the same one");

        // A second version of store.md in another library: not a copy to tidy there.
        db.add_source_in("ar003", "local_file", "/docs/store-v2.md", other).unwrap();
        db.replace_items("ar003", &[NewItem { chunk: chunk(0, "The store keeps events in SQLite."), extraction: None, tags: vec![] }], false, &file("h3")).unwrap();
        assert!(db.overlaps(30).unwrap().is_empty());
        db.move_source("ar003", GENERAL).unwrap();
        assert_eq!(db.overlaps(30).unwrap().iter().filter(|o| o.source == "ar003").count(), 1);
    }

    #[test]
    fn migrates_version_one() {
        let path = temp_db("migrate-v1");
        {
            let conn = Connection::open(&path).unwrap();
            let v1 = schema_v4()
                .replace(",\n    embedding    BLOB,\n    embedding_sig TEXT NOT NULL DEFAULT '',\n    content_hash TEXT NOT NULL DEFAULT ''", "")
                .replace("CREATE INDEX items_by_sig ON items(embedding_sig);\n", "")
                .replace("CREATE INDEX items_by_hash ON items(content_hash);\n", "");
            conn.execute_batch(&v1).unwrap();
            conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO meta VALUES ('schema_version', '1');").unwrap();
            let cols: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('items') WHERE name = 'embedding'", [], |r| r.get(0)).unwrap();
            assert_eq!(cols, 0, "the old schema really lacks the column");
        }
        let db = KnowledgeDb::open(&path).unwrap();
        assert_eq!(db.embedded("x").unwrap(), (0, 0, 0));
        assert_eq!(db.libraries().unwrap().len(), 1, "on to libraries too");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db.v1.bak"));
    }

    #[test]
    fn a_selection_reads_as_entities_relations_and_passages() {
        let db = seeded();
        let g = db.graph(100).unwrap();
        let id = |name: &str| g.nodes.iter().find(|n| n.name == name).unwrap().id;
        let (rust, bc, sqlite) = (id("Rust"), id("Borrow checker"), id("SQLite"));
        let item = db.entity_items(rust).unwrap()[0].id;
        let text = db.selection_context(&[rust, bc, rust, 999_999], &[item, item], 20).unwrap();
        assert!(text.starts_with("[selection from the knowledge graph]"), "{text}");
        assert!(text.contains("entities (2):"), "repeats and missing ids are dropped: {text}");
        assert!(text.contains("- Rust (technology, mentioned 1x)"), "{text}");
        assert!(text.contains("relations (1):\n- Rust -uses-> Borrow checker"), "{text}");
        assert!(text.contains("passages (1):\n### Ownership in Rust — rust.md · lines 1-1"), "the document by its file name: {text}");
        assert!(text.contains("[...]"), "a long passage is cut: {text}");
        let alone = db.selection_context(&[sqlite], &[], 500).unwrap();
        assert!(!alone.contains("relations"), "no relation to show among one entity: {alone}");
        assert_eq!(db.selection_context(&[], &[], 500).unwrap(), "");
        assert_eq!(db.selection_context(&[999_999], &[888_888], 500).unwrap(), "", "nothing left is nothing");
    }

    #[test]
    fn graph_lists_nodes_and_edges() {
        let db = seeded();
        let g = db.graph(100).unwrap();
        assert_eq!(g.nodes[0].name, "Borrow checker");
        assert_eq!(g.nodes[0].mentions, 2);
        assert_eq!(g.edges.len(), 1);
        let bc = g.nodes[0].id;
        assert_eq!(db.entity_items(bc).unwrap().len(), 2);
    }

    #[test]
    fn a_query_reaches_the_entities_search_follows() {
        let db = seeded();
        let g = db.graph(100).unwrap();
        let id = |name: &str| g.nodes.iter().find(|n| n.name == name).unwrap().id;
        let q = db.query_entities("Rust").unwrap();
        assert_eq!(q.seeds, vec![id("Rust")]);
        let mut want = vec![id("Rust"), id("Borrow checker")];
        want.sort_unstable();
        assert_eq!(q.related, want, "a relation away; SQLite shares no relation");
        assert_eq!(db.query_entities("nightly backups").unwrap(), QueryEntities::default(), "names no entity");
        assert_eq!(db.query_entities("").unwrap(), QueryEntities::default());
    }
}
