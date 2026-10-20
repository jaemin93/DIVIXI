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

use orchestra_core::path::plain;
use orchestra_core::{AgentEvent, SessionName, RunId, RunStatus};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Bump when `SCHEMA` changes in a way that needs a migration, and add the
/// step to [`migrate`]. A step that an older build must be able to undo
/// (so a database can go back to it) gets its inverse in [`DOWNGRADES`].
const SCHEMA_VERSION: i64 = 23;

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
    // 13 -> 14: where a track's workers work: a folder of their own under
    // the track's (the default), a git worktree, or the track folder itself.
    "ALTER TABLE tracks ADD COLUMN worker_folder TEXT NOT NULL DEFAULT 'subfolder';",
    // 14 -> 15: workers that had worked in a track were given a new
    // "<name>-2" folder after a restart; their records go, so each works in
    // its own folder again.
    "DELETE FROM meta WHERE key LIKE 'worker_dir:%';",
    // 15 -> 16: routines — a track's saved instructions, each with the
    // worker it runs as, and which runs came from which.
    "CREATE TABLE IF NOT EXISTS routines (
        id          TEXT    PRIMARY KEY,
        track       TEXT    NOT NULL,
        name        TEXT    NOT NULL,
        instruction TEXT    NOT NULL,
        worker      TEXT    NOT NULL,
        agent       TEXT    NOT NULL DEFAULT '',
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS routines_by_track ON routines(track, updated_at);
    CREATE TABLE IF NOT EXISTS routine_runs (
        routine TEXT    NOT NULL,
        run     TEXT    NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
        at      INTEGER NOT NULL,
        PRIMARY KEY (routine, run)
    );
    CREATE INDEX IF NOT EXISTS routine_runs_by_routine ON routine_runs(routine, at);",
    // 16 -> 17: a routine is its own thing, not a worker. It carries the
    // folder it works in and the agent it runs on rather than borrowing a
    // track's, and its runs are found by their own key, so the table that
    // linked them goes. A routine made under the old shape keeps its words
    // and takes the folder and agent its track was lending it.
    "ALTER TABLE routines ADD COLUMN cwd TEXT NOT NULL DEFAULT '';
    ALTER TABLE routines ADD COLUMN config TEXT NOT NULL DEFAULT '{}';
    UPDATE routines SET cwd = COALESCE((SELECT t.cwd FROM tracks t WHERE t.id = routines.track), '') WHERE cwd = '';
    UPDATE routines SET agent = COALESCE((SELECT CASE WHEN t.worker_agent <> '' THEN t.worker_agent ELSE t.agent END
                                          FROM tracks t WHERE t.id = routines.track), '') WHERE agent = '';
    DROP TABLE IF EXISTS routine_runs;
    DROP INDEX IF EXISTS routines_by_track;
    CREATE INDEX IF NOT EXISTS routines_by_time ON routines(updated_at);",
    // 17 -> 18: a routine's runs get a table of their own. Keeping them in
    // `runs` under a made-up track key was a leak: every query that reads
    // runs without naming a track saw them, and a routine's work appeared
    // in a track's history. The ones already written are removed with
    // everything they left in `events` and the search index -- they are not
    // worth migrating into the new shape, and leaving them is the bug.
    // The `track` a routine was told to report to goes as well: a finished
    // run says so on the bell now, so there is nothing to choose.
    "CREATE TABLE IF NOT EXISTS routine_runs (
        id          TEXT    PRIMARY KEY,
        routine     TEXT    NOT NULL,
        agent       TEXT    NOT NULL,
        cwd         TEXT    NOT NULL,
        instruction TEXT    NOT NULL,
        status      TEXT    NOT NULL,
        started_at  INTEGER NOT NULL,
        ended_at    INTEGER,
        output      TEXT    NOT NULL DEFAULT '',
        error       TEXT    NOT NULL DEFAULT '',
        tools       INTEGER NOT NULL DEFAULT 0,
        report      TEXT    NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS routine_runs_by_routine ON routine_runs(routine, started_at);
    DELETE FROM runs_fts WHERE run_id IN (SELECT id FROM runs WHERE track LIKE 'routine:%');
    DELETE FROM events WHERE run_id IN (SELECT id FROM runs WHERE track LIKE 'routine:%');
    DELETE FROM runs WHERE track LIKE 'routine:%';
    ALTER TABLE routines DROP COLUMN track;",
    // 18 -> 19: whether the conductor may put a worker on something other
    // than the track's default. `follow` is what every track did until now,
    // so an existing track keeps its behaviour exactly.
    "ALTER TABLE tracks ADD COLUMN worker_choice TEXT NOT NULL DEFAULT 'follow';",
    // 19 -> 20: `routines` as `SCHEMA` has it. The table was made at
    // 15 -> 16 carrying `worker TEXT NOT NULL`, from when a routine ran as
    // one. Two revisions later it runs on its own agent and nothing writes
    // that column, but no step ever removed it -- so a database that came
    // through the migrations kept a NOT NULL column with no default, and
    // making a routine on it failed with "NOT NULL constraint failed:
    // routines.worker" while a fresh install was fine.
    //
    // Rebuilt rather than `DROP COLUMN`, because both shapes are out there:
    // a database first created while `SCHEMA` already omitted `worker` does
    // not have the column to drop. Naming the columns to carry over says
    // what the table is either way, and leaves nothing behind.
    "CREATE TABLE routines_rebuilt (
        id          TEXT    PRIMARY KEY,
        name        TEXT    NOT NULL,
        instruction TEXT    NOT NULL,
        cwd         TEXT    NOT NULL,
        agent       TEXT    NOT NULL,
        config      TEXT    NOT NULL DEFAULT '{}',
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    );
    INSERT INTO routines_rebuilt(id, name, instruction, cwd, agent, config, created_at, updated_at)
        SELECT id, name, instruction, cwd, agent, config, created_at, updated_at FROM routines;
    DROP TABLE routines;
    ALTER TABLE routines_rebuilt RENAME TO routines;
    CREATE INDEX IF NOT EXISTS routines_by_time ON routines(updated_at);",
    // 20 -> 21: a routine may carry a plan: the artifact (kind 'plan') that
    // holds its steps and the conversation that shaped them. Empty is no
    // plan, and the routine runs on its instruction alone, as before. A run
    // keeps the plan it was given beyond its instruction, and what its agent
    // said of each step; both stay empty for a run without a plan.
    "ALTER TABLE routines ADD COLUMN plan TEXT NOT NULL DEFAULT '';
    ALTER TABLE routine_runs ADD COLUMN plan TEXT NOT NULL DEFAULT '';
    ALTER TABLE routine_runs ADD COLUMN steps TEXT NOT NULL DEFAULT '';",
    // 21 -> 22: a routine's colour and tags, as a track's and a design's: the
    // bar beside it in its list and the tags shown above its name.
    "ALTER TABLE routines ADD COLUMN color TEXT NOT NULL DEFAULT '';
    ALTER TABLE routines ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';",
    // 22 -> 23: what survives the app going away. A run keeps the id of the
    // agent process behind it (so a run the app lost can be told from one
    // whose agent is still alive) and whether it is safe to run again by
    // itself. Three tables that outlive sessions: notices for the bell,
    // schedules a conductor set, and background jobs being watched.
    // `DOWNGRADES[0]` is the way back.
    "ALTER TABLE runs ADD COLUMN pid INTEGER;
    ALTER TABLE runs ADD COLUMN pid_start INTEGER;
    ALTER TABLE runs ADD COLUMN repeatable INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE routine_runs ADD COLUMN pid INTEGER;
    ALTER TABLE routine_runs ADD COLUMN pid_start INTEGER;
    CREATE TABLE IF NOT EXISTS notices (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        track      TEXT    NOT NULL DEFAULT '',
        kind       TEXT    NOT NULL,
        title      TEXT    NOT NULL,
        body       TEXT    NOT NULL DEFAULT '',
        data       TEXT    NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL,
        read_at    INTEGER
    );
    CREATE INDEX IF NOT EXISTS notices_open ON notices(read_at, created_at);
    CREATE TABLE IF NOT EXISTS schedules (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        track      TEXT    NOT NULL,
        kind       TEXT    NOT NULL,
        spec       TEXT    NOT NULL DEFAULT '{}',
        message    TEXT    NOT NULL,
        next_at    INTEGER,
        last_at    INTEGER,
        fired      INTEGER NOT NULL DEFAULT 0,
        status     TEXT    NOT NULL DEFAULT 'active',
        note       TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        created_in TEXT,
        seen       INTEGER,
        seen_sum   INTEGER
    );
    CREATE INDEX IF NOT EXISTS schedules_due ON schedules(status, next_at);
    CREATE TABLE IF NOT EXISTS jobs (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        track         TEXT    NOT NULL,
        worker        TEXT    NOT NULL DEFAULT '',
        what          TEXT    NOT NULL DEFAULT '',
        pid           INTEGER,
        pid_start     INTEGER,
        log           TEXT    NOT NULL DEFAULT '',
        done_pattern  TEXT    NOT NULL DEFAULT '',
        stall_minutes INTEGER NOT NULL DEFAULT 30,
        status        TEXT    NOT NULL DEFAULT 'running',
        boot          TEXT    NOT NULL DEFAULT '',
        registered_at INTEGER NOT NULL,
        changed_at    INTEGER NOT NULL,
        ended_at      INTEGER,
        read_bytes    INTEGER NOT NULL DEFAULT 0,
        read_sum      INTEGER,
        last_line     TEXT    NOT NULL DEFAULT '',
        note          TEXT    NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS jobs_by_track ON jobs(track, status);",
];

/// The way back from a schema version to the one before it, newest first:
/// `DOWNGRADES[0]` takes [`SCHEMA_VERSION`] to `SCHEMA_VERSION - 1`. Only
/// steps an older build may need to be handed a database from have one;
/// [`Store::downgrade`] refuses a version with no way back.
///
/// Going back loses what the newer shape kept and nothing else. Runs that
/// ended in a way the older build does not know (`interrupted`,
/// `timed_out`, `detached`) read as failed there, with the same words in
/// `error`, and the events that moved them are rewritten the same way --
/// an older build cannot read an event of a kind it does not know, and one
/// such event would keep it from opening the whole run.
const DOWNGRADES: &[&str] = &[
    // 23 -> 22
    "UPDATE runs SET status = 'failed' WHERE status IN ('interrupted', 'timed_out', 'detached');
    UPDATE events SET kind = 'failed',
                      payload = json_set(json_remove(payload, '$.why'), '$.kind', 'failed', '$.error', COALESCE(json_extract(payload, '$.why'), ''))
      WHERE kind IN ('interrupted', 'timed_out', 'detached');
    ALTER TABLE runs DROP COLUMN pid;
    ALTER TABLE runs DROP COLUMN pid_start;
    ALTER TABLE runs DROP COLUMN repeatable;
    ALTER TABLE routine_runs DROP COLUMN pid;
    ALTER TABLE routine_runs DROP COLUMN pid_start;
    DROP TABLE IF EXISTS notices;
    DROP TABLE IF EXISTS schedules;
    DROP TABLE IF EXISTS jobs;
    DELETE FROM meta WHERE key IN ('boot_id', 'alive_at', 'comeback_pending') OR key LIKE 'notes:%' OR key LIKE 'quiet:%' OR key LIKE 'worker_scope:%' OR key LIKE 'resumed:%';",
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
    tags             TEXT    NOT NULL DEFAULT '[]',
    worker_folder    TEXT    NOT NULL DEFAULT 'subfolder',
    worker_choice    TEXT    NOT NULL DEFAULT 'follow'
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
    agent       TEXT    NOT NULL DEFAULT 'claude_code',
    -- The agent process behind the run, while the app had one: what tells
    -- a run the app lost from one whose agent is still alive. With when
    -- that process started (as the OS counts it), so a number given to
    -- another process later is not taken for it.
    pid         INTEGER,
    pid_start   INTEGER,
    -- Safe to run again by itself after an interruption (the conductor
    -- said so): a check that only reads, a report that only writes its own
    -- file. Default no: a turn that writes to a repository or a database
    -- is started again by a person, or not at all.
    repeatable  INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS runs_by_track ON runs(track, n);

-- What the human has to be told and has not read yet, kept here so it
-- survives the app closing and reaches whichever device opens next. The
-- bell shows these beside the open decision cards.
CREATE TABLE IF NOT EXISTS notices (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    track      TEXT    NOT NULL DEFAULT '',
    -- `interrupted`, `job`, `stall`, `schedule`: what the bell draws it as.
    kind       TEXT    NOT NULL,
    title      TEXT    NOT NULL,
    body       TEXT    NOT NULL DEFAULT '',
    -- Anything the screen needs to open the right thing (JSON).
    data       TEXT    NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    read_at    INTEGER
);
CREATE INDEX IF NOT EXISTS notices_open ON notices(read_at, created_at);

-- A conductor's schedules: when to wake it, with what. In the record so a
-- reboot does not lose them -- a schedule an agent keeps inside its own
-- session goes with that session, which is what this replaces.
CREATE TABLE IF NOT EXISTS schedules (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    track      TEXT    NOT NULL,
    -- `at`, `every`, `daily`, `weekly`, `file`, `process`.
    kind       TEXT    NOT NULL,
    -- The schedule as it was given (JSON), see src-tauri/src/schedule.rs.
    spec       TEXT    NOT NULL DEFAULT '{}',
    -- What the conductor is told when it fires.
    message    TEXT    NOT NULL,
    -- Unix milliseconds of the next firing; null for a condition that is
    -- looked at rather than waited for, and for one that is over.
    next_at    INTEGER,
    last_at    INTEGER,
    fired      INTEGER NOT NULL DEFAULT 0,
    -- `active`, `done`, `cancelled`.
    status     TEXT    NOT NULL DEFAULT 'active',
    note       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    -- The conductor run that set it, when known.
    created_in TEXT,
    -- For a `file` schedule: how many bytes of the file have been looked
    -- at, so nothing written is skipped and a restart goes on from there.
    -- Null before the first look.
    seen       INTEGER,
    -- A fingerprint of the bytes just before `seen`, so a file replaced by
    -- one already longer than that is read again from its start.
    seen_sum   INTEGER
);
CREATE INDEX IF NOT EXISTS schedules_due ON schedules(status, next_at);

-- A background process somebody started and nobody would otherwise watch:
-- its pid, its log, what its last line looks like when it is done. The app
-- looks at each every so often and wakes the conductor when one ends,
-- dies or goes quiet.
CREATE TABLE IF NOT EXISTS jobs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    track         TEXT    NOT NULL,
    worker        TEXT    NOT NULL DEFAULT '',
    what          TEXT    NOT NULL DEFAULT '',
    pid           INTEGER,
    -- When that process started, as the OS counts it: a number given to
    -- another process later is not taken for it.
    pid_start     INTEGER,
    log           TEXT    NOT NULL DEFAULT '',
    -- A regular expression that, matched in the log, means it finished.
    done_pattern  TEXT    NOT NULL DEFAULT '',
    stall_minutes INTEGER NOT NULL DEFAULT 30,
    -- `running`, `stalled`, `done`, `died`, `lost`.
    status        TEXT    NOT NULL DEFAULT 'running',
    -- The boot the process belongs to: a different one means it is gone.
    boot          TEXT    NOT NULL DEFAULT '',
    registered_at INTEGER NOT NULL,
    changed_at    INTEGER NOT NULL,
    ended_at      INTEGER,
    read_bytes    INTEGER NOT NULL DEFAULT 0,
    -- A fingerprint of the bytes just before `read_bytes`, so a log
    -- replaced by one already longer than that is read again from its start.
    read_sum      INTEGER,
    last_line     TEXT    NOT NULL DEFAULT '',
    note          TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS jobs_by_track ON jobs(track, status);

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

CREATE TABLE IF NOT EXISTS routines (
    id          TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    instruction TEXT    NOT NULL,
    cwd         TEXT    NOT NULL,
    agent       TEXT    NOT NULL,
    config      TEXT    NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    -- Its plan: the id of an artifact of kind 'plan', or empty for none.
    plan        TEXT    NOT NULL DEFAULT '',
    -- `#rrggbb` for the bar beside it in its list, or empty.
    color       TEXT    NOT NULL DEFAULT '',
    tags        TEXT    NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS routines_by_time ON routines(updated_at);

-- A routine's runs are its own, in their own table. They were kept in
-- `runs` once, under a made-up track key, and that leaked: a query that
-- reached for runs without naming a track found them, and a routine's work
-- turned up in a track's history. Nothing here is reachable from `runs`,
-- `events` or `runs_fts` at all, which is the only way to be sure.
CREATE TABLE IF NOT EXISTS routine_runs (
    id          TEXT    PRIMARY KEY,
    routine     TEXT    NOT NULL,
    agent       TEXT    NOT NULL,
    cwd         TEXT    NOT NULL,
    -- What was sent, as it read then: editing the routine afterwards must
    -- not rewrite what an earlier run was asked to do.
    instruction TEXT    NOT NULL,
    status      TEXT    NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    output      TEXT    NOT NULL DEFAULT '',
    error       TEXT    NOT NULL DEFAULT '',
    tools       INTEGER NOT NULL DEFAULT 0,
    -- The checked report, as JSON; empty when there was none to check.
    report      TEXT    NOT NULL DEFAULT '',
    -- The plan it was given, as it read then (JSON); empty for none.
    plan        TEXT    NOT NULL DEFAULT '',
    -- What its agent said of each step of that plan (JSON); empty for none.
    steps       TEXT    NOT NULL DEFAULT '',
    -- Its agent's process while it runs, and when that process started.
    -- Cleared when the run ends, and kept past a crash only while that
    -- agent is still alive: the routine does not run again beside it.
    pid         INTEGER,
    pid_start   INTEGER
);
CREATE INDEX IF NOT EXISTS routine_runs_by_routine ON routine_runs(routine, started_at);

CREATE VIRTUAL TABLE IF NOT EXISTS runs_fts USING fts5(
    run_id UNINDEXED,
    prompt,
    output,
    tools
);
"#;

/// Error text recorded on runs that were live when the app last closed,
/// when nothing more is known about how it closed. The app usually knows
/// more (a reboot, a crash, when the run was last seen alive) and says so
/// through [`Store::close_live_runs`] instead.
pub const INTERRUPTED: &str = "interrupted: the app closed while the run was live";

/// Meta key holding the highest run number ever handed out (see [`Store::begin_run`]).
const RUN_HIGH_WATER: &str = "run_high_water";

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
    /// Unix milliseconds, once the run ended. For a run the app lost, when
    /// it was last known to be alive, not when the app came back.
    #[serde(default)]
    pub ended_at: Option<i64>,
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
    /// The agent process behind the run, while the app had one.
    #[serde(default)]
    pub pid: Option<u32>,
    /// When that process started, as the OS counts it, when known.
    #[serde(default)]
    pub pid_start: Option<u64>,
    /// Safe to run again by itself after an interruption.
    #[serde(default)]
    pub repeatable: bool,
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

/// Something the human has to be told, kept until they have read it. The
/// bell shows these; a decision card is not one of them (it is its own
/// row, and it is answered rather than read).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: i64,
    /// The track it is about; empty when it is about the app.
    pub track: String,
    /// `interrupted`, `job`, `stall`, `schedule`, …: what the bell draws it as.
    pub kind: String,
    pub title: String,
    pub body: String,
    /// What the screen needs to open the right thing, per kind.
    pub data: serde_json::Value,
    /// Unix milliseconds.
    pub created_at: i64,
    pub read_at: Option<i64>,
}

/// What a new notice carries.
#[derive(Debug, Clone, Default)]
pub struct NewNotice {
    pub track: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub data: serde_json::Value,
}

const NOTICE_SELECT: &str = "SELECT id, track, kind, title, body, data, created_at, read_at FROM notices";

fn row_to_notice(r: &rusqlite::Row<'_>) -> rusqlite::Result<Notice> {
    let data: String = r.get(5)?;
    Ok(Notice {
        id: r.get(0)?,
        track: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        body: r.get(4)?,
        data: serde_json::from_str(&data).unwrap_or(serde_json::Value::Null),
        created_at: r.get(6)?,
        read_at: r.get(7)?,
    })
}

/// Where a schedule stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    /// Waiting for its time or its condition.
    Active,
    /// Fired for the last time.
    Done,
    /// Taken back before it fired.
    Cancelled,
}

