//! Event store: the durable side of the membrane.
//!
//! Every [`AgentEvent`] a run emits is appended, in order, to an append-only
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

use orchestra_core::{AgentEvent, SessionName, RunId, RunStatus};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Bump when `SCHEMA` changes in a way that needs a migration, and add the
/// step to [`migrate`].
const SCHEMA_VERSION: i64 = 13;

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
    // 5 -> 6: decisions get a table. The ones `record_decision` kept as a
    // JSON list in meta move over as decided, their options as labels.
    "CREATE TABLE IF NOT EXISTS decisions (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        track       TEXT    NOT NULL,
        run         TEXT,
        question    TEXT    NOT NULL,
        context     TEXT    NOT NULL DEFAULT '',
        options     TEXT    NOT NULL DEFAULT '[]',
        recommended INTEGER,
        allow_other INTEGER NOT NULL DEFAULT 1,
        status      TEXT    NOT NULL,
        choice      INTEGER,
        answer      TEXT,
        note        TEXT    NOT NULL DEFAULT '',
        created_at  INTEGER NOT NULL,
        decided_at  INTEGER
    );
    CREATE INDEX IF NOT EXISTS decisions_by_track ON decisions(track, id);
    INSERT INTO decisions(track, question, options, allow_other, status, answer, note, created_at, decided_at)
        SELECT COALESCE(json_extract(d.value, '$.track'), ''),
               COALESCE(json_extract(d.value, '$.question'), ''),
               COALESCE((SELECT json_group_array(json_object('label', o.value, 'detail', ''))
                         FROM json_each(json_extract(d.value, '$.options')) o), '[]'),
               0, 'decided',
               json_extract(d.value, '$.choice'),
               COALESCE(json_extract(d.value, '$.rationale'), ''),
               COALESCE(json_extract(d.value, '$.at'), 0),
               COALESCE(json_extract(d.value, '$.at'), 0)
        FROM meta m, json_each(m.value) d
        WHERE m.key = 'decisions' AND json_valid(m.value);
    DELETE FROM meta WHERE key = 'decisions';",
    // 6 -> 7: drafts, sketch boards worked out with an agent before a track.
    "CREATE TABLE IF NOT EXISTS drafts (
        id         TEXT    PRIMARY KEY,
        title      TEXT    NOT NULL,
        agent      TEXT    NOT NULL,
        doc        TEXT    NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );",
    // 7 -> 8: a draft's own session options (model, effort…), as a track's conductor has.
    "ALTER TABLE drafts ADD COLUMN config TEXT NOT NULL DEFAULT '{}';",
    // 8 -> 9: a colour and tags per draft, as tracks have.
    "ALTER TABLE drafts ADD COLUMN color TEXT NOT NULL DEFAULT '';
    ALTER TABLE drafts ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';",
    // 9 -> 10: drafts become artifacts of kind 'design', the first kind of
    // thing a human keeps beside tracks (knowledge comes next). Ids, the
    // conversation's key and lane, and remembered sessions move with them.
    "CREATE TABLE IF NOT EXISTS artifacts (
        id         TEXT    PRIMARY KEY,
        kind       TEXT    NOT NULL,
        title      TEXT    NOT NULL,
        agent      TEXT    NOT NULL DEFAULT '',
        config     TEXT    NOT NULL DEFAULT '{}',
        color      TEXT    NOT NULL DEFAULT '',
        tags       TEXT    NOT NULL DEFAULT '[]',
        body       TEXT    NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS artifacts_by_kind ON artifacts(kind, updated_at);
    INSERT INTO artifacts(id, kind, title, agent, config, color, tags, body, created_at, updated_at)
        SELECT 'ar' || substr(id, 3), 'design', title, agent, config, color, tags, doc, created_at, updated_at FROM drafts;
    UPDATE runs SET track = 'artifact:ar' || substr(track, 9), lane = 'artifact' WHERE track LIKE 'draft:dr%';
    UPDATE meta SET key = 'artifact_session:ar' || substr(key, 17) WHERE key LIKE 'draft_session:dr%';
    DROP TABLE drafts;",
    // 10 -> 11: on screen and in code a lane is a worker now. A run's column
    // says which session it belongs to (`conductor`, a worker's name,
    // `artifact`), and a worker's remembered session moves to its new key.
    "ALTER TABLE runs RENAME COLUMN lane TO session;
    UPDATE meta SET key = 'worker_session:' || substr(key, 14) WHERE key LIKE 'lane_session:%';",
    // 11 -> 12: a decision can be an agent's permission question put to the
    // human: which session asked, and its request id, as JSON.
    "ALTER TABLE decisions ADD COLUMN permission TEXT;",
    // 12 -> 13: worker reports kept from before lanes became workers read
    // like the ones after, so the timeline shows them as reports.
    "UPDATE runs SET prompt = '[worker-report] worker=' || substr(prompt, 20) WHERE prompt LIKE '[lane-report] lane=%';
    UPDATE runs_fts SET prompt = '[worker-report] worker=' || substr(prompt, 20) WHERE prompt LIKE '[lane-report] lane=%';",
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
    session     TEXT    NOT NULL,
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

