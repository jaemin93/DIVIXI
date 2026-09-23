//! Event store: the durable side of the membrane.
//!
//! Every [`LaneEvent`] a run emits is appended, in order, to an append-only
//! `events` log. The `runs` table is a projection folded from that log at the
//! run's lifecycle boundaries (`started`, `finished`, `failed`), and
//! `runs_fts` is a full-text index over each finished run's prompt, output and
//! tool titles.
//!
//! The split matters for the membrane: the Track timeline is rebuilt from
//! `runs` alone (one row per run, above the membrane), while the inspector
//! replays `events` for a single run on demand. Nothing below the membrane
//! has to be loaded to draw the timeline.
//!
//! Storage is SQLite (bundled, WAL). One file per app; runs carry their own
//! working directory.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use orchestra_core::{LaneEvent, LaneId, RunId, RunStatus};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Bump when `SCHEMA` changes in a way that needs a migration, and add the
/// step to [`migrate`].
const SCHEMA_VERSION: i64 = 5;

/// Migration steps, applied in order from the stored version to
/// [`SCHEMA_VERSION`]. Step `i` upgrades from version `i + 1` to `i + 2`.
const MIGRATIONS: &[&str] = &[
    // 1 -> 2: runs record which agent ran them.
    "ALTER TABLE runs ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude_code';",
    // 2 -> 3: tracks. Every run belongs to one; what was there before
    // becomes the first track, and its conductor and lane sessions keep
    // their memory under the track-scoped keys.
    "CREATE TABLE IF NOT EXISTS tracks (
        id         TEXT    PRIMARY KEY,
        name       TEXT    NOT NULL,
        intent     TEXT    NOT NULL DEFAULT '',
        cwd        TEXT    NOT NULL,
        agent      TEXT    NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    ALTER TABLE runs ADD COLUMN track TEXT NOT NULL DEFAULT '';
    CREATE INDEX IF NOT EXISTS runs_by_track ON runs(track, n);
    INSERT INTO tracks(id, name, intent, cwd, agent, created_at, updated_at)
        SELECT 'tr001', 'Track 1', '',
               (SELECT cwd FROM runs ORDER BY n LIMIT 1),
               COALESCE((SELECT agent FROM runs WHERE lane = 'conductor' ORDER BY n DESC LIMIT 1), 'claude_code'),
               (SELECT MIN(started_at) FROM runs),
               (SELECT MAX(started_at) FROM runs)
        WHERE EXISTS (SELECT 1 FROM runs);
    UPDATE runs SET track = 'tr001';
    UPDATE meta SET key = 'conductor_session:tr001:' || substr(key, 19) WHERE key LIKE 'conductor_session:%';
    UPDATE meta SET key = 'lane_session:tr001/' || substr(key, 14) WHERE key LIKE 'lane_session:%';",
    // 3 -> 4: per-track session options for the conductor and for lanes
    // (agent, and a map of the agent's own select options: mode, model,
    // effort…). The global per-agent model choice becomes the first
    // track's conductor model.
    "ALTER TABLE tracks ADD COLUMN conductor_config TEXT NOT NULL DEFAULT '{}';
    ALTER TABLE tracks ADD COLUMN worker_agent TEXT NOT NULL DEFAULT '';
    ALTER TABLE tracks ADD COLUMN worker_config TEXT NOT NULL DEFAULT '{}';
    UPDATE tracks SET conductor_config = json_object('model',
        json_extract((SELECT value FROM meta WHERE key = 'setting:models'), '$.' || agent))
      WHERE json_valid((SELECT value FROM meta WHERE key = 'setting:models'))
        AND json_extract((SELECT value FROM meta WHERE key = 'setting:models'), '$.' || agent) IS NOT NULL;
    DELETE FROM meta WHERE key = 'setting:models';",
    // 4 -> 5: a colour and tags per track, for the track list.
    "ALTER TABLE tracks ADD COLUMN color TEXT NOT NULL DEFAULT '';
    ALTER TABLE tracks ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';",
];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tracks (
    id               TEXT    PRIMARY KEY,
    name             TEXT    NOT NULL,
    intent           TEXT    NOT NULL DEFAULT '',
    cwd              TEXT    NOT NULL,
    agent            TEXT    NOT NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    conductor_config TEXT    NOT NULL DEFAULT '{}',
    worker_agent     TEXT    NOT NULL DEFAULT '',
    worker_config    TEXT    NOT NULL DEFAULT '{}',
    color            TEXT    NOT NULL DEFAULT '',
    tags             TEXT    NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS runs (
    n           INTEGER PRIMARY KEY AUTOINCREMENT,
    id          TEXT    NOT NULL UNIQUE,
    track       TEXT    NOT NULL DEFAULT '',
    lane        TEXT    NOT NULL,
    prompt      TEXT    NOT NULL,
    cwd         TEXT    NOT NULL,
    status      TEXT    NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    duration_ms INTEGER,
    session_id  TEXT,
    stop_reason TEXT,
    error       TEXT,
    output      TEXT    NOT NULL DEFAULT '',
    plan        TEXT    NOT NULL DEFAULT '[]',
    tool_count  INTEGER NOT NULL DEFAULT 0,
    agent       TEXT    NOT NULL DEFAULT 'claude_code'
);

CREATE INDEX IF NOT EXISTS runs_by_track ON runs(track, n);

CREATE TABLE IF NOT EXISTS events (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id  TEXT    NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    at_ms   INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    above   INTEGER NOT NULL,
    payload TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS events_by_run ON events(run_id, seq);

CREATE VIRTUAL TABLE IF NOT EXISTS runs_fts USING fts5(
    run_id UNINDEXED,
    prompt,
    output,
    tools
);
"#;

/// Error text recorded on runs that were live when the app last closed.
pub const INTERRUPTED: &str = "interrupted: the app closed while the run was live";

/// One run, as the Track timeline sees it.
///
/// Everything here is above the membrane or a fold of what is below it
/// (`output`, `plan`, `tool_count`). Transcript detail is not included; use
/// [`Store::events`] for that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: RunId,
    /// The track this run belongs to.
    pub track: String,
    pub lane: LaneId,
    /// Which agent ran it (`claude_code`, `codex`, …).
    pub agent: String,
    pub prompt: String,
    pub cwd: String,
    pub status: RunStatus,
    /// Unix milliseconds.
    pub started_at: i64,
    /// Set once the run ended.
    pub duration_ms: Option<u64>,
    pub session_id: Option<String>,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    /// The agent's visible message text, concatenated.
    pub output: String,
    /// The last plan the agent published.
    pub plan: Vec<String>,
    pub tool_count: u32,
}

/// A track: one conductor, its lanes, one working directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: String,
    pub name: String,
    /// One line on what the track is for; shown under the name.
    pub intent: String,
    /// Directory the conductor and its lanes work in.
    pub cwd: String,
    /// Agent the conductor runs on.
    pub agent: String,
    /// The conductor's session options, `option id → value id`, in the
    /// agent's own terms (`mode`, `model`, `reasoning_effort`, …). Absent
    /// options keep the agent's default; an absent `mode` means the most
    /// autonomous one.
    pub conductor_config: BTreeMap<String, String>,
    /// Agent lanes run on; empty means the conductor's.
    pub worker_agent: String,
    /// Lanes' session options, like `conductor_config`.
    pub worker_config: BTreeMap<String, String>,
    /// A colour for the list, `#rrggbb`; empty means none.
    pub color: String,
    /// Free-form labels for the list and its search.
    pub tags: Vec<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Unix milliseconds of the last run started in it, or its creation.
    pub updated_at: i64,
    pub runs: u32,
}