impl ScheduleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduleStatus::Active => "active",
            ScheduleStatus::Done => "done",
            ScheduleStatus::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "active" => ScheduleStatus::Active,
            "cancelled" => ScheduleStatus::Cancelled,
            _ => ScheduleStatus::Done,
        }
    }
}

/// A time, a repetition or a condition on which a track's conductor is
/// woken with a message. Kept here so it outlives every session and the
/// machine's uptime; what the kinds and specs mean is the app's
/// (src-tauri/src/schedule.rs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: i64,
    pub track: String,
    pub kind: String,
    pub spec: serde_json::Value,
    pub message: String,
    /// Unix milliseconds of the next firing; `None` for a condition that
    /// is looked at rather than waited for, and once it is over.
    pub next_at: Option<i64>,
    pub last_at: Option<i64>,
    pub fired: u32,
    pub status: ScheduleStatus,
    /// What happened to it last, for a human: "ran 3 h late: the machine
    /// was off".
    pub note: String,
    pub created_at: i64,
    /// The conductor run that set it, when known.
    pub created_in: Option<String>,
    /// For a `file` schedule, how many bytes of the file were looked at;
    /// none before the first look.
    pub seen: Option<u64>,
    /// A fingerprint of the bytes just before `seen` (see `Job::read_sum`).
    pub seen_sum: Option<i64>,
}

/// What a new schedule carries.
#[derive(Debug, Clone, Default)]
pub struct NewSchedule {
    pub track: String,
    pub kind: String,
    pub spec: serde_json::Value,
    pub message: String,
    pub next_at: Option<i64>,
    pub created_in: Option<String>,
}

const SCHEDULE_SELECT: &str = "SELECT id, track, kind, spec, message, next_at, last_at, fired, status, note, created_at, created_in, seen, seen_sum FROM schedules";

fn row_to_schedule(r: &rusqlite::Row<'_>) -> rusqlite::Result<Schedule> {
    let spec: String = r.get(3)?;
    let status: String = r.get(8)?;
    Ok(Schedule {
        id: r.get(0)?,
        track: r.get(1)?,
        kind: r.get(2)?,
        spec: serde_json::from_str(&spec).unwrap_or(serde_json::Value::Null),
        message: r.get(4)?,
        next_at: r.get(5)?,
        last_at: r.get(6)?,
        fired: r.get::<_, i64>(7)? as u32,
        status: ScheduleStatus::parse(&status),
        note: r.get(9)?,
        created_at: r.get(10)?,
        created_in: r.get(11)?,
        seen: r.get::<_, Option<i64>>(12)?.map(|n| n.max(0) as u64),
        seen_sum: r.get(13)?,
    })
}

/// Where a watched background job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// The process is alive and its log moves.
    Running,
    /// Alive, but its log has not moved for longer than it was given.
    Stalled,
    /// Its log said it finished.
    Done,
    /// The process is gone without its log saying it finished.
    Died,
    /// The app can no longer tell: the log is unreadable, or the platform
    /// cannot ask whether a process is alive.
    Lost,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Stalled => "stalled",
            JobStatus::Done => "done",
            JobStatus::Died => "died",
            JobStatus::Lost => "lost",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "running" => JobStatus::Running,
            "stalled" => JobStatus::Stalled,
            "done" => JobStatus::Done,
            "died" => JobStatus::Died,
            _ => JobStatus::Lost,
        }
    }

    /// Whether the app still looks at it.
    pub fn is_open(self) -> bool {
        matches!(self, JobStatus::Running | JobStatus::Stalled)
    }
}

/// A background process registered to be watched: by a worker that
/// started a two-hour collection with `nohup` and ended its turn, or by the
/// conductor on its behalf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub track: String,
    /// The worker that owns it; empty for the conductor's own.
    pub worker: String,
    /// What it is, in the registrant's words.
    pub what: String,
    pub pid: Option<u32>,
    /// When that process started, when known.
    pub pid_start: Option<u64>,
    /// The log file it writes, if any.
    pub log: String,
    /// A regular expression that, matched in the log, means it finished.
    pub done_pattern: String,
    /// Minutes the log may stay still before it is reported stalled.
    pub stall_minutes: u32,
    pub status: JobStatus,
    /// The boot its process belongs to.
    pub boot: String,
    pub registered_at: i64,
    /// When its log (or its state) last moved.
    pub changed_at: i64,
    pub ended_at: Option<i64>,
    /// How much of the log has been read, so the rest is read next time.
    pub read_bytes: u64,
    /// A fingerprint of the bytes just before `read_bytes`: when they read
    /// otherwise, the log was replaced and is read from its start.
    pub read_sum: Option<i64>,
    pub last_line: String,
    /// What happened to it, for a human.
    pub note: String,
}

/// What a new job carries.
#[derive(Debug, Clone, Default)]
pub struct NewJob {
    pub track: String,
    pub worker: String,
    pub what: String,
    pub pid: Option<u32>,
    /// When that process started (see `RunSummary::pid_start`).
    pub pid_start: Option<u64>,
    pub log: String,
    pub done_pattern: String,
    pub stall_minutes: u32,
    pub boot: String,
}

const JOB_SELECT: &str = "SELECT id, track, worker, what, pid, log, done_pattern, stall_minutes, status, boot,
                                 registered_at, changed_at, ended_at, read_bytes, last_line, note, read_sum, pid_start
                          FROM jobs";

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let status: String = r.get(8)?;
    Ok(Job {
        id: r.get(0)?,
        track: r.get(1)?,
        worker: r.get(2)?,
        what: r.get(3)?,
        pid: r.get::<_, Option<i64>>(4)?.and_then(|p| u32::try_from(p).ok()),
        log: r.get(5)?,
        done_pattern: r.get(6)?,
        stall_minutes: r.get::<_, i64>(7)? as u32,
        status: JobStatus::parse(&status),
        boot: r.get(9)?,
        registered_at: r.get(10)?,
        changed_at: r.get(11)?,
        ended_at: r.get(12)?,
        read_bytes: r.get::<_, i64>(13)? as u64,
        last_line: r.get(14)?,
        note: r.get(15)?,
        read_sum: r.get(16)?,
        pid_start: r.get::<_, Option<i64>>(17)?.map(|t| t as u64),
    })
}

/// How the app found its live runs when it came back: why they are not
/// live any more, when they were last known to be, and how to tell whether
/// an agent process is still there.
pub struct Interruption<'a> {
    /// For a human: what happened and when ("the machine rebooted at
    /// 23:34; last seen alive at 23:31").
    pub why: String,
    /// Unix milliseconds the app was last known alive, when known. That is
    /// the run's end as far as anyone can tell -- not the moment the app
    /// came back and noticed, which may be days later.
    pub at: Option<i64>,
    /// Whether the agent process with this id (and, when kept, this start
    /// time) is still alive. A run whose agent is goes `detached` rather
    /// than `interrupted`: it is not over, and must not be started again
    /// until that process is gone.
    pub alive: &'a dyn Fn(u32, Option<u64>) -> bool,
}

/// A run [`Store::close_live_runs`] dealt with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedRun {
    pub run: RunSummary,
    /// Its agent was still alive, so it went `detached` rather than
    /// `interrupted`.
    pub detached: bool,
}

/// A run of a routine that a comeback closed as failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedRoutineRun {
    pub run: String,
    pub routine: String,
    /// Its agent's pid when that agent was still alive: the routine waits
    /// for it before it runs again.
    pub left: Option<u32>,
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

/// A saved instruction, run on its own whenever it is wanted.
///
/// A routine is not a schedule, and it is not a worker. It is the work a
/// human would otherwise dictate a second time — and a third — written down
/// with everything it needs to run alone: what to do, the folder to do it
/// in, and the agent to do it on. Running one opens an agent session of its
/// own. It borrows no conversation, holds no place in a track's worker list,
/// and nothing it does reaches a worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    /// What the human calls it.
    pub name: String,
    /// What the agent is told. Written to stand alone: it is read again
    /// weeks later with none of the conversation that produced it.
    pub instruction: String,
    /// The folder it works in. Its own, not a track's.
    pub cwd: String,
    /// The agent it runs on. Its own, and required — there is no track to
    /// fall back to.
    pub agent: String,
    /// Its session options, `option id -> value id`, in the agent's own
    /// terms (`model`, `mode`, `reasoning_effort`, ...), as a track's and an
    /// artifact's are. Absent options keep the agent's default.
    pub config: BTreeMap<String, String>,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Unix milliseconds of its last run, or its creation.
    pub updated_at: i64,
    /// How many times it has run.
    pub runs: u32,
    /// Its most recent run, when it has one.
    pub last_run: Option<String>,
    /// When its most recent run started (Unix milliseconds), when it has one.
    /// Not `updated_at`, which an edit moves too.
    #[serde(default)]
    pub last_run_at: Option<i64>,
    /// Its plan: the id of an artifact of kind `plan`, or empty for none.
    /// Set by opening the plan, never by a patch.
    #[serde(default)]
    pub plan: String,
    /// `#rrggbb` for the bar beside it in its list, or empty.
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// What a routine change may touch; `None` keeps a field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutinePatch {
    pub name: Option<String>,
    pub instruction: Option<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    pub config: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

fn row_to_routine(r: &rusqlite::Row<'_>) -> rusqlite::Result<Routine> {
    Ok(Routine {
        id: r.get(0)?,
        name: r.get(1)?,
        instruction: r.get(2)?,
        cwd: plain(&r.get::<_, String>(3)?),
        agent: r.get(4)?,
        config: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        runs: r.get(8)?,
        last_run: r.get(9)?,
        plan: r.get(10)?,
        color: r.get(11)?,
        tags: serde_json::from_str(&r.get::<_, String>(12)?).unwrap_or_default(),
        last_run_at: r.get(13)?,
    })
}

/// Columns of a routine, in the order `row_to_routine` reads them. The run
/// count and the newest run are folded from the runs kept under its key.
const ROUTINE_SELECT: &str = "SELECT r.id, r.name, r.instruction, r.cwd, r.agent, r.config,
                                     r.created_at, r.updated_at,
                                     (SELECT COUNT(*) FROM routine_runs x WHERE x.routine = r.id),
                                     (SELECT x.id FROM routine_runs x WHERE x.routine = r.id
                                      ORDER BY x.started_at DESC LIMIT 1),
                                     r.plan, r.color, r.tags,
                                     (SELECT MAX(x.started_at) FROM routine_runs x WHERE x.routine = r.id)
                              FROM routines r";

/// Where one run of a routine stands.
///
/// Its own words, not a worker run's. A worker run ends `done` with a stop
/// reason the agent gives for every ordinary turn, which read as "stopped"
/// the moment it was shown to someone. Here, `Stopped` means a person
/// stopped it, and nothing else sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineStatus {
    /// The agent has the work and has not finished.
    Running,
    /// It finished by itself.
    Done,
    /// It could not finish: the agent failed, or the app closed under it.
    Failed,
    /// A person stopped it part-way.
    Stopped,
}