CREATE TABLE IF NOT EXISTS decisions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track       TEXT    NOT NULL,
    run         TEXT,
    question    TEXT    NOT NULL,
    context     TEXT    NOT NULL DEFAULT '',
    options     TEXT    NOT NULL DEFAULT '[]',
    recommended INTEGER,
    allow_other INTEGER NOT NULL DEFAULT 1,
    status      TEXT    NOT NULL,
    choice      INTEGER,
    answer      TEXT,
    note        TEXT    NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    decided_at  INTEGER,
    permission  TEXT
);
CREATE INDEX IF NOT EXISTS decisions_by_track ON decisions(track, id);

CREATE TABLE IF NOT EXISTS artifacts (
    id         TEXT    PRIMARY KEY,
    kind       TEXT    NOT NULL,
    title      TEXT    NOT NULL,
    agent      TEXT    NOT NULL DEFAULT '',
    config     TEXT    NOT NULL DEFAULT '{}',
    color      TEXT    NOT NULL DEFAULT '',
    tags       TEXT    NOT NULL DEFAULT '[]',
    body       TEXT    NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS artifacts_by_kind ON artifacts(kind, updated_at);

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
    pub session: SessionName,
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

/// One choice a decision offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOption {
    /// Short, what the human clicks.
    pub label: String,
    /// What choosing it means: consequences, trade-offs. May be empty.
    #[serde(default)]
    pub detail: String,
    /// For a permission question, the agent's id for this answer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
}

/// A decision that is an agent's permission question: which session asked
/// (`conductor`, or `artifact` for an artifact's agent) and its request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAsk {
    pub session: String,
    pub request: String,
}

/// Where a decision stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Waiting for the human.
    Open,
    /// The human chose.
    Decided,
    /// The human set it aside without choosing.
    Dismissed,
}

impl DecisionStatus {
    fn as_str(self) -> &'static str {
        match self {
            DecisionStatus::Open => "open",
            DecisionStatus::Decided => "decided",
            DecisionStatus::Dismissed => "dismissed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "open" => DecisionStatus::Open,
            "dismissed" => DecisionStatus::Dismissed,
            _ => DecisionStatus::Decided,
        }
    }
}

/// A question the conductor put to the human, with the options it offered
/// and, once answered, what the human chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub track: String,
    /// The conductor run that asked, when known.
    pub run: Option<String>,
    pub question: String,
    /// Why it matters, in a sentence or two.
    pub context: String,
    pub options: Vec<DecisionOption>,
    /// Index into `options` the conductor recommends.
    pub recommended: Option<usize>,
    /// Whether the human may answer in their own words.
    pub allow_other: bool,
    pub status: DecisionStatus,
    /// Index into `options` the human chose; `None` for an answer in their own words.
    pub choice: Option<usize>,
    /// The chosen option's label, or the human's own words.
    pub answer: Option<String>,
    pub note: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub decided_at: Option<i64>,
    /// Set when the decision is an agent's permission question.
    pub permission: Option<PermissionAsk>,
}

/// What a new open decision carries.
#[derive(Debug, Clone, Default)]
pub struct NewDecision {
    pub track: String,
    pub run: Option<String>,
    pub question: String,
    pub context: String,
    pub options: Vec<DecisionOption>,
    pub recommended: Option<usize>,
    pub allow_other: bool,
    pub permission: Option<PermissionAsk>,
}

const DECISION_SELECT: &str = "SELECT id, track, run, question, context, options, recommended, allow_other, status, choice, answer, note, created_at, decided_at, permission FROM decisions";

fn row_to_decision(r: &rusqlite::Row<'_>) -> rusqlite::Result<Decision> {
    let options: String = r.get(5)?;
    let status: String = r.get(8)?;
    Ok(Decision {
        id: r.get(0)?,
        track: r.get(1)?,
        run: r.get(2)?,
        question: r.get(3)?,
        context: r.get(4)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        recommended: r.get::<_, Option<i64>>(6)?.and_then(|i| usize::try_from(i).ok()),
        allow_other: r.get::<_, i64>(7)? != 0,
        status: DecisionStatus::parse(&status),
        choice: r.get::<_, Option<i64>>(9)?.and_then(|i| usize::try_from(i).ok()),
        answer: r.get(10)?,
        note: r.get(11)?,
        created_at: r.get(12)?,
        decided_at: r.get(13)?,
        permission: r.get::<_, Option<String>>(14)?.and_then(|p| serde_json::from_str(&p).ok()),
    })
}

/// An artifact: something the human keeps beside tracks and can attach to
/// them — a design (a sketch board worked out with an agent), knowledge
/// later. What is inside (`body`) is JSON the app owns per kind; the store
/// keeps it whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub id: String,
    /// `design`, and more kinds to come.
    pub kind: String,
    pub title: String,
    /// The agent that works on it, when its kind has one.
    pub agent: String,
    /// That agent's session options (mode, model, effort…).
    pub config: BTreeMap<String, String>,
    /// `#rrggbb` for the bar beside it in its list, or empty.
    pub color: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What an artifact change may touch; `None` keeps a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtifactPatch {
    pub body: Option<String>,
    pub title: Option<String>,
    pub agent: Option<String>,
    pub config: Option<BTreeMap<String, String>>,
    pub color: Option<String>,
    pub tags: Option<Vec<String>>,
}

const ARTIFACT_SELECT: &str = "SELECT id, kind, title, agent, config, color, tags, created_at, updated_at FROM artifacts";

