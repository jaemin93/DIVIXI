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

const SCHEMA_VERSION: i64 = 4;

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
];

const SCHEMA: &str = r#"
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
    created_at     INTEGER NOT NULL
);
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
    extracted, done, total, (SELECT COUNT(*) FROM items i WHERE i.source_id = s.id), created_at FROM sources s";

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
    })
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
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> anyhow::Result<Self> {
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
                for step in v..SCHEMA_VERSION {
                    let tx = conn.transaction()?;
                    tx.execute_batch(MIGRATIONS[(step - 1) as usize])?;
                    tx.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", params![(step + 1).to_string()])?;
                    tx.commit()?;
                }
            }
            Some(v) => anyhow::bail!("knowledge schema version {v} is not supported by this build ({SCHEMA_VERSION})"),
        }
        backfill_hashes(&mut conn)?;
        // A sync cut short by a quit resumes from the start next time.
        conn.execute(
            "UPDATE sources SET status = ?1 WHERE status = ?2",
            params![status::PENDING, status::INDEXING],
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Register a document. Errors when the id or the path is taken.
    pub fn add_source(&self, id: &str, source_type: &str, uri: &str) -> anyhow::Result<Source> {
        self.conn.lock().execute(
            "INSERT INTO sources(id, source_type, uri, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, source_type, uri, status::PENDING, now_ms()],
        )?;
        self.source(id)?.ok_or_else(|| anyhow::anyhow!("source {id} vanished"))
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

    /// Another source already holding exactly this text.
    pub fn hash_owner(&self, hash: &str, except: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id FROM sources WHERE content_hash = ?1 AND id != ?2 AND status = ?3 LIMIT 1",
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
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{ITEM_SELECT_BY_SOURCE} WHERE ?1 IS NULL OR i.source_id = ?1 ORDER BY s.created_at DESC, i.source_id, i.chunk_index LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![source, limit as i64], row_to_item)?;
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
        let mut stmt = conn.prepare(
            "SELECT a.source_id, b.source_id, COUNT(DISTINCT a.content_hash)
             FROM items a JOIN items b ON a.content_hash = b.content_hash AND a.source_id != b.source_id
             WHERE a.content_hash != ''
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

    pub fn item(&self, id: i64) -> anyhow::Result<Option<Item>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{ITEM_SELECT} WHERE id = ?1"), params![id], row_to_item).optional()?)
    }

    pub fn stats(&self) -> anyhow::Result<Stats> {
        let conn = self.conn.lock();
        let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(Stats {
            sources: count("SELECT COUNT(*) FROM sources")?,
            items: count("SELECT COUNT(*) FROM items")?,
            entities: count("SELECT COUNT(*) FROM entities")?,
            relations: count("SELECT COUNT(*) FROM relations")?,
        })
    }

    /// The entity graph, the most mentioned entities first, up to `limit`.
    pub fn graph(&self, limit: usize) -> anyhow::Result<Graph> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT e.id, e.name, e.kind, e.description, (SELECT COUNT(*) FROM mentions m WHERE m.entity_id = e.id) AS n
             FROM entities e ORDER BY n DESC, e.id LIMIT ?1",
        )?;
        let nodes: Vec<GraphNode> = stmt
            .query_map(params![limit as i64], |r| {
                Ok(GraphNode { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, description: r.get(3)?, mentions: r.get(4)? })
            })?
            .collect::<Result<_, _>>()?;
        let shown: HashSet<i64> = nodes.iter().map(|n| n.id).collect();
        let mut stmt = conn.prepare("SELECT DISTINCT source, target, kind FROM relations")?;
        let edges = stmt
            .query_map([], |r| Ok(GraphEdge { source: r.get(0)?, target: r.get(1)?, kind: r.get(2)? }))?
            .filter_map(Result::ok)
            .filter(|e| shown.contains(&e.source) && shown.contains(&e.target))
            .collect();
        Ok(Graph { nodes, edges })
    }

    /// Items that mention an entity.
    pub fn entity_items(&self, entity: i64) -> anyhow::Result<Vec<Item>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{ITEM_SELECT} WHERE id IN (SELECT item_id FROM mentions WHERE entity_id = ?1) ORDER BY source_id, chunk_index"
        ))?;
        let rows = stmt.query_map(params![entity], row_to_item)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keyword, graph and (given the query's vector and the space it is
    /// in) vector search, fused by reciprocal rank with the vector leg
    /// counting double, as Kiro Crew weighs it. The keyword search's best
    /// match is always kept, so up to `limit + 1` come back. Results under
    /// [`MIN_SCORE`] are dropped.
    pub fn search(&self, query: &str, limit: usize, source: Option<&str>, vector: Option<(&[f32], &str)>) -> anyhow::Result<Vec<Hit>> {
        let limit = limit.max(1);
        let keyword = self.keyword_search(query, 20, source)?;
        let graph = self.graph_search(query, 20, source)?;
        let semantic = match vector {
            Some((v, sig)) => self.vector_search(v, sig, 20, source)?,
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
            let source_uri = self.source(&item.source_id)?.map(|s| s.uri).unwrap_or_default();
            hits.push(Hit { item, score, match_type, source_uri });
        }
        Ok(hits)
    }

    /// Items closest to `query` by cosine similarity, among those embedded
    /// in the space `sig` names. A brute-force scan: fine for the tens of
    /// thousands of items a personal library holds.
    fn vector_search(&self, query: &[f32], sig: &str, limit: usize, source: Option<&str>) -> anyhow::Result<Vec<i64>> {
        let qn = norm(query);
        if qn == 0.0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, embedding FROM items WHERE embedding IS NOT NULL AND embedding_sig = ?1 AND (?2 IS NULL OR source_id = ?2)",
        )?;
        let mut scored: Vec<(i64, f32)> = stmt
            .query_map(params![sig, source], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
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

    fn keyword_search(&self, query: &str, limit: usize, source: Option<&str>) -> anyhow::Result<Vec<i64>> {
        let Some(expr) = fts::match_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT i.id FROM items_fts f JOIN items i ON i.id = f.rowid
             WHERE items_fts MATCH ?1 AND (?2 IS NULL OR i.source_id = ?2) ORDER BY f.rank LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![expr, source, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Entities named in the query, their neighbours two hops out, and the
    /// items that mention most of them.
    fn graph_search(&self, query: &str, limit: usize, source: Option<&str>) -> anyhow::Result<Vec<i64>> {
        let conn = self.conn.lock();
        let mut seeds: HashSet<i64> = HashSet::new();
        for term in fts::entity_candidates(query) {
            if let Some(id) = conn
                .query_row("SELECT id FROM entities WHERE name_key = ?1", params![term.to_lowercase()], |r| r.get::<_, i64>(0))
                .optional()?
            {
                seeds.insert(id);
            }
        }
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen: HashSet<i64> = seeds.clone();
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
        let list = seen.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT m.item_id FROM mentions m JOIN items i ON i.id = m.item_id
             WHERE m.entity_id IN ({list}) AND (?1 IS NULL OR i.source_id = ?1)
             GROUP BY m.item_id ORDER BY COUNT(*) DESC, m.item_id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![source, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
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

    #[test]
    fn migrates_version_one() {
        let path = std::env::temp_dir().join(format!("kn-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let conn = Connection::open(&path).unwrap();
            let v1 = SCHEMA
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
        drop(db);
        let _ = std::fs::remove_file(&path);
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
}