impl RoutineStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RoutineStatus::Running => "running",
            RoutineStatus::Done => "done",
            RoutineStatus::Failed => "failed",
            RoutineStatus::Stopped => "stopped",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "running" => RoutineStatus::Running,
            "done" => RoutineStatus::Done,
            "failed" => RoutineStatus::Failed,
            "stopped" => RoutineStatus::Stopped,
            _ => return None,
        })
    }
}

/// One run of a routine, whole: there is no second table to join for the
/// rest of it, and none of it is in `runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineRun {
    /// `rr` and sixteen hex digits. Never a worker run's `t001`: the two
    /// belong to different tables and must not look alike.
    pub id: String,
    pub routine: String,
    pub agent: String,
    pub cwd: String,
    /// The instruction as it read when this run was started.
    pub instruction: String,
    pub status: RoutineStatus,
    /// Unix milliseconds.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// What the agent said, its visible text.
    pub output: String,
    pub error: String,
    pub tools: u32,
    /// The checked report, when the run left one.
    pub report: Option<serde_json::Value>,
    /// The plan it was given beyond its instruction, as it read then.
    #[serde(default)]
    pub plan: Option<serde_json::Value>,
    /// What its agent said of each step of that plan.
    #[serde(default)]
    pub steps: Option<serde_json::Value>,
}

fn row_to_routine_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoutineRun> {
    let status: String = r.get(5)?;
    let report: String = r.get(11)?;
    let plan: String = r.get(12)?;
    let steps: String = r.get(13)?;
    Ok(RoutineRun {
        id: r.get(0)?,
        routine: r.get(1)?,
        agent: r.get(2)?,
        cwd: r.get(3)?,
        instruction: r.get(4)?,
        status: RoutineStatus::parse(&status).unwrap_or(RoutineStatus::Failed),
        started_at: r.get(6)?,
        ended_at: r.get(7)?,
        output: r.get(8)?,
        error: r.get(9)?,
        tools: r.get(10)?,
        report: serde_json::from_str(&report).ok(),
        plan: serde_json::from_str(&plan).ok(),
        steps: serde_json::from_str(&steps).ok(),
    })
}

const ROUTINE_RUN_SELECT: &str = "SELECT id, routine, agent, cwd, instruction, status, started_at,
                                         ended_at, output, error, tools, report, plan, steps
                                  FROM routine_runs";

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
    /// Where workers work: `subfolder` (a folder of their own under the
    /// track's), `worktree` (a git checkout of their own; the human merges)
    /// or `shared` (the track folder itself). See [`WORKER_FOLDERS`].
    pub worker_folder: String,
    /// Whether the conductor may put a worker on an agent or model other
    /// than this track's: `follow` (never; what every track did before this
    /// setting existed) or `propose` (it may ask, and the human approves on
    /// a decision card). See [`WORKER_CHOICES`].
    pub worker_choice: String,
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

    /// Whether the conductor may ask for a worker on something other than
    /// this track's agent and options. When it may not, a worker's agent is
    /// this track's, whoever opened it.
    pub fn proposes_worker_choice(&self) -> bool {
        self.worker_choice == "propose"
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
    pub worker_folder: Option<String>,
    pub worker_choice: Option<String>,
}

/// The ways a track's workers can work, the first the default.
pub const WORKER_FOLDERS: [&str; 3] = ["subfolder", "worktree", "shared"];

fn worker_folder(patch: &TrackPatch) -> anyhow::Result<Option<&str>> {
    match patch.worker_folder.as_deref() {
        None => Ok(None),
        Some(f) if WORKER_FOLDERS.contains(&f) => Ok(Some(f)),
        Some(f) => anyhow::bail!("worker_folder must be one of {WORKER_FOLDERS:?}, not {f:?}"),
    }
}

/// Who picks a worker's agent and model, the first the default.
///
/// `follow`: the track's own choice, always. `propose`: the conductor may
/// ask for something else, and the human approves it on a decision card.
pub const WORKER_CHOICES: [&str; 2] = ["follow", "propose"];