impl TrackInfo {
    /// The agent lanes run on.
    pub fn lane_agent(&self) -> &str {
        if self.worker_agent.is_empty() {
            &self.agent
        } else {
            &self.worker_agent
        }
    }
}

/// Fields of a track a caller may set; `None` keeps what is there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackPatch {
    pub name: Option<String>,
    pub intent: Option<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    pub conductor_config: Option<BTreeMap<String, String>>,
    pub worker_agent: Option<String>,
    pub worker_config: Option<BTreeMap<String, String>>,
    pub color: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// A lane as the record knows it: its runs, whoever ran them last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneInfo {
    pub name: String,
    /// Agent of the latest run.
    pub agent: String,
    pub runs: u32,
    pub last_run: String,
    pub last_status: RunStatus,
    /// Unix milliseconds of the latest run's start.
    pub last_at: i64,
}

/// One event from the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub seq: i64,
    pub at_ms: u64,
    pub event: LaneEvent,
}

/// A full-text match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub run: RunId,
    /// Matched fragment of the output, with `[` `]` around the terms.
    pub snippet: String,
}

/// The store. Cheap to share behind an `Arc`; all access is serialized.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open or create the database at `path`.
    ///
    /// Runs left live by a previous process are closed out as failed with
    /// [`INTERRUPTED`], so the timeline never shows a run that is running in
    /// a process that no longer exists.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// An in-memory store, for tests and `ORCHESTRA_DB=:memory:`.
    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // A fresh file gets the current schema; an existing one is migrated
        // step by step. The `meta` table is created first so the version can
        // be read before anything else is touched.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        let version: Option<i64> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|v| v.parse().ok());
        match version {
            None => {
                conn.execute_batch(SCHEMA)?;
                conn.execute(
                    "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) if v < SCHEMA_VERSION => migrate(&conn, v)?,
            Some(v) => anyhow::bail!("store schema version {v} is newer than this build supports ({SCHEMA_VERSION})"),
        }

        let store = Self { conn: Mutex::new(conn) };
        store.close_interrupted_runs()?;
        Ok(store)
    }

    /// Read a free-form setting.
    pub fn get_meta(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    /// Write a free-form setting.
    pub fn set_meta(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn close_interrupted_runs(&self) -> anyhow::Result<()> {
        let live: Vec<(RunId, u64)> = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT r.id, COALESCE((SELECT MAX(at_ms) FROM events e WHERE e.run_id = r.id), 0)
                 FROM runs r WHERE r.status IN ('connecting', 'running') ORDER BY r.n",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (run, at_ms) in live {
            tracing::warn!(%run, "closing run left live by a previous process");
            self.append(&run, at_ms, &LaneEvent::Failed { error: INTERRUPTED.to_string() })?;
        }
        Ok(())
    }

    /// Create a track. Ids are `tr001`, `tr002`, … in creation order. The
    /// patch's `name`, `cwd` and `agent` are required; the rest defaults.
    pub fn create_track(&self, patch: &TrackPatch) -> anyhow::Result<TrackInfo> {
        let name = patch.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let (Some(name), Some(cwd), Some(agent)) = (name, patch.cwd.as_deref(), patch.agent.as_deref()) else {
            anyhow::bail!("a track needs a name, a working directory and an agent");
        };
        let id = {
            let conn = self.conn.lock();
            let next: i64 = conn.query_row(
                "SELECT COALESCE(MAX(CAST(substr(id, 3) AS INTEGER)), 0) + 1 FROM tracks",
                [],
                |r| r.get(0),
            )?;
            let id = format!("tr{next:03}");
            let now = now_ms();
            conn.execute(
                "INSERT INTO tracks(id, name, intent, cwd, agent, created_at, updated_at,
                                    conductor_config, worker_agent, worker_config, color, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    name,
                    patch.intent.as_deref().unwrap_or("").trim(),
                    cwd,
                    agent,
                    now,
                    config_json(patch.conductor_config.as_ref())?,
                    patch.worker_agent.as_deref().unwrap_or(""),
                    config_json(patch.worker_config.as_ref())?,
                    patch.color.as_deref().unwrap_or(""),
                    tags_json(patch.tags.as_ref())?,
                ],
            )?;
            id
        };
        self.track(&id)?.ok_or_else(|| anyhow::anyhow!("track {id} vanished after insert"))
    }

    /// Every track, oldest first, with its run count.
    pub fn tracks(&self) -> anyhow::Result<Vec<TrackInfo>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{TRACK_SELECT} ORDER BY t.created_at, t.id"))?;
        let rows = stmt.query_map([], row_to_track)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One track by id.
    pub fn track(&self, id: &str) -> anyhow::Result<Option<TrackInfo>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("{TRACK_SELECT} WHERE t.id = ?1"), params![id], row_to_track)
            .optional()?)
    }

    /// Change a track's fields; `None` in the patch keeps a field.
    pub fn update_track(&self, id: &str, patch: &TrackPatch) -> anyhow::Result<TrackInfo> {
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE tracks SET name = COALESCE(?2, name), intent = COALESCE(?3, intent),
                        cwd = COALESCE(?4, cwd), agent = COALESCE(?5, agent),
                        conductor_config = COALESCE(?6, conductor_config),
                        worker_agent = COALESCE(?7, worker_agent),
                        worker_config = COALESCE(?8, worker_config),
                        color = COALESCE(?9, color), tags = COALESCE(?10, tags)
                 WHERE id = ?1",
                params![
                    id,
                    patch.name.as_deref().map(str::trim),
                    patch.intent.as_deref().map(str::trim),
                    patch.cwd.as_deref(),
                    patch.agent.as_deref(),
                    patch.conductor_config.as_ref().map(|c| config_json(Some(c))).transpose()?,
                    patch.worker_agent.as_deref(),
                    patch.worker_config.as_ref().map(|c| config_json(Some(c))).transpose()?,
                    patch.color.as_deref(),
                    patch.tags.as_ref().map(|t| tags_json(Some(t))).transpose()?,
                ],
            )?;
            if changed == 0 {
                anyhow::bail!("no track {id}");
            }
        }
        self.track(id)?.ok_or_else(|| anyhow::anyhow!("no track {id}"))
    }

    /// Forget a track's remembered conductor and lane sessions, so they
    /// open fresh next time (needed when the working directory changes).
    pub fn forget_track_sessions(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'lane_session:' || ?1 || '/%'",
            params![id],
        )?;
        Ok(())
    }

    /// Delete a track with every run, event and remembered session in it.
    pub fn delete_track(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM runs_fts WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![id])?;
        tx.execute("DELETE FROM events WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![id])?;
        tx.execute("DELETE FROM runs WHERE track = ?1", params![id])?;
        tx.execute(
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'lane_session:' || ?1 || '/%'",
            params![id],
        )?;
        let changed = tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
        if changed == 0 {
            anyhow::bail!("no track {id}");
        }
        tx.commit()?;
        Ok(())
    }

    /// Register a new run in a track and return its id.
    ///
    /// Ids are `t001`, `t002`, … in creation order, durable across restarts.
    pub fn begin_run(&self, track: &str, lane: &str, agent: &str, prompt: &str, cwd: &str) -> anyhow::Result<RunId> {
        let conn = self.conn.lock();
        let next: i64 = conn.query_row("SELECT COALESCE(MAX(n), 0) + 1 FROM runs", [], |r| r.get(0))?;
        let id = format!("t{next:03}");
        let now = now_ms();
        conn.execute(
            "INSERT INTO runs(n, id, track, lane, agent, prompt, cwd, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![next, id, track, lane, agent, prompt, cwd, RunStatus::Connecting.as_str(), now],
        )?;
        conn.execute("UPDATE tracks SET updated_at = ?2 WHERE id = ?1", params![track, now])?;
        Ok(id)
    }

    /// Append one event to a run's log and fold it into the projection.
    ///
    /// Returns the event's sequence number. Appending to a run that already
    /// ended is allowed (the log stays truthful) but does not reopen it.
    pub fn append(&self, run: &str, at_ms: u64, event: &LaneEvent) -> anyhow::Result<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let payload = serde_json::to_string(event)?;
        tx.execute(
            "INSERT INTO events(run_id, at_ms, kind, above, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run, at_ms as i64, event.kind(), event.above_membrane() as i64, payload],
        )?;
        let seq = tx.last_insert_rowid();

        match event {
            LaneEvent::Started { session_id, .. } => {
                tx.execute(
                    "UPDATE runs SET status = ?2, session_id = ?3 WHERE id = ?1 AND status = 'connecting'",
                    params![run, RunStatus::Running.as_str(), session_id],
                )?;
            }
            LaneEvent::Finished { stop_reason } => {
                let changed = tx.execute(
                    "UPDATE runs SET status = ?2, stop_reason = ?3, ended_at = ?4, duration_ms = ?5
                     WHERE id = ?1 AND status IN ('connecting', 'running')",
                    params![run, RunStatus::Done.as_str(), stop_reason, now_ms(), at_ms as i64],
                )?;
                if changed > 0 {
                    fold(&tx, run)?;
                }
            }
            LaneEvent::Failed { error } => {
                let changed = tx.execute(
                    "UPDATE runs SET status = ?2, error = ?3, ended_at = ?4, duration_ms = ?5
                     WHERE id = ?1 AND status IN ('connecting', 'running')",
                    params![run, RunStatus::Failed.as_str(), error, now_ms(), at_ms as i64],
                )?;
                if changed > 0 {
                    fold(&tx, run)?;
                }
            }
            _ => {}
        }

        tx.commit()?;
        Ok(seq)
    }

    /// Every run, oldest first. This is what rebuilds the Track timeline.
    pub fn runs(&self) -> anyhow::Result<Vec<RunSummary>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{RUN_SELECT} ORDER BY n"))?;
        let rows = stmt.query_map([], row_to_summary)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every lane that ever had a run in a track, newest activity first,
    /// with the agent and status of its latest run. Lanes outlive their
    /// sessions: this is how a closed lane is still known.
    pub fn lanes(&self, track: &str) -> anyhow::Result<Vec<LaneInfo>> {
        let conn = self.conn.lock();
        // The latest run per lane, joined back for its id, status and time.
        let mut stmt = conn.prepare(
            "SELECT r.lane, r.agent, c.runs, r.id, r.status, r.started_at
             FROM runs r
             JOIN (SELECT lane, COUNT(*) AS runs, MAX(n) AS last_n FROM runs WHERE track = ?1 GROUP BY lane) c
               ON c.lane = r.lane AND c.last_n = r.n
             ORDER BY r.n DESC",
        )?;
        let rows = stmt.query_map(params![track], |r| {
            let status: String = r.get(4)?;
            Ok(LaneInfo {
                name: r.get(0)?,
                agent: r.get(1)?,
                runs: r.get::<_, i64>(2)? as u32,
                last_run: r.get(3)?,
                last_status: RunStatus::parse(&status).unwrap_or(RunStatus::Failed),
                last_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One run's summary, if it exists.
    pub fn run(&self, id: &str) -> anyhow::Result<Option<RunSummary>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("{RUN_SELECT} WHERE id = ?1"), params![id], row_to_summary)
            .optional()?)
    }

    /// One run's full event log, in order. This is what the inspector replays.
    pub fn events(&self, run: &str) -> anyhow::Result<Vec<StoredEvent>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT seq, at_ms, payload FROM events WHERE run_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![run], |r| {
            let payload: String = r.get(2)?;
            let event: LaneEvent = serde_json::from_str(&payload).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
            })?;
            Ok(StoredEvent { seq: r.get(0)?, at_ms: r.get::<_, i64>(1)? as u64, event })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Full-text search over finished runs' prompt, output and tool titles.
    ///
    /// `query` is plain words; each is matched as a prefix and all must
    /// appear. Best matches first.
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
        let Some(expr) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT run_id, snippet(runs_fts, 2, '[', ']', '…', 14)
             FROM runs_fts WHERE runs_fts MATCH ?1 ORDER BY bm25(runs_fts)",
        )?;
        let rows = stmt.query_map(params![expr], |r| Ok(SearchHit { run: r.get(0)?, snippet: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Upgrade an existing database from `from` to [`SCHEMA_VERSION`].
fn migrate(conn: &Connection, from: i64) -> anyhow::Result<()> {
    for v in from..SCHEMA_VERSION {
        let step = MIGRATIONS
            .get((v - 1) as usize)
            .ok_or_else(|| anyhow::anyhow!("no migration from schema version {v}"))?;
        tracing::info!(from = v, to = v + 1, "migrating event store");
        conn.execute_batch(step)?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![(v + 1).to_string()],
        )?;
    }
    Ok(())
}

/// Recompute a run's folded columns from its event log and index it.
///
/// Called exactly once per run, when it ends; a second call would insert a
/// duplicate FTS row, which is why the callers guard on the status update
/// having changed a row.
fn fold(tx: &rusqlite::Transaction<'_>, run: &str) -> anyhow::Result<()> {
    let mut output = String::new();
    let mut plan: Vec<String> = Vec::new();
    let mut tools: Vec<String> = Vec::new();

    let mut stmt = tx.prepare(
        "SELECT kind, payload FROM events
         WHERE run_id = ?1 AND kind IN ('message', 'plan', 'tool_call') ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![run], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (kind, payload) = row?;
        let value: serde_json::Value = serde_json::from_str(&payload)?;
        match kind.as_str() {
            "message" => output.push_str(value["text"].as_str().unwrap_or_default()),
            "plan" => {
                plan = value["entries"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
            }
            "tool_call" => tools.push(value["title"].as_str().unwrap_or_default().to_owned()),
            _ => {}
        }
    }
    drop(stmt);

    tx.execute(
        "UPDATE runs SET output = ?2, plan = ?3, tool_count = ?4 WHERE id = ?1",
        params![run, output, serde_json::to_string(&plan)?, tools.len() as i64],
    )?;

    let prompt: String = tx.query_row("SELECT prompt FROM runs WHERE id = ?1", params![run], |r| r.get(0))?;
    tx.execute(
        "INSERT INTO runs_fts(run_id, prompt, output, tools) VALUES (?1, ?2, ?3, ?4)",
        params![run, prompt, output, tools.join("\n")],
    )?;
    Ok(())
}

/// Turn free text into an FTS5 expression that cannot fail to parse:
/// every whitespace-separated word becomes a quoted prefix term.
fn fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Columns of a run summary, in the order `row_to_summary` reads them.
const RUN_SELECT: &str = "SELECT id, lane, prompt, cwd, status, started_at, duration_ms, session_id,
                                 stop_reason, error, output, plan, tool_count, agent, track
                          FROM runs";

/// Columns of a track, in the order `row_to_track` reads them.
const TRACK_SELECT: &str = "SELECT t.id, t.name, t.intent, t.cwd, t.agent, t.created_at, t.updated_at,
                                   (SELECT COUNT(*) FROM runs r WHERE r.track = t.id),
                                   t.conductor_config, t.worker_agent, t.worker_config, t.color, t.tags
                            FROM tracks t";

fn row_to_track(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackInfo> {
    let conductor: String = r.get(8)?;
    let worker: String = r.get(10)?;
    let tags: String = r.get(12)?;
    Ok(TrackInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        intent: r.get(2)?,
        cwd: r.get(3)?,
        agent: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        runs: r.get::<_, i64>(7)? as u32,
        conductor_config: serde_json::from_str(&conductor).unwrap_or_default(),
        worker_agent: r.get(9)?,
        worker_config: serde_json::from_str(&worker).unwrap_or_default(),
        color: r.get(11)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
    })
}

/// Tags as the `tracks` row stores them: JSON, trimmed, empties and
/// duplicates dropped.
fn tags_json(tags: Option<&Vec<String>>) -> anyhow::Result<String> {
    let mut clean: Vec<String> = Vec::new();
    for t in tags.into_iter().flatten() {
        let t = t.trim();
        if !t.is_empty() && !clean.iter().any(|c| c == t) {
            clean.push(t.to_string());
        }
    }
    Ok(serde_json::to_string(&clean)?)
}

/// A config map as the `tracks` row stores it: JSON, empty values dropped.
fn config_json(config: Option<&BTreeMap<String, String>>) -> anyhow::Result<String> {
    let clean: BTreeMap<&String, &String> = config
        .map(|c| c.iter().filter(|(_, v)| !v.trim().is_empty()).collect())
        .unwrap_or_default();
    Ok(serde_json::to_string(&clean)?)
}

fn row_to_summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunSummary> {
    let status: String = r.get(4)?;
    let plan: String = r.get(11)?;
    Ok(RunSummary {
        id: r.get(0)?,
        track: r.get(14)?,
        lane: r.get(1)?,
        agent: r.get(13)?,
        prompt: r.get(2)?,
        cwd: r.get(3)?,
        status: RunStatus::parse(&status).unwrap_or(RunStatus::Failed),
        started_at: r.get(5)?,
        duration_ms: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
        session_id: r.get(7)?,
        stop_reason: r.get(8)?,
        error: r.get(9)?,
        output: r.get(10)?,
        plan: serde_json::from_str(&plan).unwrap_or_default(),
        tool_count: r.get::<_, i64>(12)? as u32,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, title: &str) -> LaneEvent {
        LaneEvent::ToolCall {
            id: id.into(),
            title: title.into(),
            tool_kind: "execute".into(),
            status: "pending".into(),
        }
    }

    /// Track every test run goes into unless it says otherwise.
    const TR: &str = "tr001";

    fn drive_run(store: &Store, lane: &str, prompt: &str, output: &[&str], tools: &[&str]) -> RunId {
        let run = store.begin_run(TR, lane, "claude_code", prompt, ".").unwrap();
        store.append(&run, 10, &LaneEvent::Connected { protocol: "v1".into(), load_session: true }).unwrap();
        store.append(&run, 20, &LaneEvent::Started { session_id: "sess".into(), cwd: ".".into() }).unwrap();
        for (i, t) in tools.iter().enumerate() {
            store.append(&run, 30 + i as u64, &tool(&i.to_string(), t)).unwrap();
        }
        for chunk in output {
            store.append(&run, 50, &LaneEvent::Message { text: (*chunk).into() }).unwrap();
        }
        store.append(&run, 60, &LaneEvent::Plan { entries: vec!["a".into(), "b".into()] }).unwrap();
        store.append(&run, 90, &LaneEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
        run
    }

    #[test]
    fn ids_are_sequential() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.begin_run(TR, "solo", "codex", "a", ".").unwrap(), "t001");
        assert_eq!(store.begin_run(TR, "solo", "codex", "b", ".").unwrap(), "t002");
        assert_eq!(store.run("t001").unwrap().unwrap().agent, "codex");
        assert_eq!(store.run("t001").unwrap().unwrap().track, TR);
    }

    #[test]
    fn tracks_are_created_listed_updated_and_deleted() {
        let store = Store::in_memory().unwrap();
        let a = store.create_track(&new_track("  Parser ", "fix the tokenizer", "C:/repo", "claude_code")).unwrap();
        let b = store.create_track(&new_track("Docs", "", "C:/repo", "codex")).unwrap();
        assert_eq!((a.id.as_str(), a.name.as_str()), ("tr001", "Parser"));
        assert_eq!(b.id, "tr002");
        assert_eq!(store.tracks().unwrap().len(), 2);
        assert!(a.conductor_config.is_empty() && a.worker_agent.is_empty(), "defaults are empty");
        assert_eq!(a.lane_agent(), "claude_code", "lanes follow the conductor by default");
        assert!(store.create_track(&TrackPatch { name: Some("  ".into()), ..new_track("x", "", ".", "codex") }).is_err());

        // Runs count per track and bump its activity time.
        let before = store.track("tr001").unwrap().unwrap().updated_at;
        store.begin_run("tr001", "conductor", "claude_code", "hi", "C:/repo").unwrap();
        store.begin_run("tr001", "ui", "claude_code", "fix", "C:/repo").unwrap();
        store.begin_run("tr002", "conductor", "codex", "hi", "C:/repo").unwrap();
        let a = store.track("tr001").unwrap().unwrap();
        assert_eq!(a.runs, 2);
        assert!(a.updated_at >= before);
        assert_eq!(store.lanes("tr001").unwrap().len(), 2);
        assert_eq!(store.lanes("tr002").unwrap().len(), 1);

        let patch = TrackPatch {
            name: Some("Lexer".into()),
            agent: Some("copilot".into()),
            conductor_config: Some(BTreeMap::from([("model".to_string(), "gpt-5.4".to_string()), ("mode".to_string(), "  ".to_string())])),
            worker_agent: Some("codex".into()),
            worker_config: Some(BTreeMap::from([("mode".to_string(), "agent-full-access".to_string())])),
            ..TrackPatch::default()
        };
        let a = store.update_track("tr001", &patch).unwrap();
        assert_eq!((a.name.as_str(), a.intent.as_str(), a.agent.as_str()), ("Lexer", "fix the tokenizer", "copilot"));
        assert_eq!(a.conductor_config, BTreeMap::from([("model".to_string(), "gpt-5.4".to_string())]), "blank values are dropped");
        assert_eq!((a.worker_agent.as_str(), a.lane_agent()), ("codex", "codex"));
        assert_eq!(a.worker_config.get("mode").map(String::as_str), Some("agent-full-access"));
        let a = store
            .update_track("tr001", &TrackPatch { color: Some("#7aa2f7".into()), tags: Some(vec![" rust ".into(), "study".into(), "rust".into(), "".into()]), ..TrackPatch::default() })
            .unwrap();
        assert_eq!((a.color.as_str(), a.tags.clone()), ("#7aa2f7", vec!["rust".to_string(), "study".to_string()]));
        // A patch without a field keeps it.
        let a = store.update_track("tr001", &TrackPatch { intent: Some("lex it".into()), ..TrackPatch::default() }).unwrap();
        assert_eq!((a.name.as_str(), a.intent.as_str(), a.worker_agent.as_str()), ("Lexer", "lex it", "codex"));
        assert!(store.update_track("tr009", &TrackPatch { name: Some("x".into()), ..TrackPatch::default() }).is_err());

        // Deleting takes the runs, their events and remembered sessions with it.
        store.set_meta("conductor_session:tr001:copilot", "s1").unwrap();
        store.set_meta("lane_session:tr001/ui", "{}").unwrap();
        store.set_meta("lane_session:tr002/ui", "{}").unwrap();
        store.delete_track("tr001").unwrap();
        assert_eq!(store.tracks().unwrap().len(), 1);
        assert!(store.run("t001").unwrap().is_none());
        assert_eq!(store.runs().unwrap().len(), 1);
        assert_eq!(store.get_meta("conductor_session:tr001:copilot").unwrap(), None);
        assert_eq!(store.get_meta("lane_session:tr001/ui").unwrap(), None);
        assert_eq!(store.get_meta("lane_session:tr002/ui").unwrap().as_deref(), Some("{}"));
        assert!(store.delete_track("tr001").is_err());
        // Ids never reuse a deleted number.
        assert_eq!(store.create_track(&new_track("Again", "", ".", "codex")).unwrap().id, "tr003");
    }

    fn new_track(name: &str, intent: &str, cwd: &str, agent: &str) -> TrackPatch {
        TrackPatch {
            name: Some(name.into()),
            intent: Some(intent.into()),
            cwd: Some(cwd.into()),
            agent: Some(agent.into()),
            ..TrackPatch::default()
        }
    }

    #[test]
    fn meta_round_trips() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.get_meta("agents").unwrap(), None);
        store.set_meta("agents", "[1]").unwrap();
        store.set_meta("agents", "[2]").unwrap();
        assert_eq!(store.get_meta("agents").unwrap().as_deref(), Some("[2]"));
        assert_eq!(store.get_meta("schema_version").unwrap().as_deref(), Some(SCHEMA_VERSION.to_string().as_str()));
    }

    #[test]
    fn migrates_a_version_1_database() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v1.db");
        let _ = std::fs::remove_file(&path);

        // Build a v1 file by hand: the v1 schema had no `agent` column and
        // no tracks.
        {
            let conn = Connection::open(&path).unwrap();
            let v1 = SCHEMA
                .replace(",\n    agent       TEXT    NOT NULL DEFAULT 'claude_code'", "")
                .replace("    track       TEXT    NOT NULL DEFAULT '',\n", "")
                .replace("CREATE INDEX IF NOT EXISTS runs_by_track ON runs(track, n);", "");
            conn.execute_batch(&v1).unwrap();
            conn.execute("DROP TABLE tracks", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', '1')", []).unwrap();
            conn.execute(
                "INSERT INTO runs(n, id, lane, prompt, cwd, status, started_at) VALUES (1, 't001', 'conductor', 'old', 'C:/old', 'done', 5)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('conductor_session:claude_code', 'sess-c')", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('lane_session:ui', '{\"a\":1}')", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('setting:models', '{\"claude_code\":\"opus[1m]\",\"codex\":\"gpt-5.5\"}')", []).unwrap();
        }

        let store = Store::open(&path).unwrap();
        assert_eq!(store.get_meta("schema_version").unwrap().as_deref(), Some(SCHEMA_VERSION.to_string().as_str()));
        // The global model choice for the first track's agent became its conductor model.
        let first = store.track("tr001").unwrap().unwrap();
        assert_eq!(first.conductor_config.get("model").map(String::as_str), Some("opus[1m]"));
        assert_eq!(store.get_meta("setting:models").unwrap(), None);
        let old = store.run("t001").unwrap().unwrap();
        assert_eq!(old.agent, "claude_code", "pre-migration runs default to Claude");
        assert_eq!(old.track, "tr001", "pre-migration runs join the first track");
        let tracks = store.tracks().unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!((tracks[0].cwd.as_str(), tracks[0].runs, tracks[0].created_at), ("C:/old", 1, 5));
        // Remembered sessions move under the track so nothing forgets.
        assert_eq!(store.get_meta("conductor_session:tr001:claude_code").unwrap().as_deref(), Some("sess-c"));
        assert_eq!(store.get_meta("lane_session:tr001/ui").unwrap().as_deref(), Some("{\"a\":1}"));
        let new = store.begin_run("tr001", "solo", "copilot", "new", ".").unwrap();
        assert_eq!(store.run(&new).unwrap().unwrap().agent, "copilot");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn projection_folds_at_finish() {
        let store = Store::in_memory().unwrap();
        let run = drive_run(&store, "solo", "do it", &["hel", "lo"], &["cargo test", "ls"]);

        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::Done);
        assert_eq!(s.output, "hello");
        assert_eq!(s.plan, vec!["a", "b"]);
        assert_eq!(s.tool_count, 2);
        assert_eq!(s.duration_ms, Some(90));
        assert_eq!(s.session_id.as_deref(), Some("sess"));
        assert_eq!(s.stop_reason.as_deref(), Some("end_turn"));

        // Full log is replayable, in order, including what is below the membrane.
        let events = store.events(&run).unwrap();
        assert_eq!(events.len(), 8);
        assert_eq!(events[0].event.kind(), "connected");
        assert_eq!(events[7].event.kind(), "finished");
        assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn timeline_lists_runs_in_order_without_detail() {
        let store = Store::in_memory().unwrap();
        drive_run(&store, "solo", "first", &["x"], &[]);
        let live = store.begin_run(TR, "solo", "claude_code", "second", ".").unwrap();
        store.append(&live, 5, &LaneEvent::Started { session_id: "s2".into(), cwd: ".".into() }).unwrap();

        let runs = store.runs().unwrap();
        assert_eq!(runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["t001", "t002"]);
        assert_eq!(runs[0].status, RunStatus::Done);
        assert_eq!(runs[1].status, RunStatus::Running);
        assert_eq!(runs[1].output, "", "live runs are not folded yet");
    }

    #[test]
    fn lanes_are_grouped_from_runs_newest_first() {
        let store = Store::in_memory().unwrap();
        drive_run(&store, "conductor", "hi", &["x"], &[]);
        drive_run(&store, "ui", "fix", &["y"], &["ls"]);
        drive_run(&store, "ui", "more", &["z"], &[]);
        drive_run(&store, "docs", "write", &["w"], &[]);

        let lanes = store.lanes(TR).unwrap();
        let names: Vec<&str> = lanes.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["docs", "ui", "conductor"]);
        let ui = lanes.iter().find(|l| l.name == "ui").unwrap();
        assert_eq!(ui.runs, 2);
        assert_eq!(ui.last_run, "t003");
        assert_eq!(ui.last_status, RunStatus::Done);
        assert_eq!(ui.agent, "claude_code");
    }

    #[test]
    fn failure_records_error_and_folds() {
        let store = Store::in_memory().unwrap();
        let run = store.begin_run(TR, "solo", "claude_code", "p", ".").unwrap();
        store.append(&run, 1, &LaneEvent::Message { text: "partial".into() }).unwrap();
        store.append(&run, 2, &LaneEvent::Failed { error: "boom".into() }).unwrap();
        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::Failed);
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert_eq!(s.output, "partial");
    }

    #[test]
    fn events_after_end_do_not_reopen_or_reindex() {
        let store = Store::in_memory().unwrap();
        let run = drive_run(&store, "solo", "p", &["done"], &[]);
        store.append(&run, 100, &LaneEvent::Failed { error: "late".into() }).unwrap();
        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::Done);
        assert_eq!(s.error, None);
        assert_eq!(store.search("done").unwrap().len(), 1, "one FTS row per run");
    }

    #[test]
    fn interrupted_runs_are_closed_on_open() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);

        let run = {
            let store = Store::open(&path).unwrap();
            let run = store.begin_run(TR, "solo", "claude_code", "never finishes", ".").unwrap();
            store.append(&run, 7, &LaneEvent::Started { session_id: "s".into(), cwd: ".".into() }).unwrap();
            store.append(&run, 8, &LaneEvent::Message { text: "half".into() }).unwrap();
            run
        };

        let store = Store::open(&path).unwrap();
        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::Failed);
        assert_eq!(s.error.as_deref(), Some(INTERRUPTED));
        assert_eq!(s.output, "half");
        assert_eq!(s.duration_ms, Some(8), "closed at the last event's time");
        let last = store.events(&run).unwrap().pop().unwrap();
        assert_eq!(last.event.kind(), "failed");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_matches_prompt_output_and_tools() {
        let store = Store::in_memory().unwrap();
        let a = drive_run(&store, "solo", "fix the parser", &["rewrote tokenizer"], &["cargo test -p parser"]);
        let b = drive_run(&store, "solo", "write docs", &["added README section"], &[]);

        let hit = |q: &str| store.search(q).unwrap().into_iter().map(|h| h.run).collect::<Vec<_>>();
        assert_eq!(hit("tokenizer"), vec![a.clone()]);
        assert_eq!(hit("parser"), vec![a.clone()]);
        assert_eq!(hit("README"), vec![b.clone()]);
        assert_eq!(hit("readm"), vec![b.clone()], "prefix match");
        assert_eq!(hit("nothing-here"), Vec::<String>::new());
        assert_eq!(hit("  "), Vec::<String>::new());
        assert_eq!(hit("\"quoted\" or (junk"), Vec::<String>::new(), "user text never breaks the query");

        let snippet = store.search("tokenizer").unwrap().remove(0).snippet;
        assert!(snippet.contains("[tokenizer]"), "{snippet}");
    }

    #[test]
    fn fts_query_quotes_every_term() {
        assert_eq!(fts_query("a b"), Some("\"a\"* \"b\"*".into()));
        assert_eq!(fts_query("say \"hi\""), Some("\"say\"* \"\"\"hi\"\"\"*".into()));
        assert_eq!(fts_query("   "), None);
    }
}