fn row_to_artifact(r: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactInfo> {
    Ok(ArtifactInfo {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        agent: r.get(3)?,
        config: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
        color: r.get(5)?,
        tags: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

/// A track: one conductor, its workers, one working directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: String,
    pub name: String,
    /// One line on what the track is for; shown under the name.
    pub intent: String,
    /// Directory the conductor and its workers work in.
    pub cwd: String,
    /// Agent the conductor runs on.
    pub agent: String,
    /// The conductor's session options, `option id → value id`, in the
    /// agent's own terms (`mode`, `model`, `reasoning_effort`, …). Absent
    /// options keep the agent's default; an absent `mode` means the most
    /// autonomous one.
    pub conductor_config: BTreeMap<String, String>,
    /// Agent workers run on; empty means the conductor's.
    pub worker_agent: String,
    /// Workers' session options, like `conductor_config`.
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
    /// The options workers open with: the worker's own, or, when the worker
    /// was never set apart (no agent, no options), the conductor's.
    pub fn effective_worker_config(&self) -> &BTreeMap<String, String> {
        if self.worker_agent.is_empty() && self.worker_config.is_empty() {
            &self.conductor_config
        } else {
            &self.worker_config
        }
    }

    /// The agent workers run on.
    pub fn effective_worker_agent(&self) -> &str {
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

/// A session as the record knows it: its runs, whoever ran them last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
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
    pub event: AgentEvent,
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

    /// An in-memory store, for tests and `DIVISI_DB=:memory:`.
    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> anyhow::Result<Self> {
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
                let tx = conn.transaction()?;
                tx.execute_batch(SCHEMA)?;
                tx.execute(
                    "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
                tx.commit()?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) if v < SCHEMA_VERSION => migrate(&mut conn, v)?,
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
            self.append(&run, at_ms, &AgentEvent::Failed { error: INTERRUPTED.to_string() })?;
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

    /// Keep a new artifact; ids are `ar001`, `ar002`, … in creation order
    /// across kinds.
    pub fn create_artifact(&self, kind: &str, title: &str, agent: &str, body: &str) -> anyhow::Result<ArtifactInfo> {
        let id = {
            let conn = self.conn.lock();
            // Past the highest id there is and the highest ever given out,
            // so a deleted artifact's id (and its folder) is never reused.
            let highest: i64 = conn.query_row("SELECT COALESCE(MAX(CAST(substr(id, 3) AS INTEGER)), 0) FROM artifacts", [], |r| r.get(0))?;
            let given: i64 = conn
                .query_row("SELECT value FROM meta WHERE key = 'artifact_seq'", [], |r| r.get::<_, String>(0))
                .optional()?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let next = highest.max(given) + 1;
            conn.execute(
                "INSERT INTO meta(key, value) VALUES ('artifact_seq', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![next.to_string()],
            )?;
            let id = format!("ar{next:03}");
            let now = now_ms();
            conn.execute(
                "INSERT INTO artifacts(id, kind, title, agent, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![id, kind, title, agent, body, now],
            )?;
            id
        };
        self.artifact(&id)?.map(|(a, _)| a).ok_or_else(|| anyhow::anyhow!("artifact {id} vanished"))
    }

    /// Artifacts of one kind, or of every kind, most recently touched first.
    pub fn artifacts(&self, kind: Option<&str>) -> anyhow::Result<Vec<ArtifactInfo>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{ARTIFACT_SELECT} WHERE ?1 IS NULL OR kind = ?1 ORDER BY updated_at DESC, id DESC"))?;
        let rows = stmt.query_map(params![kind], row_to_artifact)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One artifact and its body.
    pub fn artifact(&self, id: &str) -> anyhow::Result<Option<(ArtifactInfo, String)>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id, kind, title, agent, config, color, tags, created_at, updated_at, body FROM artifacts WHERE id = ?1",
                params![id],
                |r| Ok((row_to_artifact(r)?, r.get::<_, String>(9)?)),
            )
            .optional()?)
    }

    /// Change an artifact's fields; `None` in the patch keeps a field.
    pub fn update_artifact(&self, id: &str, patch: &ArtifactPatch) -> anyhow::Result<ArtifactInfo> {
        let config = patch.config.as_ref().map(serde_json::to_string).transpose()?;
        let tags = patch.tags.as_ref().map(serde_json::to_string).transpose()?;
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE artifacts SET body = COALESCE(?2, body), title = COALESCE(?3, title), agent = COALESCE(?4, agent),
                 config = COALESCE(?6, config), color = COALESCE(?7, color), tags = COALESCE(?8, tags), updated_at = ?5
                 WHERE id = ?1",
                params![id, patch.body, patch.title, patch.agent, now_ms(), config, patch.color, tags],
            )?;
            if changed == 0 {
                anyhow::bail!("no artifact {id}");
            }
        }
        self.artifact(id)?.map(|(a, _)| a).ok_or_else(|| anyhow::anyhow!("artifact {id} vanished"))
    }

    /// Delete an artifact with its conversation and remembered sessions.
    pub fn delete_artifact(&self, id: &str) -> anyhow::Result<()> {
        let key = format!("artifact:{id}");
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM runs_fts WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![key])?;
        tx.execute("DELETE FROM events WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![key])?;
        tx.execute("DELETE FROM runs WHERE track = ?1", params![key])?;
        tx.execute("DELETE FROM meta WHERE key LIKE 'artifact_session:' || ?1 || ':%'", params![id])?;
        let changed = tx.execute("DELETE FROM artifacts WHERE id = ?1", params![id])?;
        if changed == 0 {
            anyhow::bail!("no artifact {id}");
        }
        tx.commit()?;
        Ok(())
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

    /// Forget a track's remembered conductor and worker sessions, so they
    /// open fresh next time (needed when the working directory changes).
    pub fn forget_track_sessions(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'worker_session:' || ?1 || '/%' OR key LIKE 'worktree:' || ?1 || '/%'",
            params![id],
        )?;
        Ok(())
    }

    /// Delete a track with every run, event and remembered session in it.
    pub fn delete_track(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM runs_fts WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![id])?;
        // Worker reports the app keeps as meta `report:<run>`; run ids are
        // reused once the newest runs are gone, so they must not outlive them.
        tx.execute("DELETE FROM meta WHERE key IN (SELECT 'report:' || id FROM runs WHERE track = ?1)", params![id])?;
        tx.execute("DELETE FROM events WHERE run_id IN (SELECT id FROM runs WHERE track = ?1)", params![id])?;
        tx.execute("DELETE FROM runs WHERE track = ?1", params![id])?;
        tx.execute("DELETE FROM decisions WHERE track = ?1", params![id])?;
        tx.execute(
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'worker_session:' || ?1 || '/%' OR key LIKE 'worktree:' || ?1 || '/%'",
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
    pub fn begin_run(&self, track: &str, session: &str, agent: &str, prompt: &str, cwd: &str) -> anyhow::Result<RunId> {
        let conn = self.conn.lock();
        let next: i64 = conn.query_row("SELECT COALESCE(MAX(n), 0) + 1 FROM runs", [], |r| r.get(0))?;
        let id = format!("t{next:03}");
        let now = now_ms();
        conn.execute(
            "INSERT INTO runs(n, id, track, session, agent, prompt, cwd, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![next, id, track, session, agent, prompt, cwd, RunStatus::Connecting.as_str(), now],
        )?;
        conn.execute("UPDATE tracks SET updated_at = ?2 WHERE id = ?1", params![track, now])?;
        Ok(id)
    }

    /// Append one event to a run's log and fold it into the projection.
    ///
    /// Returns the event's sequence number. Appending to a run that already
    /// ended is allowed (the log stays truthful) but does not reopen it.
    pub fn append(&self, run: &str, at_ms: u64, event: &AgentEvent) -> anyhow::Result<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let payload = serde_json::to_string(event)?;
        tx.execute(
            "INSERT INTO events(run_id, at_ms, kind, above, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run, at_ms as i64, event.kind(), event.above_membrane() as i64, payload],
        )?;
        let seq = tx.last_insert_rowid();

        match event {
            AgentEvent::Started { session_id, .. } => {
                tx.execute(
                    "UPDATE runs SET status = ?2, session_id = ?3 WHERE id = ?1 AND status = 'connecting'",
                    params![run, RunStatus::Running.as_str(), session_id],
                )?;
            }
            AgentEvent::Finished { stop_reason } => {
                let changed = tx.execute(
                    "UPDATE runs SET status = ?2, stop_reason = ?3, ended_at = ?4, duration_ms = ?5
                     WHERE id = ?1 AND status IN ('connecting', 'running')",
                    params![run, RunStatus::Done.as_str(), stop_reason, now_ms(), at_ms as i64],
                )?;
                if changed > 0 {
                    fold(&tx, run)?;
                }
            }
            AgentEvent::Failed { error } => {
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

    /// Every session that ever had a run in a track (the conductor, each
    /// worker, by name), newest activity first,
    /// with the agent and status of its latest run. Workers outlive their
    /// sessions: this is how a closed worker is still known.
    pub fn sessions(&self, track: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let conn = self.conn.lock();
        // The latest run per session, joined back for its id, status and time.
        let mut stmt = conn.prepare(
            "SELECT r.session, r.agent, c.runs, r.id, r.status, r.started_at
             FROM runs r
             JOIN (SELECT session, COUNT(*) AS runs, MAX(n) AS last_n FROM runs WHERE track = ?1 GROUP BY session) c
               ON c.session = r.session AND c.last_n = r.n
             ORDER BY r.n DESC",
        )?;
        let rows = stmt.query_map(params![track], |r| {
            let status: String = r.get(4)?;
            Ok(SessionInfo {
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

    /// Put a question to the human. It stays open until answered or dismissed.
    pub fn open_decision(&self, new: &NewDecision) -> anyhow::Result<Decision> {
        if new.question.trim().is_empty() {
            anyhow::bail!("a decision needs a question");
        }
        if new.options.len() < 2 && !new.allow_other {
            anyhow::bail!("a decision needs at least two options, or allow_other");
        }
        if new.recommended.is_some_and(|i| i >= new.options.len()) {
            anyhow::bail!("recommended is not one of the options");
        }
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO decisions(track, run, question, context, options, recommended, allow_other, status, created_at, permission)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new.track,
                new.run,
                new.question.trim(),
                new.context.trim(),
                serde_json::to_string(&new.options)?,
                new.recommended.map(|i| i as i64),
                new.allow_other as i64,
                DecisionStatus::Open.as_str(),
                now_ms(),
                new.permission.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.decision(id)?.ok_or_else(|| anyhow::anyhow!("decision {id} vanished"))
    }

    /// Keep a decision the human made in conversation, already decided.
    pub fn record_decision(
        &self,
        track: &str,
        run: Option<&str>,
        question: &str,
        options: &[String],
        choice: &str,
        rationale: &str,
    ) -> anyhow::Result<Decision> {
        let options: Vec<DecisionOption> = options
            .iter()
            .map(|label| DecisionOption { label: label.clone(), detail: String::new(), id: String::new() })
            .collect();
        let index = options.iter().position(|o| o.label == choice);
        let now = now_ms();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO decisions(track, run, question, options, allow_other, status, choice, answer, note, created_at, decided_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                track,
                run,
                question,
                serde_json::to_string(&options)?,
                DecisionStatus::Decided.as_str(),
                index.map(|i| i as i64),
                choice,
                rationale,
                now,
            ],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.decision(id)?.ok_or_else(|| anyhow::anyhow!("decision {id} vanished"))
    }

    /// One decision, if it exists.
    pub fn decision(&self, id: i64) -> anyhow::Result<Option<Decision>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("{DECISION_SELECT} WHERE id = ?1"), params![id], row_to_decision)
            .optional()?)
    }

    /// A track's decisions, or every track's, oldest first.
    pub fn decisions(&self, track: Option<&str>) -> anyhow::Result<Vec<Decision>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{DECISION_SELECT} WHERE ?1 IS NULL OR track = ?1 ORDER BY id"))?;
        let rows = stmt.query_map(params![track], row_to_decision)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Answer an open decision: one of its options, or the human's own
    /// words when the decision allows them.
    pub fn answer_decision(&self, id: i64, choice: Option<usize>, own: Option<&str>, note: &str) -> anyhow::Result<Decision> {
        let d = self.decision(id)?.ok_or_else(|| anyhow::anyhow!("no decision {id}"))?;
        if d.status != DecisionStatus::Open {
            anyhow::bail!("decision {id} is already {}", d.status.as_str());
        }
        let answer = match (choice, own.map(str::trim).filter(|s| !s.is_empty())) {
            (Some(i), _) => d
                .options
                .get(i)
                .map(|o| o.label.clone())
                .ok_or_else(|| anyhow::anyhow!("decision {id} has no option {i}"))?,
            (None, Some(text)) if d.allow_other => text.to_string(),
            (None, Some(_)) => anyhow::bail!("decision {id} takes one of its options"),
            (None, None) => anyhow::bail!("choose an option or write an answer"),
        };
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE decisions SET status = ?2, choice = ?3, answer = ?4, note = ?5, decided_at = ?6 WHERE id = ?1",
            params![id, DecisionStatus::Decided.as_str(), choice.map(|i| i as i64), answer, note.trim(), now_ms()],
        )?;
        drop(conn);
        self.decision(id)?.ok_or_else(|| anyhow::anyhow!("decision {id} vanished"))
    }

    /// Set an open decision aside without answering it.
    pub fn dismiss_decision(&self, id: i64) -> anyhow::Result<Decision> {
        let conn = self.conn.lock();
        let changed = conn.execute(
            "UPDATE decisions SET status = ?2, decided_at = ?3 WHERE id = ?1 AND status = 'open'",
            params![id, DecisionStatus::Dismissed.as_str(), now_ms()],
        )?;
        drop(conn);
        if changed == 0 {
            anyhow::bail!("decision {id} is not open");
        }
        self.decision(id)?.ok_or_else(|| anyhow::anyhow!("decision {id} vanished"))
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
            let event: AgentEvent = serde_json::from_str(&payload).map_err(|e| {
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

/// Upgrade an existing database from `from` to [`SCHEMA_VERSION`]. Each
/// step and its version bump commit together, so a crash mid-way never
/// leaves a half-applied step that fails on the next start.
fn migrate(conn: &mut Connection, from: i64) -> anyhow::Result<()> {
    for v in from..SCHEMA_VERSION {
        let step = MIGRATIONS
            .get((v - 1) as usize)
            .ok_or_else(|| anyhow::anyhow!("no migration from schema version {v}"))?;
        tracing::info!(from = v, to = v + 1, "migrating event store");
        let tx = conn.transaction()?;
        tx.execute_batch(step)?;
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![(v + 1).to_string()],
        )?;
        tx.commit()?;
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
const RUN_SELECT: &str = "SELECT id, session, prompt, cwd, status, started_at, duration_ms, session_id,
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
        session: r.get(1)?,
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

    #[test]
    fn old_lane_reports_read_as_worker_reports() {
        let path = std::env::temp_dir().join(format!("orchestra-store-reports-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (old, other) = {
            let store = Store::open(&path).unwrap();
            let old = store.begin_run("tr001", "conductor", "claude_code", "[lane-report] lane=scan run=t3 status=done\n\nbody", "/w").unwrap();
            let other = store.begin_run("tr001", "conductor", "claude_code", "tell me about [lane-report] lane=x", "/w").unwrap();
            store.set_meta("schema_version", "12").unwrap();
            (old, other)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.run(&old).unwrap().unwrap().prompt, "[worker-report] worker=scan run=t3 status=done\n\nbody");
        assert_eq!(store.run(&other).unwrap().unwrap().prompt, "tell me about [lane-report] lane=x", "only prompts that start with it");
        drop(store);
        let _ = std::fs::remove_file(&path);
    }

    fn tool(id: &str, title: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            id: id.into(),
            title: title.into(),
            tool_kind: "execute".into(),
            status: "pending".into(),
            paths: vec![],
        }
    }

    /// Track every test run goes into unless it says otherwise.
    const TR: &str = "tr001";

    fn drive_run(store: &Store, session: &str, prompt: &str, output: &[&str], tools: &[&str]) -> RunId {
        let run = store.begin_run(TR, session, "claude_code", prompt, ".").unwrap();
        store.append(&run, 10, &AgentEvent::Connected { protocol: "v1".into(), load_session: true }).unwrap();
        store.append(&run, 20, &AgentEvent::Started { session_id: "sess".into(), cwd: ".".into() }).unwrap();
        for (i, t) in tools.iter().enumerate() {
            store.append(&run, 30 + i as u64, &tool(&i.to_string(), t)).unwrap();
        }
        for chunk in output {
            store.append(&run, 50, &AgentEvent::Message { text: (*chunk).into() }).unwrap();
        }
        store.append(&run, 60, &AgentEvent::Plan { entries: vec!["a".into(), "b".into()] }).unwrap();
        store.append(&run, 90, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
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
        assert_eq!(a.effective_worker_agent(), "claude_code", "workers follow the conductor by default");
        assert!(store.create_track(&TrackPatch { name: Some("  ".into()), ..new_track("x", "", ".", "codex") }).is_err());

        // Runs count per track and bump its activity time.
        let before = store.track("tr001").unwrap().unwrap().updated_at;
        store.begin_run("tr001", "conductor", "claude_code", "hi", "C:/repo").unwrap();
        store.begin_run("tr001", "ui", "claude_code", "fix", "C:/repo").unwrap();
        store.begin_run("tr002", "conductor", "codex", "hi", "C:/repo").unwrap();
        let a = store.track("tr001").unwrap().unwrap();
        assert_eq!(a.runs, 2);
        assert!(a.updated_at >= before);
        assert_eq!(store.sessions("tr001").unwrap().len(), 2);
        assert_eq!(store.sessions("tr002").unwrap().len(), 1);

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
        assert_eq!((a.worker_agent.as_str(), a.effective_worker_agent()), ("codex", "codex"));
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
        store.set_meta("worker_session:tr001/ui", "{}").unwrap();
        store.set_meta("worker_session:tr002/ui", "{}").unwrap();
        store.set_meta("report:t001", "{}").unwrap();
        store.set_meta("worktree:tr001/ui", "{}").unwrap();
        store.delete_track("tr001").unwrap();
        assert_eq!(store.get_meta("report:t001").unwrap(), None, "reports go with their runs");
        assert_eq!(store.get_meta("worktree:tr001/ui").unwrap(), None, "checkout records go with the track");
        assert_eq!(store.tracks().unwrap().len(), 1);
        assert!(store.run("t001").unwrap().is_none());
        assert_eq!(store.runs().unwrap().len(), 1);
        assert_eq!(store.get_meta("conductor_session:tr001:copilot").unwrap(), None);
        assert_eq!(store.get_meta("worker_session:tr001/ui").unwrap(), None);
        assert_eq!(store.get_meta("worker_session:tr002/ui").unwrap().as_deref(), Some("{}"));
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
            // Before v11 a run's session column was called lane.
            conn.execute("ALTER TABLE runs RENAME COLUMN session TO lane", []).unwrap();
            // Nor did decisions carry a permission before v12.
            conn.execute("ALTER TABLE decisions DROP COLUMN permission", []).unwrap();
            // Artifacts came at v10 (drafts at v7); the migrations make them.
            conn.execute("DROP TABLE artifacts", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', '1')", []).unwrap();
            conn.execute(
                "INSERT INTO runs(n, id, lane, prompt, cwd, status, started_at) VALUES (1, 't001', 'conductor', 'old', 'C:/old', 'done', 5)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('conductor_session:claude_code', 'sess-c')", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('lane_session:ui', '{\"a\":1}')", []).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('setting:models', '{\"claude_code\":\"opus[1m]\",\"codex\":\"gpt-5.5\"}')", []).unwrap();
            conn.execute(
                "INSERT INTO meta(key, value) VALUES ('decisions', '[{\"track\":\"tr001\",\"question\":\"Which db?\",\"options\":[\"sqlite\",\"pg\"],\"choice\":\"sqlite\",\"rationale\":\"local\",\"at\":7,\"n\":1}]')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        assert_eq!(store.get_meta("schema_version").unwrap().as_deref(), Some(SCHEMA_VERSION.to_string().as_str()));
        // The global model choice for the first track's agent became its conductor model.
        let first = store.track("tr001").unwrap().unwrap();
        assert_eq!(first.conductor_config.get("model").map(String::as_str), Some("opus[1m]"));
        assert_eq!(store.get_meta("setting:models").unwrap(), None);
        // Decisions kept in meta moved into their table.
        let old_decisions = store.decisions(Some("tr001")).unwrap();
        assert_eq!(old_decisions.len(), 1);
        assert_eq!(old_decisions[0].status, DecisionStatus::Decided);
        assert_eq!(old_decisions[0].answer.as_deref(), Some("sqlite"));
        assert_eq!(old_decisions[0].options[1].label, "pg");
        assert_eq!(store.get_meta("decisions").unwrap(), None);
        let old = store.run("t001").unwrap().unwrap();
        assert_eq!(old.agent, "claude_code", "pre-migration runs default to Claude");
        assert_eq!(old.track, "tr001", "pre-migration runs join the first track");
        let tracks = store.tracks().unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!((tracks[0].cwd.as_str(), tracks[0].runs, tracks[0].created_at), ("C:/old", 1, 5));
        // Remembered sessions move under the track so nothing forgets.
        assert_eq!(store.get_meta("conductor_session:tr001:claude_code").unwrap().as_deref(), Some("sess-c"));
        assert_eq!(store.get_meta("worker_session:tr001/ui").unwrap().as_deref(), Some("{\"a\":1}"), "a worker's memory survives every rename");
        let new = store.begin_run("tr001", "solo", "copilot", "new", ".").unwrap();
        assert_eq!(store.run(&new).unwrap().unwrap().agent, "copilot");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workers_take_the_conductors_options_until_set_apart() {
        let store = Store::in_memory().unwrap();
        let conductor: BTreeMap<String, String> = [("model".to_string(), "opus".to_string())].into();
        let tr = store
            .create_track(&TrackPatch {
                name: Some("t".into()),
                cwd: Some(".".into()),
                agent: Some("claude_code".into()),
                conductor_config: Some(conductor.clone()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!((tr.effective_worker_agent(), tr.effective_worker_config()), ("claude_code", &conductor));
        let apart = store
            .update_track(&tr.id, &TrackPatch { worker_agent: Some("codex".into()), ..Default::default() })
            .unwrap();
        assert_eq!(apart.effective_worker_agent(), "codex");
        assert!(apart.effective_worker_config().is_empty(), "a worker set apart keeps its own options");
    }

    #[test]
    fn artifacts_are_kept_listed_by_kind_and_deleted_with_their_runs() {
        let store = Store::in_memory().unwrap();
        let a = store.create_artifact("design", "wrap-up", "claude_code", "{}").unwrap();
        let b = store.create_artifact("knowledge", "notes", "", "{}").unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str(), a.kind.as_str()), ("ar001", "ar002", "design"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        let model: BTreeMap<String, String> = [("model".to_string(), "opus".to_string())].into();
        let saved = store
            .update_artifact(
                &a.id,
                &ArtifactPatch {
                    body: Some("{\"v\":1}".into()),
                    title: Some("renamed".into()),
                    config: Some(model.clone()),
                    color: Some("#7aa2f7".into()),
                    tags: Some(vec!["ux".into()]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((saved.title.as_str(), saved.agent.as_str(), saved.color.as_str()), ("renamed", "claude_code", "#7aa2f7"));
        assert_eq!((saved.config.clone(), saved.tags.clone()), (model.clone(), vec!["ux".to_string()]));
        let kept = store.update_artifact(&a.id, &ArtifactPatch { title: Some("again".into()), ..Default::default() }).unwrap();
        assert_eq!((kept.config, kept.tags), (model, vec!["ux".to_string()]), "a patch keeps what it does not name");
        assert_eq!(store.artifact(&a.id).unwrap().unwrap().1, "{\"v\":1}");
        assert_eq!(store.artifacts(None).unwrap()[0].id, "ar001", "most recently touched first");
        assert_eq!(store.artifacts(Some("knowledge")).unwrap().len(), 1);
        let run = store.begin_run("artifact:ar001", "artifact", "claude_code", "hi", ".").unwrap();
        store.delete_artifact(&a.id).unwrap();
        assert!(store.artifact(&a.id).unwrap().is_none());
        assert!(store.run(&run).unwrap().is_none(), "its conversation goes with it");
        assert!(store.delete_artifact(&a.id).is_err());
        let again = store.create_artifact("design", "fresh", "claude_code", "{}").unwrap();
        assert_eq!(again.id, "ar003", "a deleted id is not given out again");
    }

    #[test]
    fn drafts_move_into_artifacts_with_their_conversation() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v9.db");
        {
            // A v9 file: the current schema with drafts where artifacts are.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute_batch(
                "ALTER TABLE runs RENAME COLUMN session TO lane;
                 ALTER TABLE decisions DROP COLUMN permission;
                 DROP TABLE artifacts;
                 CREATE TABLE drafts (id TEXT PRIMARY KEY, title TEXT NOT NULL, agent TEXT NOT NULL,
                   doc TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                   config TEXT NOT NULL DEFAULT '{}', color TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '[]');
                 INSERT INTO drafts VALUES ('dr003', 'board', 'codex', '{\"nodes\":[]}', 1, 2, '{}', '#e0af68', '[\"ux\"]');
                 INSERT INTO meta(key, value) VALUES ('schema_version', '9');
                 INSERT INTO meta(key, value) VALUES ('draft_session:dr003:codex', 'sess-d');
                 INSERT INTO runs(n, id, track, lane, prompt, cwd, status, started_at)
                   VALUES (1, 't001', 'draft:dr003', 'drafter', 'hi', '.', 'done', 5);",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let (a, body) = store.artifact("ar003").unwrap().unwrap();
        assert_eq!((a.kind.as_str(), a.title.as_str(), a.color.as_str(), body.as_str()), ("design", "board", "#e0af68", "{\"nodes\":[]}"));
        assert_eq!(a.tags, vec!["ux".to_string()]);
        let run = store.run("t001").unwrap().unwrap();
        assert_eq!((run.track.as_str(), run.session.as_str()), ("artifact:ar003", "artifact"));
        assert_eq!(store.get_meta("artifact_session:ar003:codex").unwrap().as_deref(), Some("sess-d"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decisions_open_answer_dismiss() {
        let store = Store::in_memory().unwrap();
        let opt = |l: &str| DecisionOption { label: l.into(), detail: String::new(), id: String::new() };
        let new = NewDecision {
            track: "tr001".into(),
            run: Some("t001".into()),
            question: "Which layout?".into(),
            context: "Affects every page".into(),
            options: vec![opt("A"), opt("B"), opt("C")],
            recommended: Some(1),
            allow_other: true,
            permission: None,
        };
        let d = store.open_decision(&new).unwrap();
        assert_eq!((d.status, d.recommended, d.run.as_deref()), (DecisionStatus::Open, Some(1), Some("t001")));
        assert!(store.answer_decision(d.id, Some(5), None, "").is_err(), "out of range");
        assert!(store.answer_decision(d.id, None, Some("  "), "").is_err(), "empty answer");
        let done = store.answer_decision(d.id, Some(2), None, " go ").unwrap();
        assert_eq!((done.status, done.choice, done.answer.as_deref(), done.note.as_str()), (DecisionStatus::Decided, Some(2), Some("C"), "go"));
        assert!(store.answer_decision(d.id, Some(0), None, "").is_err(), "answered once");

        let own = store.open_decision(&new).unwrap();
        let mine = store.answer_decision(own.id, None, Some("neither"), "").unwrap();
        assert_eq!((mine.choice, mine.answer.as_deref()), (None, Some("neither")));

        let closed = store.open_decision(&NewDecision { allow_other: false, ..new.clone() }).unwrap();
        assert!(store.answer_decision(closed.id, None, Some("x"), "").is_err(), "options only");
        assert_eq!(store.dismiss_decision(closed.id).unwrap().status, DecisionStatus::Dismissed);
        assert!(store.dismiss_decision(closed.id).is_err());

        assert!(store.open_decision(&NewDecision { recommended: Some(3), ..new.clone() }).is_err());
        assert!(store.open_decision(&NewDecision { options: vec![opt("only")], allow_other: false, ..new.clone() }).is_err());

        let rec = store.record_decision("tr002", None, "Name?", &["x".into(), "y".into()], "y", "shorter").unwrap();
        assert_eq!((rec.status, rec.choice, rec.note.as_str()), (DecisionStatus::Decided, Some(1), "shorter"));
        assert_eq!(store.decisions(Some("tr001")).unwrap().len(), 3);
        assert_eq!(store.decisions(None).unwrap().len(), 4);
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
        store.append(&live, 5, &AgentEvent::Started { session_id: "s2".into(), cwd: ".".into() }).unwrap();

        let runs = store.runs().unwrap();
        assert_eq!(runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["t001", "t002"]);
        assert_eq!(runs[0].status, RunStatus::Done);
        assert_eq!(runs[1].status, RunStatus::Running);
        assert_eq!(runs[1].output, "", "live runs are not folded yet");
    }

    #[test]
    fn sessions_are_grouped_from_runs_newest_first() {
        let store = Store::in_memory().unwrap();
        drive_run(&store, "conductor", "hi", &["x"], &[]);
        drive_run(&store, "ui", "fix", &["y"], &["ls"]);
        drive_run(&store, "ui", "more", &["z"], &[]);
        drive_run(&store, "docs", "write", &["w"], &[]);

        let sessions = store.sessions(TR).unwrap();
        let names: Vec<&str> = sessions.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["docs", "ui", "conductor"]);
        let ui = sessions.iter().find(|l| l.name == "ui").unwrap();
        assert_eq!(ui.runs, 2);
        assert_eq!(ui.last_run, "t003");
        assert_eq!(ui.last_status, RunStatus::Done);
        assert_eq!(ui.agent, "claude_code");
    }

    #[test]
    fn failure_records_error_and_folds() {
        let store = Store::in_memory().unwrap();
        let run = store.begin_run(TR, "solo", "claude_code", "p", ".").unwrap();
        store.append(&run, 1, &AgentEvent::Message { text: "partial".into() }).unwrap();
        store.append(&run, 2, &AgentEvent::Failed { error: "boom".into() }).unwrap();
        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::Failed);
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert_eq!(s.output, "partial");
    }

    #[test]
    fn events_after_end_do_not_reopen_or_reindex() {
        let store = Store::in_memory().unwrap();
        let run = drive_run(&store, "solo", "p", &["done"], &[]);
        store.append(&run, 100, &AgentEvent::Failed { error: "late".into() }).unwrap();
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
            store.append(&run, 7, &AgentEvent::Started { session_id: "s".into(), cwd: ".".into() }).unwrap();
            store.append(&run, 8, &AgentEvent::Message { text: "half".into() }).unwrap();
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