fn worker_choice(patch: &TrackPatch) -> anyhow::Result<Option<&str>> {
    match patch.worker_choice.as_deref() {
        None => Ok(None),
        Some(c) if WORKER_CHOICES.contains(&c) => Ok(Some(c)),
        Some(c) => anyhow::bail!("worker_choice must be one of {WORKER_CHOICES:?}, not {c:?}"),
    }
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
    /// Runs left live by a previous process are closed out as interrupted
    /// with [`INTERRUPTED`], so the timeline never shows a run that is
    /// running in a process that no longer exists. An app that knows more
    /// about how it went (a reboot, when it was last alive, whether the
    /// agents are still there) opens with [`Store::open_keeping_live`] and
    /// closes them itself with [`Store::close_live_runs`].
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn, true)
    }

    /// Open or create the database at `path`, leaving runs a previous
    /// process left live as they are, for the caller to close with
    /// [`Store::close_live_runs`] once it knows what became of them.
    pub fn open_keeping_live(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::init(Connection::open(path)?, false)
    }

    /// Open the database beside a process that may already have it open
    /// (`divixi-server token`, `owner`, `phone` … while the server runs).
    ///
    /// The same as [`Store::open`] except that live runs are left alone: they
    /// may be live in that other process, and closing them out as
    /// [`INTERRUPTED`] from here would fail runs that are still going.
    pub fn open_alongside(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::init(Connection::open(path)?, false)
    }

    /// An in-memory store, for tests and `DIVISI_DB=:memory:`.
    pub fn in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?, true)
    }

    /// One `meta` value out of a database file, touching nothing else: no
    /// migration, no closing of runs. For a command that has to look
    /// before it decides whether to open the store at all (a downgrade
    /// must not bring the database forward first).
    pub fn peek_meta(path: impl AsRef<Path>, key: &str) -> anyhow::Result<Option<String>> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let has_meta: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_meta {
            return Ok(None);
        }
        Ok(conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    fn init(mut conn: Connection, close_interrupted: bool) -> anyhow::Result<Self> {
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
        if close_interrupted {
            store.close_live_runs(&Interruption { why: INTERRUPTED.to_string(), at: None, alive: &|_, _| false })?;
        }
        Ok(store)
    }

    /// Take a database back to an earlier schema, for an older build to
    /// open. Refused when the version has no way back ([`DOWNGRADES`]),
    /// and when the database is not newer than asked. Each step and its
    /// version bump commit together, as going forward does.
    pub fn downgrade(path: impl AsRef<Path>, to: i64) -> anyhow::Result<i64> {
        let mut conn = Connection::open(path)?;
        let from: i64 = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("no schema version: not a divixi store"))?;
        if to >= from {
            anyhow::bail!("the store is at schema version {from}, which is not newer than {to}");
        }
        if from > SCHEMA_VERSION {
            anyhow::bail!("the store is at schema version {from}, newer than this build knows ({SCHEMA_VERSION})");
        }
        // Every step must have a way back before the first is taken, or a
        // refusal half-way would leave the database at a version nobody asked for.
        for v in (to + 1..=from).rev() {
            if DOWNGRADES.get((SCHEMA_VERSION - v) as usize).is_none() {
                anyhow::bail!("no way back from schema version {v}");
            }
        }
        for v in (to + 1..=from).rev() {
            let step = DOWNGRADES[(SCHEMA_VERSION - v) as usize];
            tracing::info!(from = v, to = v - 1, "taking the event store back");
            let tx = conn.transaction()?;
            tx.execute_batch(step)?;
            tx.execute("UPDATE meta SET value = ?1 WHERE key = 'schema_version'", params![(v - 1).to_string()])?;
            tx.commit()?;
        }
        Ok(to)
    }

    /// Read a free-form setting.
    pub fn get_meta(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    /// Remove a free-form setting.
    /// Set a key only if it is not set yet, in one step: whether this call
    /// set it. What two callers racing for the same thing use to tell
    /// which of them has it.
    pub fn claim_meta(&self, key: &str, value: &str) -> anyhow::Result<bool> {
        let changed = self.conn.lock().execute("INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(changed == 1)
    }

    pub fn delete_meta(&self, key: &str) -> anyhow::Result<()> {
        self.conn.lock().execute("DELETE FROM meta WHERE key = ?1", params![key])?;
        Ok(())
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

    /// Runs a previous process left live (`connecting`, `running`) or
    /// detached, oldest first, with the run-relative time of the last event
    /// each saw.
    fn live_runs_with_last_event(&self) -> anyhow::Result<Vec<(RunSummary, u64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{RUN_SELECT} WHERE status IN ('connecting', 'running', 'detached') ORDER BY n"
        ))?;
        let runs: Vec<RunSummary> = stmt.query_map([], row_to_summary)?.collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(runs.len());
        for run in runs {
            let at_ms: i64 = conn.query_row(
                "SELECT COALESCE(MAX(at_ms), 0) FROM events WHERE run_id = ?1",
                params![run.id],
                |r| r.get(0),
            )?;
            out.push((run, at_ms.max(0) as u64));
        }
        Ok(out)
    }

    /// Close the runs a previous process left live, now that the app knows
    /// what became of them: `interrupted` with `how.why`, ended when the app
    /// was last seen alive (`how.at`) -- or `detached` for a run whose agent
    /// process `how.alive` still finds, which is not over and is watched
    /// until it is. A run already detached from an earlier comeback is
    /// looked at the same way: still alive, it stays as it is (and is not
    /// returned); gone, it ends as interrupted like the rest. Routine runs
    /// left live are closed as failed with the same words. Returns what was
    /// done, for the app to tell people.
    pub fn close_live_runs(&self, how: &Interruption<'_>) -> anyhow::Result<Vec<ClosedRun>> {
        Ok(self.close_live_runs_and_routines(how)?.0)
    }

    /// [`Store::close_live_runs`], with the routine runs it closed as well,
    /// so a person can be told of those too.
    pub fn close_live_runs_and_routines(&self, how: &Interruption<'_>) -> anyhow::Result<(Vec<ClosedRun>, Vec<ClosedRoutineRun>)> {
        let mut out = Vec::new();
        for (run, at_ms) in self.live_runs_with_last_event()? {
            let detached = run.pid.is_some_and(|pid| (how.alive)(pid, run.pid_start));
            if detached && run.status == RunStatus::Detached {
                continue;
            }
            if detached {
                tracing::warn!(run = %run.id, pid = ?run.pid, "a run the app lost still has its agent alive; watching it");
                self.append(&run.id, at_ms, &AgentEvent::Detached { why: how.why.clone() })?;
            } else {
                tracing::warn!(run = %run.id, "closing run left live by a previous process");
                self.append_ending_at(&run.id, at_ms, &AgentEvent::Interrupted { why: how.why.clone() }, how.at)?;
            }
            let run = self.run(&run.id)?.unwrap_or(run);
            out.push(ClosedRun { run, detached });
        }
        let routines = self.close_interrupted_routine_runs(how)?;
        Ok((out, routines))
    }

    /// Runs whose agent went on alone after the app lost them, oldest first.
    pub fn detached_runs(&self) -> anyhow::Result<Vec<RunSummary>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{RUN_SELECT} WHERE status = 'detached' ORDER BY n"))?;
        let rows = stmt.query_map([], row_to_summary)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The agent process behind a run, once the app has one, and when it
    /// started.
    pub fn set_run_pid(&self, run: &str, pid: Option<u32>, started: Option<u64>) -> anyhow::Result<()> {
        self.conn.lock().execute("UPDATE runs SET pid = ?2, pid_start = ?3 WHERE id = ?1", params![run, pid.map(i64::from), started.map(|t| t as i64)])?;
        Ok(())
    }

    /// Whether a run may be started again by itself after an interruption.
    pub fn set_run_repeatable(&self, run: &str, repeatable: bool) -> anyhow::Result<()> {
        self.conn.lock().execute("UPDATE runs SET repeatable = ?2 WHERE id = ?1", params![run, repeatable as i64])?;
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
                                    conductor_config, worker_agent, worker_config, color, tags, worker_folder,
                                    worker_choice)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                    worker_folder(patch)?.unwrap_or(WORKER_FOLDERS[0]),
                    worker_choice(patch)?.unwrap_or(WORKER_CHOICES[0]),
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

    // ── Routines ──

    /// Every routine, most recently touched first.
    pub fn routines(&self) -> anyhow::Result<Vec<Routine>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{ROUTINE_SELECT} ORDER BY r.updated_at DESC, r.id DESC"))?;
        let rows = stmt.query_map([], row_to_routine)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One routine by id.
    pub fn routine(&self, id: &str) -> anyhow::Result<Option<Routine>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("{ROUTINE_SELECT} WHERE r.id = ?1"), params![id], row_to_routine)
            .optional()?)
    }

    /// Save a routine. It needs everything it takes to run alone: a name, an
    /// instruction, a folder and an agent. A track is optional — it only
    /// decides who is told afterwards.
    pub fn create_routine(&self, patch: &RoutinePatch) -> anyhow::Result<Routine> {
        let name = patch.name.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let instruction = patch.instruction.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let cwd = patch.cwd.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let agent = patch.agent.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let (Some(name), Some(instruction), Some(cwd), Some(agent)) = (name, instruction, cwd, agent) else {
            anyhow::bail!("a routine needs a name, an instruction, a folder and an agent");
        };
        let config = serde_json::to_string(&patch.config.clone().unwrap_or_default())?;
        let id = {
            let conn = self.conn.lock();
            let next: i64 = conn.query_row(
                "SELECT COALESCE(MAX(CAST(substr(id, 3) AS INTEGER)), 0) + 1 FROM routines",
                [],
                |r| r.get(0),
            )?;
            let id = format!("ro{next:03}");
            let now = now_ms();
            conn.execute(
                "INSERT INTO routines(id, name, instruction, cwd, agent, config, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, name, instruction, cwd, agent, config, now],
            )?;
            id
        };
        Ok(self.routine(&id)?.expect("just written"))
    }

    /// Change a routine's fields; `None` in the patch keeps a field.
    pub fn update_routine(&self, id: &str, patch: &RoutinePatch) -> anyhow::Result<Routine> {
        let config = patch.config.as_ref().map(serde_json::to_string).transpose()?;
        let tags = patch.tags.as_ref().map(serde_json::to_string).transpose()?;
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE routines SET name = COALESCE(?2, name), instruction = COALESCE(?3, instruction),
                 cwd = COALESCE(?4, cwd), agent = COALESCE(?5, agent), config = COALESCE(?6, config),
                 color = COALESCE(?8, color), tags = COALESCE(?9, tags),
                 updated_at = ?7
                 WHERE id = ?1",
                params![
                    id,
                    patch.name.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                    patch.instruction.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                    patch.cwd.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                    patch.agent.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                    config,
                    now_ms(),
                    patch.color.as_deref().map(str::trim),
                    tags,
                ],
            )?;
            if changed == 0 {
                anyhow::bail!("no routine {id}");
            }
        }
        // Deleted in between (another window, the phone): said, not a panic.
        self.routine(id)?.ok_or_else(|| anyhow::anyhow!("no routine {id}"))
    }

    /// Link a routine to its plan (an artifact of kind `plan`), or unlink it
    /// with an empty id. Not a patch field, and it leaves `updated_at`
    /// alone: the list is ordered by when a routine last ran or changed,
    /// and opening its plan is neither.
    pub fn set_routine_plan(&self, id: &str, plan: &str) -> anyhow::Result<Routine> {
        {
            let conn = self.conn.lock();
            let changed = conn.execute("UPDATE routines SET plan = ?2 WHERE id = ?1", params![id, plan])?;
            if changed == 0 {
                anyhow::bail!("no routine {id}");
            }
        }
        // Deleted in between (another window, the phone): said, not a panic.
        self.routine(id)?.ok_or_else(|| anyhow::anyhow!("no routine {id}"))
    }

    /// Remove a routine and everything its runs left behind. Unlike a
    /// worker's, a routine's runs belong to no track, so nothing else is
    /// left holding them.
    pub fn delete_routine(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM routine_runs WHERE routine = ?1", params![id])?;
        let changed = tx.execute("DELETE FROM routines WHERE id = ?1", params![id])?;
        if changed == 0 {
            anyhow::bail!("no routine {id}");
        }
        tx.commit()?;
        Ok(())
    }

    /// A routine's runs, newest first.
    pub fn routine_runs(&self, id: &str, limit: u32) -> anyhow::Result<Vec<RoutineRun>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(&format!("{ROUTINE_RUN_SELECT} WHERE routine = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2"))?;
        let rows = stmt.query_map(params![id, limit], row_to_routine_run)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One run of a routine by id.
    pub fn routine_run(&self, id: &str) -> anyhow::Result<Option<RoutineRun>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("{ROUTINE_RUN_SELECT} WHERE id = ?1"), params![id], row_to_routine_run)
            .optional()?)
    }

    /// Open a run of a routine and return its id.
    ///
    /// The id is random rather than the next number: these are a routine's
    /// own, nothing reads them in order, and they must never be mistaken for
    /// a worker run's `t001` — which is exactly what happened while the two
    /// shared a table.
    pub fn begin_routine_run(&self, routine: &str, agent: &str, cwd: &str, instruction: &str) -> anyhow::Result<String> {
        let id = format!("rr{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO routine_runs(id, routine, agent, cwd, instruction, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
            params![id, routine, agent, cwd, instruction, now_ms()],
        )?;
        Ok(id)
    }

    /// Close a run of a routine with what it left behind.
    pub fn end_routine_run(
        &self,
        id: &str,
        status: RoutineStatus,
        output: &str,
        error: &str,
        tools: u32,
        report: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let report = report.map(serde_json::to_string).transpose()?.unwrap_or_default();
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE routine_runs SET status = ?2, ended_at = ?3, output = ?4, error = ?5, tools = ?6, report = ?7, pid = NULL, pid_start = NULL
             WHERE id = ?1",
            params![id, status.as_str(), now_ms(), output, error, tools, report],
        )?;
        conn.execute(
            "UPDATE routines SET updated_at = ?2 WHERE id = (SELECT routine FROM routine_runs WHERE id = ?1)",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    /// Keep the plan a run of a routine was given beyond its instruction, as
    /// it read then. Like the instruction, it is the run's own copy: the plan
    /// changing afterwards does not rewrite what this run was asked to do.
    pub fn set_routine_run_plan(&self, id: &str, plan: &serde_json::Value) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        if conn.execute("UPDATE routine_runs SET plan = ?2 WHERE id = ?1", params![id, serde_json::to_string(plan)?])? == 0 {
            anyhow::bail!("no routine run {id}");
        }
        Ok(())
    }

    /// Keep what a run's agent said of each step of its plan.
    pub fn set_routine_run_steps(&self, id: &str, steps: &serde_json::Value) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        if conn.execute("UPDATE routine_runs SET steps = ?2 WHERE id = ?1", params![id, serde_json::to_string(steps)?])? == 0 {
            anyhow::bail!("no routine run {id}");
        }
        Ok(())
    }

    /// Close runs of routines that were live when the app last closed, as
    /// [`Store::close_live_runs`] does for a track's: failed, with the same
    /// words, ended when the app was last seen alive. A run whose agent is
    /// still alive keeps its pid, so the routine is not started again beside
    /// it ([`Store::routine_left_running`]); a pid kept that way from an
    /// earlier comeback is let go once its agent has gone.
    fn close_interrupted_routine_runs(&self, how: &Interruption<'_>) -> anyhow::Result<Vec<ClosedRoutineRun>> {
        let conn = self.conn.lock();
        type Row = (String, Option<i64>, Option<i64>);
        let read = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?)) };
        let lives = |pid: Option<i64>, start: Option<i64>| pid.is_some_and(|p| (how.alive)(p as u32, start.map(|t| t as u64)));
        let kept: Vec<Row> = conn
            .prepare("SELECT id, pid, pid_start FROM routine_runs WHERE pid IS NOT NULL AND status <> 'running'")?
            .query_map([], read)?
            .collect::<Result<_, _>>()?;
        for (id, pid, start) in kept {
            if !lives(pid, start) {
                conn.execute("UPDATE routine_runs SET pid = NULL, pid_start = NULL WHERE id = ?1", params![id])?;
            }
        }
        let live: Vec<(String, Option<i64>, Option<i64>, String)> = conn
            .prepare("SELECT id, pid, pid_start, routine FROM routine_runs WHERE status = 'running'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let mut closed = Vec::new();
        for (id, pid, start, routine) in live {
            let still = lives(pid, start);
            conn.execute(
                "UPDATE routine_runs SET status = 'failed', ended_at = COALESCE(ended_at, MAX(started_at, COALESCE(?2, started_at))), error = ?1,
                        pid = CASE WHEN ?4 THEN pid END, pid_start = CASE WHEN ?4 THEN pid_start END
                 WHERE id = ?3",
                params![how.why, how.at, id, still],
            )?;
            closed.push(ClosedRoutineRun { run: id, routine, left: if still { pid.map(|p| p as u32) } else { None } });
        }
        Ok(closed)
    }

    /// Keep the process a routine's run is running in, and when it started.
    pub fn set_routine_run_pid(&self, id: &str, pid: Option<u32>, started: Option<u64>) -> anyhow::Result<()> {
        self.conn.lock().execute("UPDATE routine_runs SET pid = ?2, pid_start = ?3 WHERE id = ?1", params![id, pid, started.map(|t| t as i64)])?;
        Ok(())
    }

    /// A run of this routine that a crash cut while its agent went on,
    /// with that agent's pid and start, if there is one: the caller asks
    /// whether it is alive still before starting the routine again.
    pub fn routine_left_running(&self, routine: &str) -> anyhow::Result<Option<(String, u32, Option<u64>)>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT id, pid, pid_start FROM routine_runs WHERE routine = ?1 AND pid IS NOT NULL AND status <> 'running' ORDER BY started_at DESC LIMIT 1",
                params![routine],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)),
            )
            .optional()?;
        Ok(row.map(|(id, pid, start)| (id, pid as u32, start.map(|t| t as u64))))
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
                        color = COALESCE(?9, color), tags = COALESCE(?10, tags),
                        worker_folder = COALESCE(?11, worker_folder),
                        worker_choice = COALESCE(?12, worker_choice)
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
                    worker_folder(patch)?,
                    worker_choice(patch)?,
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
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'worker_session:' || ?1 || '/%' OR key LIKE 'worktree:' || ?1 || '/%' OR key LIKE 'worker_dir:' || ?1 || '/%' OR key = 'handoff:' || ?1",
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
        // What outlived its sessions does not outlive its track: a schedule
        // left behind would go on waking a conductor that has nothing to
        // conduct, a watched job would be reported to it, a notice would
        // point at a track that is gone.
        tx.execute("DELETE FROM schedules WHERE track = ?1", params![id])?;
        tx.execute("DELETE FROM jobs WHERE track = ?1", params![id])?;
        tx.execute("DELETE FROM notices WHERE track = ?1", params![id])?;
        tx.execute(
            "DELETE FROM meta WHERE key LIKE 'conductor_session:' || ?1 || ':%' OR key LIKE 'worker_session:' || ?1 || '/%' OR key LIKE 'worktree:' || ?1 || '/%' OR key LIKE 'worker_dir:' || ?1 || '/%' OR key = 'handoff:' || ?1
                 OR key = 'notes:' || ?1 OR key = 'quiet:' || ?1 OR key LIKE 'worker_scope:' || ?1 || '/%' OR key = 'parked:' || ?1",
            params![id],
        )?;
        let changed = tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
        if changed == 0 {
            anyhow::bail!("no track {id}");
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete one worker's record in a track: its runs with their events,
    /// full-text rows and kept reports, and the agent session the app
    /// remembers for it. Returns how many runs went.
    ///
    /// What is deliberately left: the worker's folder (`worker_dir:`) and its
    /// checkout (`worktree:`), on disk and in the record. Those hold work the
    /// human made and asked for; forgetting the mapping would strand them or
    /// send the next worker of that name somewhere else.
    pub fn delete_worker(&self, track: &str, worker: &str) -> anyhow::Result<u32> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM runs_fts WHERE run_id IN (SELECT id FROM runs WHERE track = ?1 AND session = ?2)",
            params![track, worker],
        )?;
        tx.execute(
            "DELETE FROM meta WHERE key IN (SELECT 'report:' || id FROM runs WHERE track = ?1 AND session = ?2)",
            params![track, worker],
        )?;
        tx.execute(
            "DELETE FROM events WHERE run_id IN (SELECT id FROM runs WHERE track = ?1 AND session = ?2)",
            params![track, worker],
        )?;
        // Nothing puts a decision on a worker run today — every card hangs
        // off the conductor's turn or an artifact's, even the permission
        // questions a worker raises. This is here so that stays true by
        // construction rather than by reading three call sites: a card
        // pointing at a deleted run could never be answered, and the bell
        // would keep offering it. Scoped to this worker's runs, so the
        // conductor's own cards are untouched.
        tx.execute(
            "DELETE FROM decisions WHERE track = ?1 AND run IN (SELECT id FROM runs WHERE track = ?1 AND session = ?2)",
            params![track, worker],
        )?;
        let gone = tx.execute("DELETE FROM runs WHERE track = ?1 AND session = ?2", params![track, worker])?;
        tx.execute("DELETE FROM meta WHERE key = 'worker_session:' || ?1 || '/' || ?2 OR key = 'worker_scope:' || ?1 || '/' || ?2", params![track, worker])?;
        tx.commit()?;
        Ok(gone as u32)
    }

    /// Register a new run in a track and return its id.
    ///
    /// Ids are `t001`, `t002`, … in creation order, durable across restarts.
    ///
    /// The counter is the high-water mark of every run there has ever been,
    /// not of the rows left: deleting a track's or a worker's runs must not
    /// hand their ids to later runs, or what still points at an id (a kept
    /// report, a decision) would point at a stranger.
    pub fn begin_run(&self, track: &str, session: &str, agent: &str, prompt: &str, cwd: &str) -> anyhow::Result<RunId> {
        let conn = self.conn.lock();
        let highest: i64 = conn.query_row("SELECT COALESCE(MAX(n), 0) FROM runs", [], |r| r.get(0))?;
        let ever: i64 = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![RUN_HIGH_WATER], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let next = highest.max(ever) + 1;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![RUN_HIGH_WATER, next.to_string()],
        )?;
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
        self.append_ending_at(run, at_ms, event, None)
    }

    /// [`Store::append`], with the moment a terminal event ends the run
    /// given rather than taken from the clock: a run the app lost ended
    /// when the app was last seen alive, not when it came back and noticed.
    fn append_ending_at(&self, run: &str, at_ms: u64, event: &AgentEvent, ended_at: Option<i64>) -> anyhow::Result<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let payload = serde_json::to_string(event)?;
        tx.execute(
            "INSERT INTO events(run_id, at_ms, kind, above, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run, at_ms as i64, event.kind(), event.above_membrane() as i64, payload],
        )?;
        let seq = tx.last_insert_rowid();

        // When the run ended, and how long it ran. Given an end, the
        // length is measured to it; otherwise it is the time of the event.
        // A given end is never before the start: the app's last sign of
        // life can predate a run that began just after it, and a run that
        // ended before it began would be read as having never run.
        let (ended, duration): (i64, i64) = match ended_at {
            Some(end) => {
                let started: i64 = tx.query_row("SELECT started_at FROM runs WHERE id = ?1", params![run], |r| r.get(0)).optional()?.unwrap_or(end);
                let end = end.max(started);
                (end, end - started)
            }
            None => (now_ms(), at_ms as i64),
        };
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
                     WHERE id = ?1 AND status IN ('connecting', 'running', 'detached')",
                    params![run, RunStatus::Done.as_str(), stop_reason, ended, duration],
                )?;
                if changed > 0 {
                    fold(&tx, run)?;
                }
            }
            // The three ways a turn ends without finishing, each its own
            // status with its reason in `error`; a detached run ends too,
            // once its agent is gone.
            AgentEvent::Failed { error: why } | AgentEvent::Interrupted { why } | AgentEvent::TimedOut { why } => {
                let status = event.ends_as().unwrap_or(RunStatus::Failed);
                let changed = tx.execute(
                    "UPDATE runs SET status = ?2, error = ?3, ended_at = ?4, duration_ms = ?5
                     WHERE id = ?1 AND status IN ('connecting', 'running', 'detached')",
                    params![run, status.as_str(), why, ended, duration],
                )?;
                if changed > 0 {
                    fold(&tx, run)?;
                }
            }
            // Not an end: the agent is still going, out of reach. No fold
            // (there is nothing final to index) and no end time.
            AgentEvent::Detached { why } => {
                tx.execute(
                    "UPDATE runs SET status = ?2, error = ?3 WHERE id = ?1 AND status IN ('connecting', 'running')",
                    params![run, RunStatus::Detached.as_str(), why],
                )?;
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
    /// The answer lands only if the card is still open, and that is decided
    /// by the `UPDATE` itself (as [`Self::dismiss_decision`] does), not by a
    /// read before it. Reading first and writing after leaves a gap two
    /// answers can both pass — a double click, or two devices on the same
    /// Divixi — and each would hand the conductor its own turn for one card.
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
        // The check that counts. The read above only works out what the
        // answer means; this is what makes it the one answer.
        let changed = conn.execute(
            "UPDATE decisions SET status = ?2, choice = ?3, answer = ?4, note = ?5, decided_at = ?6 WHERE id = ?1 AND status = 'open'",
            params![id, DecisionStatus::Decided.as_str(), choice.map(|i| i as i64), answer, note.trim(), now_ms()],
        )?;
        drop(conn);
        if changed == 0 {
            anyhow::bail!("decision {id} was answered or set aside already");
        }
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

    // ── Notices ──

    /// Keep something the human has to be told. It stays until read.
    pub fn add_notice(&self, new: &NewNotice) -> anyhow::Result<Notice> {
        if new.title.trim().is_empty() {
            anyhow::bail!("a notice needs a title");
        }
        let id = {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO notices(track, kind, title, body, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![new.track, new.kind, new.title.trim(), new.body.trim(), serde_json::to_string(&new.data)?, now_ms()],
            )?;
            conn.last_insert_rowid()
        };
        self.notice(id)?.ok_or_else(|| anyhow::anyhow!("notice {id} vanished"))
    }

    pub fn notice(&self, id: i64) -> anyhow::Result<Option<Notice>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{NOTICE_SELECT} WHERE id = ?1"), params![id], row_to_notice).optional()?)
    }

    /// Notices not read yet, newest first.
    pub fn unread_notices(&self) -> anyhow::Result<Vec<Notice>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("{NOTICE_SELECT} WHERE read_at IS NULL ORDER BY created_at DESC, id DESC"))?;
        let rows = stmt.query_map([], row_to_notice)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The human has read it. Reading twice is nothing.
    pub fn read_notice(&self, id: i64) -> anyhow::Result<Notice> {
        self.conn.lock().execute("UPDATE notices SET read_at = COALESCE(read_at, ?2) WHERE id = ?1", params![id, now_ms()])?;
        self.notice(id)?.ok_or_else(|| anyhow::anyhow!("no notice {id}"))
    }

    /// Whether an unread notice of this kind is up for a track already, so
    /// the same thing is not said twice while nobody has read it once.
    pub fn has_unread_notice(&self, track: &str, kind: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notices WHERE track = ?1 AND kind = ?2 AND read_at IS NULL",
            params![track, kind],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    // ── Schedules ──

    pub fn create_schedule(&self, new: &NewSchedule) -> anyhow::Result<Schedule> {
        if new.message.trim().is_empty() {
            anyhow::bail!("a schedule needs a message for the conductor");
        }
        let id = {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO schedules(track, kind, spec, message, next_at, created_at, created_in) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![new.track, new.kind, serde_json::to_string(&new.spec)?, new.message.trim(), new.next_at, now_ms(), new.created_in],
            )?;
            conn.last_insert_rowid()
        };
        self.schedule(id)?.ok_or_else(|| anyhow::anyhow!("schedule {id} vanished"))
    }

    pub fn schedule(&self, id: i64) -> anyhow::Result<Option<Schedule>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{SCHEDULE_SELECT} WHERE id = ?1"), params![id], row_to_schedule).optional()?)
    }

    /// A track's schedules, or every track's; the active ones only, or all
    /// of them. Oldest first.
    pub fn schedules(&self, track: Option<&str>, active_only: bool) -> anyhow::Result<Vec<Schedule>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{SCHEDULE_SELECT} WHERE (?1 IS NULL OR track = ?1) AND (?2 = 0 OR status = 'active') ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![track, active_only as i64], row_to_schedule)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Active schedules whose time has come, oldest first. Conditions
    /// (no `next_at`) are not here: the app looks at those itself.
    pub fn due_schedules(&self, now: i64) -> anyhow::Result<Vec<Schedule>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{SCHEDULE_SELECT} WHERE status = 'active' AND next_at IS NOT NULL AND next_at <= ?1 ORDER BY next_at, id"
        ))?;
        let rows = stmt.query_map(params![now], row_to_schedule)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// It fired: count it, note what happened, and set the next time -- or
    /// none, which is the end of it.
    pub fn schedule_fired(&self, id: i64, next_at: Option<i64>, note: &str) -> anyhow::Result<Schedule> {
        let status = if next_at.is_some() { ScheduleStatus::Active } else { ScheduleStatus::Done };
        let changed = self.conn.lock().execute(
            "UPDATE schedules SET fired = fired + 1, last_at = ?2, next_at = ?3, status = ?4, note = ?5 WHERE id = ?1 AND status = 'active'",
            params![id, now_ms(), next_at, status.as_str(), note.trim()],
        )?;
        if changed == 0 {
            anyhow::bail!("schedule {id} is not active");
        }
        self.schedule(id)?.ok_or_else(|| anyhow::anyhow!("schedule {id} vanished"))
    }

    /// Move an active schedule's next time without counting a firing (it
    /// was pushed past quiet hours, say), with a note saying why.
    pub fn schedule_moved(&self, id: i64, next_at: i64, note: &str) -> anyhow::Result<Schedule> {
        let changed = self.conn.lock().execute(
            "UPDATE schedules SET next_at = ?2, note = ?3 WHERE id = ?1 AND status = 'active'",
            params![id, next_at, note.trim()],
        )?;
        if changed == 0 {
            anyhow::bail!("schedule {id} is not active");
        }
        self.schedule(id)?.ok_or_else(|| anyhow::anyhow!("schedule {id} vanished"))
    }

    /// How far into its file a `file` schedule has looked, with a
    /// fingerprint of the bytes just before that point.
    pub fn schedule_seen(&self, id: i64, seen: u64, sum: Option<i64>) -> anyhow::Result<()> {
        self.conn.lock().execute("UPDATE schedules SET seen = ?2, seen_sum = ?3 WHERE id = ?1 AND status = 'active'", params![id, seen as i64, sum])?;
        Ok(())
    }

    /// An active schedule is over without a firing (its `until` passed
    /// while it was held back), with a note saying so.
    pub fn end_schedule(&self, id: i64, note: &str) -> anyhow::Result<Schedule> {
        let changed = self.conn.lock().execute(
            "UPDATE schedules SET status = 'done', next_at = NULL, note = ?2 WHERE id = ?1 AND status = 'active'",
            params![id, note.trim()],
        )?;
        if changed == 0 {
            anyhow::bail!("schedule {id} is not active");
        }
        self.schedule(id)?.ok_or_else(|| anyhow::anyhow!("schedule {id} vanished"))
    }

    pub fn cancel_schedule(&self, id: i64) -> anyhow::Result<Schedule> {
        let changed = self.conn.lock().execute(
            "UPDATE schedules SET status = 'cancelled', next_at = NULL WHERE id = ?1 AND status = 'active'",
            params![id],
        )?;
        if changed == 0 {
            anyhow::bail!("schedule {id} is not active");
        }
        self.schedule(id)?.ok_or_else(|| anyhow::anyhow!("schedule {id} vanished"))
    }

    // ── Jobs ──

    pub fn add_job(&self, new: &NewJob) -> anyhow::Result<Job> {
        if new.pid.is_none() && new.log.trim().is_empty() {
            anyhow::bail!("a job needs a pid or a log file to watch");
        }
        let id = {
            let conn = self.conn.lock();
            let now = now_ms();
            conn.execute(
                "INSERT INTO jobs(track, worker, what, pid, log, done_pattern, stall_minutes, boot, registered_at, changed_at, pid_start)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
                params![
                    new.track,
                    new.worker,
                    new.what.trim(),
                    new.pid.map(i64::from),
                    new.log.trim(),
                    new.done_pattern.trim(),
                    i64::from(new.stall_minutes.max(1)),
                    new.boot,
                    now,
                    new.pid_start.map(|t| t as i64)
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.job(id)?.ok_or_else(|| anyhow::anyhow!("job {id} vanished"))
    }

    pub fn job(&self, id: i64) -> anyhow::Result<Option<Job>> {
        let conn = self.conn.lock();
        Ok(conn.query_row(&format!("{JOB_SELECT} WHERE id = ?1"), params![id], row_to_job).optional()?)
    }

    /// A track's jobs, or every track's; the ones still watched only, or
    /// all of them. Oldest first.
    pub fn jobs(&self, track: Option<&str>, open_only: bool) -> anyhow::Result<Vec<Job>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "{JOB_SELECT} WHERE (?1 IS NULL OR track = ?1) AND (?2 = 0 OR status IN ('running', 'stalled')) ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![track, open_only as i64], row_to_job)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The log moved (or did not): how far it has been read and its last
    /// line; `moved` is what resets the stall clock.
    pub fn job_progress(&self, id: i64, read_bytes: u64, read_sum: Option<i64>, last_line: &str, moved: bool) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        if moved {
            conn.execute(
                "UPDATE jobs SET read_bytes = ?2, read_sum = ?5, last_line = ?3, changed_at = ?4, status = CASE WHEN status = 'stalled' THEN 'running' ELSE status END WHERE id = ?1",
                params![id, read_bytes as i64, last_line, now_ms(), read_sum],
            )?;
        } else {
            conn.execute("UPDATE jobs SET read_bytes = ?2, read_sum = ?4, last_line = ?3 WHERE id = ?1", params![id, read_bytes as i64, last_line, read_sum])?;
        }
        Ok(())
    }

    /// Where an open job stands now, with what happened in a human's
    /// words. An end sets the end time. A job that is over already (it
    /// ended, or somebody stopped watching it while a look was under way)
    /// is left as it is and this is an error: the first ending stands, and
    /// nobody is told of a job they stopped watching.
    pub fn set_job_status(&self, id: i64, status: JobStatus, note: &str) -> anyhow::Result<Job> {
        let ended = (!status.is_open()).then(now_ms);
        let changed = self.conn.lock().execute(
            "UPDATE jobs SET status = ?2, note = ?3, ended_at = COALESCE(ended_at, ?4), changed_at = ?5 WHERE id = ?1 AND status IN ('running', 'stalled')",
            params![id, status.as_str(), note.trim(), ended, now_ms()],
        )?;
        if changed == 0 {
            anyhow::bail!("job {id} is not being watched (it has ended, or nobody watches it any more)");
        }
        self.job(id)?.ok_or_else(|| anyhow::anyhow!("job {id} vanished"))
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
    let mut plan: Vec<String> = Vec::new();
    // (id, title): an update can rename a call after it started.
    let mut tools: Vec<(String, String)> = Vec::new();

    let mut stmt = tx.prepare(
        "SELECT kind, payload FROM events
         WHERE run_id = ?1 AND kind IN ('message', 'plan', 'tool_call', 'tool_update') ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![run], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    // Put together as a routine's reply is: see `Reply`.
    let mut reply = orchestra_core::Reply::default();
    for row in rows {
        let (kind, payload) = row?;
        let value: serde_json::Value = serde_json::from_str(&payload)?;
        match kind.as_str() {
            "message" => reply.say(value["text"].as_str().unwrap_or_default()),
            "plan" => {
                plan = value["entries"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
            }
            "tool_call" => {
                reply.tool();
                tools.push((value["id"].as_str().unwrap_or_default().to_owned(), value["title"].as_str().unwrap_or_default().to_owned()))
            }
            "tool_update" => {
                let (id, title) = (value["id"].as_str().unwrap_or_default(), value["title"].as_str().unwrap_or_default());
                if let Some(tool) = tools.iter_mut().find(|(t, _)| !title.is_empty() && t == id) {
                    tool.1 = title.to_owned();
                }
            }
            _ => {}
        }
    }
    drop(stmt);
    let output = reply.into_string();

    tx.execute(
        "UPDATE runs SET output = ?2, plan = ?3, tool_count = ?4 WHERE id = ?1",
        params![run, output, serde_json::to_string(&plan)?, tools.len() as i64],
    )?;

    let prompt: String = tx.query_row("SELECT prompt FROM runs WHERE id = ?1", params![run], |r| r.get(0))?;
    tx.execute(
        "INSERT INTO runs_fts(run_id, prompt, output, tools) VALUES (?1, ?2, ?3, ?4)",
        params![run, prompt, output, tools.iter().map(|(_, title)| title.as_str()).collect::<Vec<_>>().join("\n")],
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
                                 stop_reason, error, output, plan, tool_count, agent, track, ended_at, pid, repeatable, pid_start
                          FROM runs";

/// Columns of a track, in the order `row_to_track` reads them.
const TRACK_SELECT: &str = "SELECT t.id, t.name, t.intent, t.cwd, t.agent, t.created_at, t.updated_at,
                                   (SELECT COUNT(*) FROM runs r WHERE r.track = t.id),
                                   t.conductor_config, t.worker_agent, t.worker_config, t.color, t.tags,
                                   t.worker_folder, t.worker_choice
                            FROM tracks t";

fn row_to_track(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackInfo> {
    let conductor: String = r.get(8)?;
    let worker: String = r.get(10)?;
    let tags: String = r.get(12)?;
    Ok(TrackInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        intent: r.get(2)?,
        // Older builds could keep `\\?\C:\…` (the remote folder picker);
        // it leaves as `C:\…`, the form shells and agents take.
        cwd: plain(&r.get::<_, String>(3)?),
        agent: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        runs: r.get::<_, i64>(7)? as u32,
        conductor_config: serde_json::from_str(&conductor).unwrap_or_default(),
        worker_agent: r.get(9)?,
        worker_config: serde_json::from_str(&worker).unwrap_or_default(),
        color: r.get(11)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        worker_folder: r.get(13)?,
        worker_choice: r.get(14)?,
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
        ended_at: r.get(15)?,
        pid: r.get::<_, Option<i64>>(16)?.and_then(|p| u32::try_from(p).ok()),
        repeatable: r.get::<_, i64>(17)? != 0,
        pid_start: r.get::<_, Option<i64>>(18)?.map(|t| t as u64),
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
            // What v12 did not have yet.
            store
                .conn
                .lock()
                .execute_batch(
                    "ALTER TABLE tracks DROP COLUMN worker_folder;
                     ALTER TABLE tracks DROP COLUMN worker_choice;
                     ALTER TABLE runs DROP COLUMN pid; ALTER TABLE runs DROP COLUMN pid_start; ALTER TABLE runs DROP COLUMN repeatable;
                     DROP TABLE notices; DROP TABLE schedules; DROP TABLE jobs;
                     DROP TABLE routines;",
                )
                .unwrap();
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
        assert_eq!(a.worker_folder, "subfolder", "workers get folders of their own by default");
        assert!(store.update_track(&a.id, &TrackPatch { worker_folder: Some("elsewhere".into()), ..TrackPatch::default() }).is_err());
        let w = store.update_track(&a.id, &TrackPatch { worker_folder: Some("worktree".into()), ..TrackPatch::default() }).unwrap();
        assert_eq!(w.worker_folder, "worktree");
        store.update_track(&a.id, &TrackPatch { worker_folder: Some("subfolder".into()), ..TrackPatch::default() }).unwrap();
        assert_eq!(a.worker_choice, "follow", "a track keeps its own agent for workers by default");
        assert!(!a.proposes_worker_choice());
        assert!(store.update_track(&a.id, &TrackPatch { worker_choice: Some("whatever".into()), ..TrackPatch::default() }).is_err());
        let p = store.update_track(&a.id, &TrackPatch { worker_choice: Some("propose".into()), ..TrackPatch::default() }).unwrap();
        assert!(p.proposes_worker_choice(), "the conductor may ask once the human turns it on");
        store.update_track(&a.id, &TrackPatch { worker_choice: Some("follow".into()), ..TrackPatch::default() }).unwrap();
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

    #[test]
    fn deleting_a_worker_takes_its_runs_but_not_its_folder() {
        let store = Store::in_memory().unwrap();
        let track = store.create_track(&new_track("T", "", ".", "claude_code")).unwrap().id;
        let conductor = store.begin_run(&track, "conductor", "claude_code", "hi", ".").unwrap();
        let first = store.begin_run(&track, "ui", "claude_code", "do it", ".").unwrap();
        store.append(&first, 1, &AgentEvent::Message { text: "done".into() }).unwrap();
        store.append(&first, 2, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
        let second = store.begin_run(&track, "api", "claude_code", "and this", ".").unwrap();
        store.set_meta(&format!("report:{first}"), "{}").unwrap();
        store.set_meta(&format!("report:{second}"), "{}").unwrap();
        store.set_meta(&format!("worker_session:{track}/ui"), "{}").unwrap();
        store.set_meta(&format!("worker_dir:{track}/ui"), "C:/work/ui").unwrap();
        store.set_meta(&format!("worktree:{track}/ui"), "{}").unwrap();

        assert_eq!(store.delete_worker(&track, "ui").unwrap(), 1);
        assert!(store.run(&first).unwrap().is_none(), "its runs go");
        assert!(store.events(&first).unwrap().is_empty(), "with their events");
        assert_eq!(store.get_meta(&format!("report:{first}")).unwrap(), None, "and their kept reports");
        assert_eq!(store.get_meta(&format!("worker_session:{track}/ui")).unwrap(), None, "and the session it would resume");
        assert_eq!(
            store.get_meta(&format!("worker_dir:{track}/ui")).unwrap().as_deref(),
            Some("C:/work/ui"),
            "the folder it worked in stays, files and all"
        );
        assert!(store.get_meta(&format!("worktree:{track}/ui")).unwrap().is_some(), "so does its checkout");
        assert!(store.run(&conductor).unwrap().is_some(), "other sessions are untouched");
        assert!(store.run(&second).unwrap().is_some());
        assert_eq!(store.get_meta(&format!("report:{second}")).unwrap().as_deref(), Some("{}"));
        assert!(!store.sessions(&track).unwrap().iter().any(|s| s.name == "ui"));

        // A new run never takes a deleted run's id.
        let next = store.begin_run(&track, "ui", "claude_code", "again", ".").unwrap();
        assert_ne!(next, first);
        assert_ne!(next, second);
    }

    #[test]
    fn a_card_takes_one_answer_however_many_arrive() {
        // Two devices on the same Divixi, or one double click: both used to
        // pass the read-then-write check and the conductor got two turns
        // for one card.
        let store = Store::in_memory().unwrap();
        let track = store.create_track(&new_track("T", "", ".", "claude_code")).unwrap().id;
        let card = store
            .open_decision(&NewDecision {
                track: track.clone(),
                run: None,
                question: "Which way?".into(),
                context: String::new(),
                options: vec![opt("left"), opt("right")],
                recommended: None,
                allow_other: false,
                permission: None,
            })
            .unwrap();

        let first = store.answer_decision(card.id, Some(0), None, "").unwrap();
        assert_eq!(first.status, DecisionStatus::Decided);
        assert_eq!(first.answer.as_deref(), Some("left"));

        let second = store.answer_decision(card.id, Some(1), None, "");
        assert!(second.is_err(), "the second answer is refused, not written over the first");
        assert_eq!(store.decision(card.id).unwrap().unwrap().answer.as_deref(), Some("left"), "the first answer stands");

        // Setting aside an answered card is refused the same way.
        assert!(store.dismiss_decision(card.id).is_err());
    }

    #[test]
    fn deleting_a_worker_takes_the_decisions_of_its_runs_and_no_others() {
        let store = Store::in_memory().unwrap();
        let track = store.create_track(&new_track("T", "", ".", "claude_code")).unwrap().id;
        let conductor = store.begin_run(&track, "conductor", "claude_code", "hi", ".").unwrap();
        let worker = store.begin_run(&track, "ui", "claude_code", "do it", ".").unwrap();
        let card = |run: &str| NewDecision {
            track: track.clone(),
            run: Some(run.to_string()),
            question: "Which way?".into(),
            context: String::new(),
            options: vec![opt("left"), opt("right")],
            recommended: None,
            allow_other: false,
            permission: None,
        };
        let his = store.open_decision(&card(&conductor)).unwrap();
        let hers = store.open_decision(&card(&worker)).unwrap();

        store.delete_worker(&track, "ui").unwrap();
        assert!(store.decision(hers.id).unwrap().is_none(), "a card on a run that is gone could never be answered");
        assert!(store.decision(his.id).unwrap().is_some(), "the conductor's own cards stay");
    }

    fn opt(label: &str) -> DecisionOption {
        DecisionOption { label: label.into(), detail: String::new(), id: String::new() }
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

    /// Every table and column SQLite reports, so two databases can be held
    /// against each other.
    fn shape(conn: &Connection) -> Vec<String> {
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut out = Vec::new();
        for name in names {
            let cols: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({name})"))
                .unwrap()
                .query_map([], |r| {
                    Ok(format!(
                        "{} {} notnull={} default={:?} pk={}",
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            out.push(format!("{name}({})", cols.join(", ")));
        }
        out
    }

    /// A database that came through the migrations has the same tables as one
    /// made from `SCHEMA` today.
    ///
    /// The two are written in different places and nothing tied them together:
    /// `routines` was created at 15 -> 16 with a `worker TEXT NOT NULL` column
    /// that later revisions stopped writing and no step ever removed, so
    /// creating a routine failed on every migrated database while a fresh
    /// install was fine. This is the check that was missing.
    ///
    /// It starts at 16 on purpose: `SCHEMA` no longer holds the shape the
    /// routines table was born with, so the 15 -> 16 step itself has to build
    /// it, exactly as it did on the databases in the wild.
    ///
    /// Column order counts here, and deliberately: `ALTER TABLE ADD COLUMN`
    /// can only append, so a column added that way belongs at the end of its
    /// table in `SCHEMA` as well. This fails until it is.
    #[test]
    fn a_migrated_database_ends_up_shaped_like_a_fresh_one() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-shape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v15.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            // Undo everything added after 15, then let the real steps redo it.
            conn.execute_batch(
                "DROP TABLE routines;
                 DROP TABLE routine_runs;
                 ALTER TABLE tracks DROP COLUMN worker_choice;
                 ALTER TABLE runs DROP COLUMN pid; ALTER TABLE runs DROP COLUMN pid_start; ALTER TABLE runs DROP COLUMN repeatable;
                 DROP TABLE notices; DROP TABLE schedules; DROP TABLE jobs;
                 INSERT INTO meta(key, value) VALUES ('schema_version', '15');",
            )
            .unwrap();
            // The routines table as it was actually born, from the step itself
            // rather than a copy of it that could drift. It is reached by
            // index, so a step inserted ahead of it would point this at other
            // SQL and quietly leave the test building a database nobody ever
            // had. Checked rather than trusted.
            let born = MIGRATIONS[14];
            assert!(
                born.contains("CREATE TABLE IF NOT EXISTS routines") && born.contains("worker      TEXT    NOT NULL"),
                "MIGRATIONS[14] is no longer the step that creates routines; find it again"
            );
            conn.execute_batch(born).unwrap();
            conn.execute("UPDATE meta SET value = '16' WHERE key = 'schema_version'", []).unwrap();
            conn.execute(
                "INSERT INTO routines(id, track, name, instruction, worker, agent, created_at, updated_at)
                 VALUES ('ro001', 'tr001', 'nightly', 'check the deps', 'dep-pr', 'codex', 1, 2)",
                [],
            )
            .unwrap();
        }

        let migrated = Store::open(&path).unwrap();
        let fresh = Store::in_memory().unwrap();
        assert_eq!(
            shape(&migrated.conn.lock()),
            shape(&fresh.conn.lock()),
            "a migrated database and a fresh one disagree about their tables"
        );

        // And the routine written under the old shape is still there, and a
        // new one can be made beside it -- which is what actually broke.
        let kept = migrated.routines().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].name.as_str(), kept[0].agent.as_str()), ("nightly", "codex"));
        migrated
            .create_routine(&RoutinePatch {
                name: Some("fresh one".into()),
                instruction: Some("do the thing".into()),
                cwd: Some("/w".into()),
                agent: Some("claude_code".into()),
                ..RoutinePatch::default()
            })
            .unwrap();
        assert_eq!(migrated.routines().unwrap().len(), 2);
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
            // Routines came at v16, and the replace above took their `track`
            // column with the runs one; the migrations make the table.
            conn.execute("DROP TABLE routines", []).unwrap();
            conn.execute_batch("ALTER TABLE runs DROP COLUMN pid; ALTER TABLE runs DROP COLUMN pid_start; ALTER TABLE runs DROP COLUMN repeatable; DROP TABLE notices; DROP TABLE schedules; DROP TABLE jobs;").unwrap();
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
    fn a_verbatim_folder_kept_earlier_reads_plain() {
        let store = Store::in_memory().unwrap();
        let tr = store.create_track(&new_track("t", "", r"\\?\C:\Users\x\repo", "claude_code")).unwrap();
        assert_eq!(tr.cwd, r"C:\Users\x\repo");
        assert_eq!(store.track(&tr.id).unwrap().unwrap().cwd, r"C:\Users\x\repo");
        let ro = store
            .create_routine(&RoutinePatch {
                name: Some("r".into()),
                instruction: Some("i".into()),
                cwd: Some(r"\\?\UNC\srv\share\ops".into()),
                agent: Some("claude_code".into()),
                config: None,
                ..RoutinePatch::default()
            })
            .unwrap();
        assert_eq!(ro.cwd, r"\\srv\share\ops");
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

    /// A crash that leaves a routine's agent running keeps its pid on the
    /// run, so the routine is not started again beside it; once that agent
    /// has gone (or the machine rebooted) the pid is let go. A run that
    /// ends the ordinary way keeps none.
    #[test]
    fn a_routine_left_running_by_a_crash_is_known() {
        let store = Store::in_memory().unwrap();
        let r = store
            .create_routine(&RoutinePatch { name: Some("check".into()), instruction: Some("look".into()), cwd: Some("/ops".into()), agent: Some("claude_code".into()), ..RoutinePatch::default() })
            .unwrap();
        let done = store.begin_routine_run(&r.id, "claude_code", "/ops", "look").unwrap();
        store.set_routine_run_pid(&done, Some(4242), None).unwrap();
        store.end_routine_run(&done, RoutineStatus::Done, "ok", "", 0, None).unwrap();
        assert_eq!(store.routine_left_running(&r.id).unwrap(), None, "an ended run keeps no pid");

        let cut = store.begin_routine_run(&r.id, "claude_code", "/ops", "look").unwrap();
        store.set_routine_run_pid(&cut, Some(4343), None).unwrap();
        let gone = store.begin_routine_run(&r.id, "claude_code", "/ops", "look").unwrap();
        store.set_routine_run_pid(&gone, Some(4444), None).unwrap();
        let alive = |pid: u32, _: Option<u64>| pid == 4343;
        let (_, routines) = store.close_live_runs_and_routines(&Interruption { why: "the app went down".into(), at: None, alive: &alive }).unwrap();
        assert_eq!(
            routines,
            vec![
                ClosedRoutineRun { run: cut.clone(), routine: r.id.clone(), left: Some(4343) },
                ClosedRoutineRun { run: gone.clone(), routine: r.id.clone(), left: None },
            ],
            "each cut run is said, with the agent left running when there is one"
        );
        let run = store.routine_run(&cut).unwrap().unwrap();
        assert_eq!((run.status, run.error.as_str()), (RoutineStatus::Failed, "the app went down"));
        assert_eq!(store.routine_left_running(&r.id).unwrap(), Some((cut.clone(), 4343, None)), "its agent is alive: kept");

        // The next comeback finds it gone: let go.
        let nobody = |_: u32, _: Option<u64>| false;
        store.close_live_runs(&Interruption { why: "again".into(), at: None, alive: &nobody }).unwrap();
        assert_eq!(store.routine_left_running(&r.id).unwrap(), None);
    }

    #[test]
    fn a_meta_key_is_claimed_once() {
        let store = Store::in_memory().unwrap();
        assert!(store.claim_meta("resumed:t006", "starting").unwrap());
        assert!(!store.claim_meta("resumed:t006", "starting").unwrap(), "the second caller does not have it");
        assert_eq!(store.get_meta("resumed:t006").unwrap().as_deref(), Some("starting"), "and changed nothing");
        store.delete_meta("resumed:t006").unwrap();
        assert!(store.claim_meta("resumed:t006", "starting").unwrap(), "let go, it can be claimed again");
    }

    #[test]
    fn routines_stand_alone_with_their_own_folder_agent_and_runs() {
        let store = Store::in_memory().unwrap();

        let made = || RoutinePatch {
            name: Some("  Dependency check  ".into()),
            instruction: Some("  list the stale dependencies  ".into()),
            cwd: Some("C:/ops".into()),
            agent: Some("claude_code".into()),
            config: None,
            ..RoutinePatch::default()
        };
        let a = store.create_routine(&made()).unwrap();
        assert_eq!((a.id.as_str(), a.name.as_str()), ("ro001", "Dependency check"), "ids run on, names are trimmed");
        assert_eq!(a.instruction, "list the stale dependencies", "and so is the instruction");
        assert_eq!((a.cwd.as_str(), a.agent.as_str()), ("C:/ops", "claude_code"), "its own folder and agent");
        assert_eq!((a.runs, a.last_run.as_deref()), (0, None), "a new routine has never run");

        // Every part it needs to run alone is required.
        assert!(store.create_routine(&RoutinePatch { name: Some("  ".into()), ..made() }).is_err());
        assert!(store.create_routine(&RoutinePatch { instruction: Some("".into()), ..made() }).is_err());
        assert!(store.create_routine(&RoutinePatch { cwd: None, ..made() }).is_err());
        assert!(store.create_routine(&RoutinePatch { agent: None, ..made() }).is_err(), "there is no track to borrow an agent from");

        // The human rewrites what was saved.
        let edited = store
            .update_routine(&a.id, &RoutinePatch {
                instruction: Some("read package.json and report".into()),
                config: Some(BTreeMap::from([("model".to_string(), "opus".to_string())])),
                ..RoutinePatch::default()
            })
            .unwrap();
        assert_eq!(edited.instruction, "read package.json and report");
        assert_eq!(edited.name, "Dependency check", "an absent field is kept");
        assert_eq!(edited.config.get("model").map(String::as_str), Some("opus"));
        assert!(store.update_routine("ro999", &RoutinePatch::default()).is_err());

        // Runs are the routine's own, whole, with ids of their own shape.
        let r1 = store.begin_routine_run(&a.id, "claude_code", "C:/ops", "check").unwrap();
        assert!(r1.starts_with("rr") && r1.len() == 18, "rr and sixteen hex: {r1}");
        assert_eq!(store.routine_run(&r1).unwrap().unwrap().status, RoutineStatus::Running);
        store
            .end_routine_run(&r1, RoutineStatus::Done, "all fine", "", 3, Some(&serde_json::json!({"status": "done"})))
            .unwrap();
        let done = store.routine_run(&r1).unwrap().unwrap();
        assert_eq!((done.status, done.tools, done.output.as_str()), (RoutineStatus::Done, 3, "all fine"));
        assert_eq!(done.report.unwrap()["status"], "done");
        assert!(done.ended_at.is_some());

        let r2 = store.begin_routine_run(&a.id, "claude_code", "C:/ops", "check").unwrap();
        assert_ne!(r1, r2, "two runs never share an id");
        let seen = store.routine(&a.id).unwrap().unwrap();
        assert_eq!(seen.runs, 2);
        let history = store.routine_runs(&a.id, 10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(store.routine_runs(&a.id, 1).unwrap().len(), 1, "the limit holds");

        // Deleting takes its runs with it.
        store.delete_routine(&a.id).unwrap();
        assert!(store.routine(&a.id).unwrap().is_none());
        assert!(store.routine_run(&r1).unwrap().is_none() && store.routine_run(&r2).unwrap().is_none());
        assert!(store.delete_routine(&a.id).is_err(), "deleting what is gone says so");
    }

    /// A routine's plan is a link to an artifact, set apart from a patch, and
    /// a run without a plan reads as one: nothing in the new columns.
    #[test]
    fn a_routine_links_to_its_plan() {
        let store = Store::in_memory().unwrap();
        let r = store
            .create_routine(&RoutinePatch {
                name: Some("deps".into()),
                instruction: Some("check".into()),
                cwd: Some("/w".into()),
                agent: Some("codex".into()),
                config: None,
                ..RoutinePatch::default()
            })
            .unwrap();
        assert_eq!(r.plan, "", "a routine starts without a plan");
        let plan = store.create_artifact("plan", "deps", "claude_code", "{}").unwrap();
        let linked = store.set_routine_plan(&r.id, &plan.id).unwrap();
        assert_eq!(linked.plan, plan.id);
        assert_eq!(linked.updated_at, r.updated_at, "opening a plan does not move the routine up the list");
        assert_eq!(store.routines().unwrap()[0].plan, plan.id);
        // An edit to the routine keeps the link.
        let edited = store.update_routine(&r.id, &RoutinePatch { name: Some("deps 2".into()), ..RoutinePatch::default() }).unwrap();
        assert_eq!(edited.plan, plan.id);
        assert!(store.set_routine_plan("ro999", &plan.id).is_err());
        assert_eq!(store.set_routine_plan(&r.id, "").unwrap().plan, "", "and it can be let go");

        // Its colour and tags, as a track's.
        let looked = store
            .update_routine(&r.id, &RoutinePatch { color: Some("#46c46a".into()), tags: Some(vec!["ops".into(), "weekly".into()]), ..RoutinePatch::default() })
            .unwrap();
        assert_eq!((looked.color.as_str(), looked.tags.clone()), ("#46c46a", vec!["ops".to_string(), "weekly".to_string()]));
        let kept = store.update_routine(&r.id, &RoutinePatch { name: Some("deps 3".into()), ..RoutinePatch::default() }).unwrap();
        assert_eq!((kept.color, kept.tags.len()), ("#46c46a".to_string(), 2), "a patch without them keeps them");
        let cleared = store.update_routine(&r.id, &RoutinePatch { color: Some(String::new()), tags: Some(vec![]), ..RoutinePatch::default() }).unwrap();
        assert_eq!((cleared.color.as_str(), cleared.tags.len()), ("", 0));

        let run = store.begin_routine_run(&r.id, "codex", "/w", "check").unwrap();
        let read = store.routine_run(&run).unwrap().unwrap();
        assert_eq!((read.plan, read.steps), (None, None), "a run kept before plans (plan = '', steps = '') reads as none");
        let ran = store.routine(&r.id).unwrap().unwrap();
        assert_eq!(ran.last_run_at, Some(read.started_at), "its last run's time, not when it was last edited");
        store.update_routine(&r.id, &RoutinePatch { color: Some("#7aa2f7".into()), ..RoutinePatch::default() }).unwrap();
        assert_eq!(store.routine(&r.id).unwrap().unwrap().last_run_at, Some(read.started_at), "an edit does not move it");
        assert!(store.set_routine_run_plan("rr-none", &serde_json::json!({})).is_err(), "no run, no silent success");
        assert!(store.set_routine_run_steps("rr-none", &serde_json::json!([])).is_err());
        assert!(store.update_routine("ro999", &RoutinePatch { name: Some("x".into()), ..RoutinePatch::default() }).is_err());

        // A run given a plan keeps it, and what its agent said of the steps.
        let given = serde_json::json!({ "version": 3, "steps": [{ "id": "s1", "title": "pull" }] });
        store.set_routine_run_plan(&run, &given).unwrap();
        store.set_routine_run_steps(&run, &serde_json::json!([{ "id": "s1", "status": "done", "note": "" }])).unwrap();
        let read = store.routine_run(&run).unwrap().unwrap();
        assert_eq!(read.plan.unwrap()["version"], 3);
        assert_eq!(read.steps.unwrap()[0]["status"], "done");
        assert_eq!(read.instruction, "check", "the instruction it was sent is its own, as before");
    }

    /// The run agent (the routine's) and the routine agent (its plan's, who
    /// works the plan out) are two choices: either changes, the other stays.
    #[test]
    fn a_routines_run_agent_and_its_plan_agent_are_apart() {
        let store = Store::in_memory().unwrap();
        let r = store
            .create_routine(&RoutinePatch {
                name: Some("deps".into()),
                instruction: Some("check".into()),
                cwd: Some("/w".into()),
                agent: Some("codex".into()),
                config: Some(BTreeMap::from([("model".to_string(), "gpt-5".to_string())])),
                ..RoutinePatch::default()
            })
            .unwrap();
        let plan = store.create_artifact("plan", "deps", "claude_code", "{}").unwrap();
        store.set_routine_plan(&r.id, &plan.id).unwrap();

        let plan_config = BTreeMap::from([("model".to_string(), "opus".to_string())]);
        store
            .update_artifact(&plan.id, &ArtifactPatch { agent: Some("copilot".into()), config: Some(plan_config.clone()), ..ArtifactPatch::default() })
            .unwrap();
        let r2 = store.routine(&r.id).unwrap().unwrap();
        assert_eq!((r2.agent.as_str(), r2.config.get("model").map(String::as_str)), ("codex", Some("gpt-5")), "the plan's agent leaves the run's alone");

        store.update_routine(&r.id, &RoutinePatch { agent: Some("gemini".into()), config: Some(BTreeMap::new()), ..RoutinePatch::default() }).unwrap();
        let (p, _) = store.artifact(&plan.id).unwrap().unwrap();
        assert_eq!((p.agent.as_str(), p.config.clone()), ("copilot", plan_config), "and the run's leaves the plan's alone");
        assert_eq!(store.routine(&r.id).unwrap().unwrap().agent, "gemini");
    }

    /// The bug this table exists for: a routine ran on the agent a track
    /// used, and its work turned up in that track's history. Every way the
    /// app reaches for runs is asked here, because it only takes one that
    /// does not name a track.
    #[test]
    fn a_routines_run_is_invisible_to_every_track_query() {
        let store = Store::in_memory().unwrap();
        let track = store.create_track(&new_track("Ops", "", "C:/repo", "copilot")).unwrap();
        let worker = store.begin_run(&track.id, "scan", "copilot", "look at the deps", "C:/repo").unwrap();

        let routine = store
            .create_routine(&RoutinePatch {
                name: Some("Dependency check".into()),
                instruction: Some("look at the deps".into()),
                cwd: Some("C:/repo".into()),
                // Deliberately the same agent and folder as the track: that
                // is the case that broke, and nothing about it may match.
                agent: Some("copilot".into()),
                config: None,
                ..RoutinePatch::default()
            })
            .unwrap();
        let rr = store.begin_routine_run(&routine.id, "copilot", "C:/repo", "look at the deps").unwrap();
        store.end_routine_run(&rr, RoutineStatus::Done, "found two", "", 1, None).unwrap();

        // Everything that reads runs.
        let all = store.runs().unwrap();
        assert_eq!(all.len(), 1, "only the worker's run is a run at all");
        assert_eq!(all[0].id, worker);
        assert!(all.iter().all(|r| !r.track.starts_with("routine:")), "no made-up track key survives");
        assert_eq!(all.iter().filter(|r| r.track == track.id).count(), 1, "the track owns exactly its own run");
        let sessions = store.sessions(&track.id).unwrap();
        assert_eq!(sessions.len(), 1, "one worker, and no routine among them");
        assert_eq!(sessions[0].name, "scan");
        assert!(store.run(&rr).unwrap().is_none(), "a routine run is not findable as a run");
        // Search reads an index with no track column at all, so it is the
        // easiest of these to leak through.
        let hits = store.search("deps").unwrap();
        assert!(hits.iter().all(|h| h.run == worker), "search finds the worker's run and nothing of the routine's");

        // And the routine still has its own, whole.
        let mine = store.routine_runs(&routine.id, 10).unwrap();
        assert_eq!((mine.len(), mine[0].id.as_str(), mine[0].output.as_str()), (1, rr.as_str(), "found two"));
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
                 ALTER TABLE tracks DROP COLUMN worker_folder;
                 ALTER TABLE tracks DROP COLUMN worker_choice;
                 ALTER TABLE runs DROP COLUMN pid; ALTER TABLE runs DROP COLUMN pid_start; ALTER TABLE runs DROP COLUMN repeatable;
                 DROP TABLE notices; DROP TABLE schedules; DROP TABLE jobs;
                 DROP TABLE artifacts;
                 DROP TABLE routines;
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
        assert_eq!(s.status, RunStatus::Interrupted, "the app's doing, not the agent's: not a failure");
        assert_eq!(s.error.as_deref(), Some(INTERRUPTED));
        assert_eq!(s.output, "half");
        assert_eq!(s.duration_ms, Some(8), "closed at the last event's time");
        assert!(s.ended_at.is_some());
        let last = store.events(&run).unwrap().pop().unwrap();
        assert_eq!(last.event.kind(), "interrupted");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The app that comes back knows more than "it closed": when it was
    /// last alive, what happened, and whether each run's agent is still
    /// there. A run whose agent is alive is not over; one whose agent is
    /// gone ended when the app was last seen, not when it noticed.
    #[test]
    fn live_runs_are_closed_with_what_the_app_knows() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-knows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);

        let (gone, alive, started) = {
            let store = Store::open(&path).unwrap();
            let gone = store.begin_run(TR, "review-core", "claude_code", "patch the repo", ".").unwrap();
            store.append(&gone, 7, &AgentEvent::Started { session_id: "s".into(), cwd: ".".into() }).unwrap();
            store.set_run_pid(&gone, Some(4242), None).unwrap();
            let alive = store.begin_run(TR, "review-data", "claude_code", "collect", ".").unwrap();
            store.append(&alive, 7, &AgentEvent::Started { session_id: "s2".into(), cwd: ".".into() }).unwrap();
            store.set_run_pid(&alive, Some(4343), None).unwrap();
            store.set_run_repeatable(&alive, true).unwrap();
            let started = store.run(&gone).unwrap().unwrap().started_at;
            (gone, alive, started)
        };

        let store = Store::open_keeping_live(&path).unwrap();
        assert_eq!(store.run(&gone).unwrap().unwrap().status, RunStatus::Running, "kept live until the app says");
        let seen = started + 90_000;
        let closed = store
            .close_live_runs(&Interruption { why: "the machine rebooted at 23:34".into(), at: Some(seen), alive: &|pid, _| pid == 4343 })
            .unwrap();
        assert_eq!(closed.len(), 2);
        let (g, a) = (&closed[0], &closed[1]);
        assert_eq!((g.run.id.as_str(), g.detached), (gone.as_str(), false));
        assert_eq!(g.run.status, RunStatus::Interrupted);
        assert_eq!(g.run.error.as_deref(), Some("the machine rebooted at 23:34"));
        assert_eq!(g.run.ended_at, Some(seen), "ended when the app was last seen alive");
        assert_eq!(g.run.duration_ms, Some(90_000), "and ran that long, not until the app noticed");
        assert_eq!((a.run.id.as_str(), a.detached), (alive.as_str(), true));
        assert_eq!(a.run.status, RunStatus::Detached, "its agent is still going");
        assert_eq!(a.run.ended_at, None, "so it has not ended");
        assert!(a.run.repeatable && a.run.pid == Some(4343));
        assert_eq!(store.detached_runs().unwrap().len(), 1);

        // The agent goes, and the run ends as interrupted, from where it was.
        store.append(&alive, 60_000, &AgentEvent::Interrupted { why: "its agent ended at 23:40".into() }).unwrap();
        let a = store.run(&alive).unwrap().unwrap();
        assert_eq!(a.status, RunStatus::Interrupted);
        assert_eq!(a.duration_ms, Some(60_000));
        assert!(store.detached_runs().unwrap().is_empty());
        // A run that is over does not come up again.
        assert!(store.close_live_runs(&Interruption { why: "again".into(), at: None, alive: &|_, _| false }).unwrap().is_empty());

        // A run detached on one comeback and still detached on the next: left
        // alone while its agent lives, ended like the rest once it does not
        // (after a reboot the app says nothing lives).
        let loose = store.begin_run(TR, "review-web", "claude_code", "screen", ".").unwrap();
        store.append(&loose, 7, &AgentEvent::Started { session_id: "s3".into(), cwd: ".".into() }).unwrap();
        store.set_run_pid(&loose, Some(4444), None).unwrap();
        let first = store.close_live_runs(&Interruption { why: "crashed".into(), at: None, alive: &|_, _| true }).unwrap();
        assert_eq!((first.len(), first[0].detached), (1, true));
        assert!(store.close_live_runs(&Interruption { why: "crashed again".into(), at: None, alive: &|_, _| true }).unwrap().is_empty(), "still alive: nothing to say twice");
        assert_eq!(store.run(&loose).unwrap().unwrap().status, RunStatus::Detached);
        let ended = store.close_live_runs(&Interruption { why: "the machine rebooted".into(), at: None, alive: &|_, _| false }).unwrap();
        assert_eq!((ended.len(), ended[0].detached), (1, false));
        assert_eq!(ended[0].run.status, RunStatus::Interrupted);
        assert_eq!(ended[0].run.error.as_deref(), Some("the machine rebooted"));
        assert_eq!(store.events(&loose).unwrap().iter().filter(|e| e.event.kind() == "detached").count(), 1, "detached once, however many comebacks");

        // A run that began after the app's last sign of life: it did not
        // end before it began.
        let late = store.begin_run(TR, "review-ops", "claude_code", "doc", ".").unwrap();
        let began = store.run(&late).unwrap().unwrap().started_at;
        let closed = store.close_live_runs(&Interruption { why: "crashed".into(), at: Some(began - 15_000), alive: &|_, _| false }).unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].run.ended_at, Some(began), "clamped to its start");
        assert_eq!(closed[0].run.duration_ms, Some(0));

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A turn the app gave up on is `timed_out`, with the reason where a
    /// failure's would be, and the fold happens as for any end.
    #[test]
    fn a_timed_out_turn_is_its_own_ending() {
        let store = Store::in_memory().unwrap();
        let run = store.begin_run(TR, "solo", "claude_code", "p", ".").unwrap();
        store.append(&run, 1, &AgentEvent::Message { text: "partial".into() }).unwrap();
        store.append(&run, 2, &AgentEvent::TimedOut { why: "said nothing for 20 minutes".into() }).unwrap();
        let s = store.run(&run).unwrap().unwrap();
        assert_eq!(s.status, RunStatus::TimedOut);
        assert_eq!(s.error.as_deref(), Some("said nothing for 20 minutes"));
        assert_eq!(s.output, "partial", "folded");
        assert_eq!(store.search("partial").unwrap().len(), 1, "and indexed");
        // Over: a later ending does not change it.
        store.append(&run, 3, &AgentEvent::Failed { error: "late".into() }).unwrap();
        assert_eq!(store.run(&run).unwrap().unwrap().status, RunStatus::TimedOut);
    }

    /// A notice stays until read; reading it twice is nothing.
    #[test]
    fn notices_are_kept_until_read() {
        let store = Store::in_memory().unwrap();
        assert!(store.add_notice(&NewNotice { title: "  ".into(), ..Default::default() }).is_err());
        let n = store
            .add_notice(&NewNotice { track: TR.into(), kind: "interrupted".into(), title: "2 runs cut".into(), body: "t564".into(), data: serde_json::json!({ "runs": ["t564"] }) })
            .unwrap();
        assert!(store.has_unread_notice(TR, "interrupted").unwrap());
        assert!(!store.has_unread_notice(TR, "stall").unwrap());
        let later = store.add_notice(&NewNotice { track: "".into(), kind: "stall".into(), title: "quiet".into(), ..Default::default() }).unwrap();
        let unread = store.unread_notices().unwrap();
        assert_eq!(unread.iter().map(|x| x.id).collect::<Vec<_>>(), vec![later.id, n.id], "newest first");
        assert_eq!(unread[1].data["runs"][0], "t564");
        let read = store.read_notice(n.id).unwrap();
        assert!(read.read_at.is_some());
        let again = store.read_notice(n.id).unwrap();
        assert_eq!(again.read_at, read.read_at, "the first reading stands");
        assert_eq!(store.unread_notices().unwrap().len(), 1);
        assert!(!store.has_unread_notice(TR, "interrupted").unwrap());
        assert!(store.read_notice(999).is_err());
    }

    /// A schedule is due by its time, counts its firings, moves on to its
    /// next time or ends, and can be taken back.
    #[test]
    fn schedules_fire_move_on_and_end() {
        let store = Store::in_memory().unwrap();
        assert!(store.create_schedule(&NewSchedule { track: TR.into(), kind: "at".into(), message: " ".into(), ..Default::default() }).is_err());
        let once = store
            .create_schedule(&NewSchedule { track: TR.into(), kind: "at".into(), spec: serde_json::json!({ "at": 1000 }), message: "check the build".into(), next_at: Some(1000), created_in: Some("t001".into()) })
            .unwrap();
        let daily = store
            .create_schedule(&NewSchedule { track: TR.into(), kind: "daily".into(), spec: serde_json::json!({ "daily": "07:10" }), message: "collect".into(), next_at: Some(5000), created_in: None })
            .unwrap();
        let watch = store
            .create_schedule(&NewSchedule { track: "tr002".into(), kind: "file".into(), spec: serde_json::json!({ "file": "/tmp/x.log", "contains": "DONE" }), message: "read it".into(), next_at: None, created_in: None })
            .unwrap();
        assert_eq!(once.status, ScheduleStatus::Active);
        assert_eq!(store.due_schedules(999).unwrap().len(), 0);
        assert_eq!(store.due_schedules(1000).unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), vec![once.id]);
        assert_eq!(store.due_schedules(9000).unwrap().len(), 2, "a condition is never due by time");
        assert_eq!(store.schedules(Some(TR), true).unwrap().len(), 2);
        assert_eq!(store.schedules(None, true).unwrap().len(), 3);

        let fired = store.schedule_fired(once.id, None, "ran 3 h late: the machine was off").unwrap();
        assert_eq!((fired.status, fired.fired, fired.next_at), (ScheduleStatus::Done, 1, None));
        assert!(fired.last_at.is_some());
        assert_eq!(fired.note, "ran 3 h late: the machine was off");
        assert!(store.schedule_fired(once.id, None, "").is_err(), "over is over");

        let moved = store.schedule_moved(daily.id, 7000, "pushed past quiet hours").unwrap();
        assert_eq!((moved.next_at, moved.fired), (Some(7000), 0), "moved, not fired");
        let again = store.schedule_fired(daily.id, Some(91_400_000), "").unwrap();
        assert_eq!((again.status, again.fired, again.next_at), (ScheduleStatus::Active, 1, Some(91_400_000)));

        // How far a watched file was read, kept so a restart goes on from there.
        assert_eq!(store.schedule(watch.id).unwrap().unwrap().seen, None, "nothing looked at yet");
        store.schedule_seen(watch.id, 4096, Some(-5)).unwrap();
        let seen = store.schedule(watch.id).unwrap().unwrap();
        assert_eq!((seen.seen, seen.seen_sum), (Some(4096), Some(-5)));
        let gone = store.cancel_schedule(watch.id).unwrap();
        assert_eq!((gone.status, gone.next_at), (ScheduleStatus::Cancelled, None));
        assert!(store.cancel_schedule(watch.id).is_err());
        assert_eq!(store.schedules(None, true).unwrap().len(), 1);
        assert_eq!(store.schedules(None, false).unwrap().len(), 3, "the record keeps what is over");
        // Over without firing: its time ran out while it was held back.
        let over = store.end_schedule(daily.id, "its until passed inside quiet hours").unwrap();
        assert_eq!((over.status, over.next_at, over.fired), (ScheduleStatus::Done, None, 1), "the earlier firing still counts");
        assert_eq!(over.note, "its until passed inside quiet hours");
        assert!(store.end_schedule(daily.id, "").is_err());
        assert!(store.schedules(None, true).unwrap().is_empty());
    }

    /// A job is watched while it runs or stalls, its log position and last
    /// line move with it, and an end sets its end time once.
    #[test]
    fn jobs_are_watched_until_they_end() {
        let store = Store::in_memory().unwrap();
        assert!(store.add_job(&NewJob { track: TR.into(), ..Default::default() }).is_err(), "nothing to watch");
        let job = store
            .add_job(&NewJob { track: TR.into(), worker: "review-data".into(), what: "weekly collection".into(), pid: Some(777), pid_start: Some(9), log: "/tmp/w.log".into(), done_pattern: "^done$".into(), stall_minutes: 0, boot: "b1".into() })
            .unwrap();
        assert_eq!((job.status, job.stall_minutes, job.read_bytes), (JobStatus::Running, 1, 0), "at least a minute");
        assert_eq!(store.jobs(Some(TR), true).unwrap().len(), 1);

        store.job_progress(job.id, 120, Some(7), "step 3", true).unwrap();
        let moved = store.job(job.id).unwrap().unwrap();
        assert_eq!((moved.read_bytes, moved.last_line.as_str()), (120, "step 3"));
        assert!(moved.changed_at >= job.changed_at);

        let stalled = store.set_job_status(job.id, JobStatus::Stalled, "no new line for 30 minutes").unwrap();
        assert_eq!((stalled.status, stalled.ended_at), (JobStatus::Stalled, None), "still watched");
        assert_eq!(store.jobs(None, true).unwrap().len(), 1);
        // The log moves again: back to running by itself.
        store.job_progress(job.id, 200, Some(8), "step 4", true).unwrap();
        assert_eq!(store.job(job.id).unwrap().unwrap().status, JobStatus::Running);

        let done = store.set_job_status(job.id, JobStatus::Done, "the log said done").unwrap();
        assert!(done.ended_at.is_some() && !done.status.is_open());
        assert!(store.jobs(None, true).unwrap().is_empty());
        assert_eq!(store.jobs(None, false).unwrap().len(), 1);
        let ended = done.ended_at;
        assert!(store.set_job_status(job.id, JobStatus::Died, "").is_err(), "the first end stands");
        assert!(store.set_job_status(job.id, JobStatus::Lost, "").is_err(), "nor is an ended job unwatched");
        assert_eq!(store.job(job.id).unwrap().unwrap().ended_at, ended);
        // Unwatched while a look was under way: the look's ending loses.
        let racing = store.add_job(&NewJob { track: TR.into(), worker: "review-data".into(), pid: Some(778), ..Default::default() }).unwrap();
        store.set_job_status(racing.id, JobStatus::Lost, "no longer watched").unwrap();
        assert!(store.set_job_status(racing.id, JobStatus::Died, "the process ended").is_err());
        assert_eq!(store.job(racing.id).unwrap().unwrap().status, JobStatus::Lost);
        assert!(store.set_job_status(999, JobStatus::Done, "").is_err());
    }

    /// Deleting a track takes its schedules, watched jobs and notices with
    /// it, and the notes, quiet hours and scopes kept for it in `meta`;
    /// another track's stay. Deleting a worker takes its scope.
    #[test]
    fn deleting_a_track_takes_what_outlived_its_sessions() {
        let store = Store::in_memory().unwrap();
        let a = store.create_track(&new_track("a", "", "/a", "claude_code")).unwrap();
        let b = store.create_track(&new_track("b", "", "/b", "claude_code")).unwrap();
        for t in [&a.id, &b.id] {
            store.create_schedule(&NewSchedule { track: t.clone(), kind: "daily".into(), spec: serde_json::json!({ "daily": "07:10" }), message: "m".into(), next_at: Some(1), created_in: None }).unwrap();
            store.add_job(&NewJob { track: t.clone(), worker: "w".into(), pid: Some(1), ..Default::default() }).unwrap();
            store.add_notice(&NewNotice { track: t.clone(), kind: "stall".into(), title: "quiet".into(), ..Default::default() }).unwrap();
            for key in ["notes:", "quiet:", "parked:"] {
                store.set_meta(&format!("{key}{t}"), "[]").unwrap();
            }
            store.set_meta(&format!("worker_scope:{t}/w"), "[\"apps/**\"]").unwrap();
        }
        store.delete_track(&a.id).unwrap();
        assert!(store.schedules(Some(&a.id), false).unwrap().is_empty());
        assert!(store.jobs(Some(&a.id), false).unwrap().is_empty());
        assert!(store.unread_notices().unwrap().iter().all(|n| n.track == b.id));
        for key in ["notes:", "quiet:", "parked:"] {
            assert_eq!(store.get_meta(&format!("{key}{}", a.id)).unwrap(), None, "{key}");
            assert!(store.get_meta(&format!("{key}{}", b.id)).unwrap().is_some(), "{key} of the other track stays");
        }
        assert_eq!(store.get_meta(&format!("worker_scope:{}/w", a.id)).unwrap(), None);
        assert_eq!(store.schedules(Some(&b.id), false).unwrap().len(), 1, "the other track's stay");
        assert_eq!(store.jobs(Some(&b.id), false).unwrap().len(), 1);
        // A worker's scope goes with its record.
        store.begin_run(&b.id, "w", "claude_code", "p", "/b").unwrap();
        store.delete_worker(&b.id, "w").unwrap();
        assert_eq!(store.get_meta(&format!("worker_scope:{}/w", b.id)).unwrap(), None);
    }

    /// The way back: a database taken to the version before reads in an
    /// older build (runs that ended the new ways read as failed there, and
    /// so do their events), and comes forward again whole.
    #[test]
    fn a_database_can_go_back_a_version_and_forward_again() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let (cut, fine) = {
            let store = Store::open(&path).unwrap();
            let cut = store.begin_run(TR, "solo", "claude_code", "p", ".").unwrap();
            store.append(&cut, 1, &AgentEvent::Message { text: "half".into() }).unwrap();
            store.append(&cut, 2, &AgentEvent::Interrupted { why: "rebooted".into() }).unwrap();
            let fine = store.begin_run(TR, "solo", "claude_code", "q", ".").unwrap();
            store.append(&fine, 1, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
            store.add_notice(&NewNotice { kind: "stall".into(), title: "x".into(), ..Default::default() }).unwrap();
            store.set_meta("boot_id", "b1").unwrap();
            (cut, fine)
        };

        assert!(Store::downgrade(&path, SCHEMA_VERSION).is_err(), "not newer than asked");
        assert!(Store::downgrade(&path, 21).is_err(), "no way back that far");
        // A look that changes nothing, from a file nobody has open.
        assert_eq!(Store::peek_meta(&path, "boot_id").unwrap().as_deref(), Some("b1"));
        assert_eq!(Store::peek_meta(&path, "nothing").unwrap(), None);
        assert_eq!(Store::peek_meta(&path, "schema_version").unwrap().as_deref(), Some(SCHEMA_VERSION.to_string().as_str()));
        assert_eq!(Store::downgrade(&path, 22).unwrap(), 22);
        assert_eq!(Store::peek_meta(&path, "schema_version").unwrap().as_deref(), Some("22"), "a peek does not bring it forward");
        {
            let conn = Connection::open(&path).unwrap();
            let version: String = conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
            assert_eq!(version, "22");
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('notices', 'schedules', 'jobs')")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(tables.is_empty(), "{tables:?}");
            let columns: Vec<String> = conn.prepare("PRAGMA table_info(runs)").unwrap().query_map([], |r| r.get(1)).unwrap().collect::<Result<_, _>>().unwrap();
            assert!(!columns.contains(&"pid".to_string()) && !columns.contains(&"repeatable".to_string()), "{columns:?}");
            let (status, error): (String, String) = conn.query_row("SELECT status, error FROM runs WHERE id = ?1", params![cut], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!((status.as_str(), error.as_str()), ("failed", "rebooted"), "reads as a failure with the same words");
            let (kind, payload): (String, String) = conn.query_row("SELECT kind, payload FROM events WHERE run_id = ?1 ORDER BY seq DESC LIMIT 1", params![cut], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(kind, "failed");
            assert_eq!(serde_json::from_str::<AgentEvent>(&payload).unwrap(), AgentEvent::Failed { error: "rebooted".into() }, "an older build can read the event");
            assert!(conn.query_row("SELECT value FROM meta WHERE key = 'boot_id'", [], |r| r.get::<_, String>(0)).optional().unwrap().is_none());
        }

        // Forward again: migrated to this build's shape, nothing else lost.
        let store = Store::open(&path).unwrap();
        let fresh = Store::in_memory().unwrap();
        assert_eq!(shape(&store.conn.lock()), shape(&fresh.conn.lock()));
        assert_eq!(store.run(&fine).unwrap().unwrap().status, RunStatus::Done);
        assert_eq!(store.run(&cut).unwrap().unwrap().status, RunStatus::Failed, "what went back stays a failure; the words say why");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_alongside_leaves_live_runs_live() {
        let dir = std::env::temp_dir().join(format!("orchestra-store-alongside-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);

        // The server holds it open with a run going; a CLI command opens it too.
        let server = Store::open(&path).unwrap();
        let run = server.begin_run(TR, "solo", "claude_code", "still going", ".").unwrap();
        server.append(&run, 7, &AgentEvent::Started { session_id: "s".into(), cwd: ".".into() }).unwrap();

        let cli = Store::open_alongside(&path).unwrap();
        cli.set_meta("setting:x", "y").unwrap();
        assert_eq!(server.get_meta("setting:x").unwrap().as_deref(), Some("y"));
        assert_eq!(server.run(&run).unwrap().unwrap().status, RunStatus::Running);

        drop((cli, server));
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

    /// What a conductor wrote before a tool call and after it are kept apart
    /// in the stored output; chunks of one passage still join as they came.
    ///
    /// A line ending in a single newline counts as unseparated: a list item is
    /// the everyday way that happens, and the renderer takes `breaks: true`, so
    /// the sentence after the call would be read as part of the item above it.
    #[test]
    fn text_on_either_side_of_a_tool_call_stays_two_passages() {
        let store = Store::in_memory().unwrap();
        let run = store.begin_run(TR, "conductor", "claude_code", "clean up", ".").unwrap();
        store.append(&run, 10, &AgentEvent::Message { text: "It's an npm global ".into() }).unwrap();
        store.append(&run, 11, &AgentEvent::Message { text: "install, not Homebrew.".into() }).unwrap();
        store.append(&run, 12, &tool("t1", "npm install -g tokscale@latest")).unwrap();
        store.append(&run, 13, &AgentEvent::Message { text: "`check-claude-app` is gone.".into() }).unwrap();
        store.append(&run, 90, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();

        let output = store.run(&run).unwrap().unwrap().output;
        assert_eq!(output, "It's an npm global install, not Homebrew.\n\n`check-claude-app` is gone.");

        // Ending on a newline is not ending on a blank line.
        let listy = store.begin_run(TR, "conductor", "claude_code", "check", ".").unwrap();
        store.append(&listy, 10, &AgentEvent::Message { text: "- reading the file\n".into() }).unwrap();
        store.append(&listy, 11, &tool("t1", "cat notes.md")).unwrap();
        store.append(&listy, 12, &AgentEvent::Message { text: "The file is empty.".into() }).unwrap();
        store.append(&listy, 90, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
        assert_eq!(store.run(&listy).unwrap().unwrap().output, "- reading the file\n\nThe file is empty.");

        // A blank line already there is not doubled.
        let spaced = store.begin_run(TR, "conductor", "claude_code", "check", ".").unwrap();
        store.append(&spaced, 10, &AgentEvent::Message { text: "Looking now.\n\n".into() }).unwrap();
        store.append(&spaced, 11, &tool("t2", "ls")).unwrap();
        store.append(&spaced, 12, &AgentEvent::Message { text: "Nothing there.".into() }).unwrap();
        store.append(&spaced, 90, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();
        assert_eq!(store.run(&spaced).unwrap().unwrap().output, "Looking now.\n\nNothing there.");
    }

    /// A call that started as "Terminal" and was named in an update is found
    /// by the command, not by the placeholder.
    #[test]
    fn search_finds_a_tool_by_the_title_an_update_gave_it() {
        let store = Store::in_memory().unwrap();
        let run = store.begin_run(TR, "solo", "claude_code", "build it", ".").unwrap();
        store.append(&run, 10, &tool("t1", "Terminal")).unwrap();
        let named = AgentEvent::ToolUpdate { id: "t1".into(), status: String::new(), title: "cargo build --release -p tokscale-cli".into(), paths: vec![] };
        store.append(&run, 11, &named).unwrap();
        store.append(&run, 12, &AgentEvent::ToolUpdate { id: "t1".into(), status: "completed".into(), title: String::new(), paths: vec![] }).unwrap();
        store.append(&run, 90, &AgentEvent::Finished { stop_reason: "end_turn".into() }).unwrap();

        let hit = |q: &str| store.search(q).unwrap().into_iter().map(|h| h.run).collect::<Vec<_>>();
        assert_eq!(hit("tokscale"), vec![run.clone()]);
        assert_eq!(store.run(&run).unwrap().unwrap().tool_count, 1, "an update is not another call");
    }

    #[test]
    fn fts_query_quotes_every_term() {
        assert_eq!(fts_query("a b"), Some("\"a\"* \"b\"*".into()));
        assert_eq!(fts_query("say \"hi\""), Some("\"say\"* \"\"\"hi\"\"\"*".into()));
        assert_eq!(fts_query("   "), None);
    }
}
