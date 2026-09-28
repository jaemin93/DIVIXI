import { invoke, listen, inTauri, local, bring, boardBase } from "./ipc.svelte";
import { boardPng, briefOf } from "./ink";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { i18n, systemLang, t, type Lang, type LangPref } from "./i18n.svelte";
import { notifyDecision, resolveDecisions, keepOnlyOpen } from "./notify.svelte";
import {
  dropQueued,
  liveQueued,
  liveQueuedFor,
  nextQueued,
  notNow,
  parseQueued,
  pushQueued,
  queuedFor,
  queuedKeys,
  releaseQueued as released,
  stillWaiting,
  unshiftQueued,
  wasStopped,
  type Queued,
} from "./queue";

import { addError, dropError, errorLife, type AppError } from "./errors";

export { wasStopped, errorLife, type Queued, type AppError };

/** Setting the waiting line is written to, so it survives the app closing. */
const QUEUE_SETTING = "queued";
/** Setting holding which tracks' headers are folded away. */
const HEADERS_SETTING = "track_headers";

/** Mirrors `orchestra_core::AgentEvent` — serde tags it with `kind`. */
export type AgentEvent =
  | { kind: "connected"; protocol: string; load_session: boolean }
  | { kind: "started"; session_id: string; cwd: string }
  | { kind: "message"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "tool_call"; id: string; title: string; tool_kind: string; status: string }
  | { kind: "tool_update"; id: string; status?: string; paths?: string[] }
  | { kind: "plan"; entries: string[] }
  | { kind: "usage"; raw: unknown }
  | { kind: "commands"; commands: SlashCommand[] }
  | {
      kind: "permission";
      request: string;
      title: string;
      tool_kind: string;
      input: string;
      options: { id: string; name: string; kind: string }[];
    }
  | { kind: "finished"; stop_reason: string }
  | { kind: "failed"; error: string };

/** Mirrors `orchestra_core::SlashCommand`: one `/name` the agent offers. */
export type SlashCommand = { name: string; description: string; hint: string | null };

/** Mirrors `conductor::ConductorState`. */
export type ConductorState = { open: boolean; busy: boolean; agent: string | null; commands: SlashCommand[] };

export type Envelope = {
  track: string;
  session: string;
  run: string;
  at_ms: number;
  event: AgentEvent;
};

/** Session options chosen for one role: `option id → value id`, in the agent's own terms. */
export type OptionConfig = Record<string, string>;

/** Mirrors `conductor::WorkerState`: one worker's live agent session. */
export type WorkerState = {
  name: string;
  /** Its agent session is open (a process is alive). */
  open: boolean;
  /** A turn is in flight. */
  running: boolean;
  run: string | null;
  agent: string;
  /** Model value id it is running on; empty means the agent's own. */
  model: string;
};

/** Mirrors `conductor::WaitingItem`: one turn a conductor has not taken yet. */
export type WaitingItem = {
  /** Unique among what is waiting; the conversation keys its list on it. */
  id: string;
  track: string;
  /** What it is, in one phrase, e.g. "report of ui run t042". */
  what: string;
  /** The worker it came from, when it is a report. */
  worker: string | null;
  /** The worker run it reports on; its report is already in the record. */
  run: string | null;
  at: number;
  why: string;
  tries: number;
};

/** Mirrors `conductor::Waiting`: what a track's conductor has not taken yet. */
export type WaitingDelivery = {
  track: string;
  pending: number;
  /** What was just put aside, when one was. */
  added: string | null;
  worker: string | null;
  run: string | null;
  why: string;
};

/** Mirrors `orchestra_store::TrackInfo`: one conductor, its workers, one folder. */
export type Track = {
  id: string;
  name: string;
  intent: string;
  cwd: string;
  /** Agent the conductor runs on. */
  agent: string;
  conductor_config: OptionConfig;
  /** Agent workers run on; empty means the conductor's. */
  worker_agent: string;
  worker_config: OptionConfig;
  /** Where workers work: a folder of their own under the track's, a git worktree, or the track folder. */
  worker_folder: WorkerFolder;
  /** Whether the conductor may propose another agent or model for a worker. */
  worker_choice: WorkerChoice;
  /** `#rrggbb` for the list, or empty. */
  color: string;
  tags: string[];
  created_at: number;
  updated_at: number;
  runs: number;
};

/** Mirrors `orchestra_store::TrackPatch`: fields to set, the rest kept. */
export type TrackPatch = Partial<{
  name: string;
  intent: string;
  cwd: string;
  agent: string;
  conductor_config: OptionConfig;
  worker_agent: string;
  worker_config: OptionConfig;
  worker_folder: WorkerFolder;
  worker_choice: WorkerChoice;
  color: string;
  tags: string[];
}>;

/** orchestra_store::WORKER_FOLDERS. */
export type WorkerFolder = "subfolder" | "worktree" | "shared";
export const WORKER_FOLDERS: WorkerFolder[] = ["subfolder", "worktree", "shared"];

/**
 * orchestra_store::WORKER_CHOICES: who picks a worker's agent and model.
 *
 * `follow`: this track's, always. `propose`: the conductor may ask for
 * something else and the human approves it on a decision card.
 */
export type WorkerChoice = "follow" | "propose";
export const WORKER_CHOICES: WorkerChoice[] = ["follow", "propose"];

/** How the track list is narrowed, ordered and folded. Persisted as the `tracks_filter` setting. */
export type TrackSort = "recent" | "oldest" | "created-desc" | "created-asc" | "az" | "za";
export type TrackFilter = {
  /** Only tracks with a run in flight. */
  running: boolean;
  /** Only tracks whose conductor session is open. */
  active: boolean;
  /** Only tracks with activity inside this window; empty means any. */
  recent: "" | "1h" | "24h" | "7d";
  sort: TrackSort;
  /** Tracks idle longer than this many days fold away; 0 keeps them all in place. */
  fold: number;
  /** Only tracks carrying every one of these tags. */
  tags: string[];
};
export const DEFAULT_TRACK_FILTER: TrackFilter = { running: false, active: false, recent: "", sort: "recent", fold: 7, tags: [] };

/** A tag the user made: its name and the colour it was dealt. */
export type TagDef = { name: string; color: string };

/** Colours a track can carry in the list; muted enough for both themes. */
export const TRACK_COLORS = ["#7aa2f7", "#73daca", "#9ece6a", "#e0af68", "#ff9e64", "#f7768e", "#bb9af7", "#c0caf5"];

/** The agent a track's workers run on. */
export function workerAgentOf(track: Track): string {
  return track.worker_agent || track.agent;
}

/** The options workers open with: the worker's own, else (never set apart) the conductor's. */
export function workerConfigOf(track: Track): OptionConfig {
  const own = track.worker_agent !== "" || Object.keys(track.worker_config).length > 0;
  return own ? track.worker_config : track.conductor_config;
}

/** Who a session setting is for. */
export type Role = "conductor" | "worker";

/** Mirrors `orchestra_store::RunSummary`: one run as the timeline sees it. */
export type RunSummary = {
  id: string;
  track: string;
  session: string;
  agent: string;
  prompt: string;
  cwd: string;
  status: RunStatus;
  started_at: number;
  duration_ms: number | null;
  session_id: string | null;
  stop_reason: string | null;
  error: string | null;
  output: string;
  plan: string[];
  tool_count: number;
};

/** Mirrors `orchestra_store::StoredEvent`. */
export type StoredEvent = { seq: number; at_ms: number; event: AgentEvent };

/** Mirrors `orchestra_agents::AgentKind` ids. */
export type AgentId = "claude_code" | "codex" | "copilot" | "antigravity";
export const AGENT_IDS: AgentId[] = ["claude_code", "codex", "copilot", "antigravity"];

export type Readiness = "ready" | "needs_login" | "needs_download" | "not_installed" | "error";

export type AuthMethodInfo = {
  id: string;
  name: string;
  description: string | null;
  terminal_command: string | null;
};

/** Mirrors `orchestra_acp::ConfigChoice`. */
export type ConfigChoice = { id: string; name: string; description: string | null; group: string | null };

/** Mirrors `orchestra_acp::ConfigOptionInfo`: a session option the agent exposes. */
export type ConfigOption = {
  id: string;
  name: string;
  description: string | null;
  /** `mode`, `model`, `thought_level`, or the agent's own word. */
  category: string;
  current: string;
  choices: ConfigChoice[];
};

/** Mirrors `orchestra_acp::ModeInfo`. */
export type ModeInfo = { id: string; name: string; description: string | null };

/** Mirrors `orchestra_agents::AgentStatus`. */
export type AgentStatus = {
  kind: AgentId;
  name: string;
  readiness: Readiness;
  cli: { path: string; version: string | null } | null;
  adapter: { kind: string; path?: string; package?: string; reason?: string };
  probe: {
    protocol: string;
    agent_name: string | null;
    agent_version: string | null;
    auth_methods: AuthMethodInfo[];
    config_options?: ConfigOption[];
    modes?: ModeInfo[];
    default_mode?: string | null;
    /** The mode Orchestra picks when none is chosen. */
    autonomous_mode?: string | null;
  } | null;
  error: string | null;
  login_hint: string;
  install_hint: string;
};

export type Tool = { id: string; title: string; toolKind: string; status: string };

/** The turn as it happened: prose and tool calls in arrival order. */
export type Segment = { kind: "text"; text: string } | { kind: "thought"; text: string } | { kind: "tool"; tool: Tool };

export type TranscriptLine = { ms: number; label: string; text: string; tone: Tone };
export type Tone = "in" | "out" | "ok" | "warn" | "dim";

export type RunStatus = "connecting" | "running" | "done" | "failed";

/** Context accounting from the agent's `usage` updates. */
export type Usage = { used: number; size: number; cost?: number; currency?: string };

export type Run = {
  id: string;
  track: string;
  session: string;
  agent: string;
  prompt: string;
  status: RunStatus;
  sessionId?: string;
  cwd?: string;
  /** Unix milliseconds. */
  startedAt: number;
  durationMs?: number;
  /** Above the membrane: what the Report card shows. */
  message: string;
  plan: string[];
  toolCount: number;
  stopReason?: string;
  error?: string;
  /**
   * The queued message this turn was made for, when the app made it.
   *
   * A run's `id` is not a handle: it starts as `pending-…` and is swapped
   * for the core's id the moment either the send comes back or an event
   * arrives first — and `apply` will hand a pending run to whichever
   * conductor turn speaks first, which is not always ours. This does not
   * change, so cleaning up after a refused send removes the right run,
   * and the conversation can tell that a waiting message already has a
   * turn on screen. Not persisted: it only matters while both could show.
   */
  fromQueued?: string;
  /** Latest context accounting; live runs update it, restored runs get it on hydrate. */
  usage?: Usage;
  /**
   * Below the membrane: what the inspector shows. Empty until `loaded`,
   * which is immediate for live runs and lazy for restored ones.
   */
  loaded: boolean;
  thought: string;
  tools: Tool[];
  transcript: TranscriptLine[];
  /** Ordered prose and tool calls, for showing a turn as it unfolds. */
  segments: Segment[];
};

export type View = "track" | "settings" | "worker" | "new-track" | "edit-track" | "design" | "knowledge";

// ----- artifacts: what the human keeps beside tracks and attaches to them —
// designs (a sketch board worked out with an agent), knowledge later -----
export type ArtifactKind = "design" | "knowledge";
export type ArtifactInfo = {
  id: string;
  kind: ArtifactKind;
  title: string;
  agent: string;
  config: OptionConfig;
  color: string;
  tags: string[];
  created_at: number;
  updated_at: number;
};
export type Stroke = { points: [number, number, number][]; color: string; size: number };
export type DesignTag = "" | "goal" | "constraint" | "question" | "idea";
export type DesignNode = {
  id: string;
  kind: "note" | "sketch" | "file" | "link" | "frame" | "question";
  x: number;
  y: number;
  w: number;
  h: number;
  text: string;
  tag: DesignTag;
  strokes: Stroke[];
  by: string;
  /** A file's place in the design's folder (`files/…`). */
  src?: string;
  /** A file's name, or a link's page title. */
  name?: string;
  mime?: string;
  url?: string;
  /** A question's answer. */
  answer?: string;
};
export type DesignEdge = { id: string; from: string; to: string; label: string; by: string };
export type DesignChange = { id: number; target: string; run: string | null; before: unknown; after: unknown };
export type DesignDoc = { version: number; nodes: DesignNode[]; edges: DesignEdge[]; changes: DesignChange[]; next: number };
/** One edit to a board, as the core applies it. */
export type DesignOp =
  | { op: "create_note"; x: number; y: number; w?: number; h?: number; text?: string; tag?: DesignTag }
  | { op: "add_stroke"; stroke: Stroke }
  | { op: "set_sketch"; id?: string; x: number; y: number; w: number; h: number; strokes: Stroke[] }
  | { op: "update"; id: string; text?: string; tag?: DesignTag; answer?: string; url?: string; title?: string }
  | { op: "create_frame"; x: number; y: number; w: number; h: number; title?: string }
  | { op: "create_question"; x: number; y: number; text: string; w?: number; h?: number }
  | { op: "create_link"; x: number; y: number; url: string; title?: string; text?: string }
  | { op: "move"; id: string; x: number; y: number; w?: number; h?: number }
  | { op: "delete"; ids: string[] }
  | { op: "connect"; from: string; to: string; label?: string };
export type DesignResult = { ok: boolean; id?: string; error?: string };
export const ARTIFACT_SESSION = "artifact";
/** What changed on a design's board, as the core sends it. */
export type DesignDelta = {
  id: string;
  base: number;
  version: number;
  nodes: DesignNode[];
  removed_nodes: string[];
  edges: DesignEdge[];
  removed_edges: string[];
  changes: DesignChange[];
  next: number;
};

/** A board with a delta applied: changed items in place, new ones at the end. */
export function withDelta(doc: DesignDoc, d: DesignDelta): DesignDoc {
  const merge = <T extends { id: string }>(list: T[], changed: T[], removed: string[]): T[] => {
    const byId = new Map(changed.map((x) => [x.id, x]));
    const gone = new Set(removed);
    const kept = list.filter((x) => !gone.has(x.id)).map((x) => byId.get(x.id) ?? x);
    const known = new Set(list.map((x) => x.id));
    return [...kept, ...changed.filter((x) => !known.has(x.id))];
  };
  return {
    version: d.version,
    nodes: merge(doc.nodes, d.nodes, d.removed_nodes),
    edges: merge(doc.edges, d.edges, d.removed_edges),
    changes: d.changes,
    next: d.next,
  };
}

/** Artifact conversations are kept under this key, apart from tracks. */
export const artifactKey = (id: string) => `artifact:${id}`;
const EMPTY_DOC: DesignDoc = { version: 0, nodes: [], edges: [], changes: [], next: 0 };

/** Prefix of conductor prompts Orchestra injects itself (worker reports). Language-neutral. */
export const REPORT_PREFIX = "[worker-report]";
/** How the app's second ask for a missing report starts (conductor.rs, report::reminder). */
export const REPORT_REMINDER = "Your reply did not end with a usable report";
/** The fence a worker's report block opens with. */
export const REPORT_FENCE = "```divixi-report";

/** A worker turn's checked report (orchestra_core::report::Report). */
export type WorkerReport = {
  status: "done" | "partial" | "blocked" | "failed";
  summary: string;
  changes: { path: string; what: string }[];
  checks: { what: string; result: "pass" | "fail" | "not_run"; detail: string }[];
  risks: string[];
  questions: string[];
  next: string[];
  structured: boolean;
  problem?: string;
  edits_seen: string[];
  /** Edits outside the worker's folder. */
  outside?: string[];
  reminder_run?: string;
  /**
   * How the turn ended when it did not end on its own (a timeout, a dead
   * agent, a stop). The worker wrote this report before that, so what it
   * says of the work stands — but it may have meant to do more after.
   */
  interrupted?: string;
};

/** What a worker changed in its own checkout and the human has not merged (worktree.rs). */
export type WorkerChanges = {
  isolated: boolean;
  branch: string;
  sub: string;
  files: { path: string; status: string; added: number | null; removed: number | null }[];
};
/** How a merge ended: files written, or the conflicts that stopped it. */
export type WorkerMerged = { files: string[]; conflicts: string[] };

/** A worker's reply without its report block (the card shows the report). */
export function withoutReportBlock(text: string): string {
  const at = text.indexOf(REPORT_FENCE);
  if (at < 0) return text;
  const close = text.indexOf("```", at + REPORT_FENCE.length);
  return (text.slice(0, at) + (close < 0 ? "" : text.slice(close + 3))).trimEnd();
}
/** Heads the list of files under a human message, as the core stores it. */
export const ATTACH_MARK = "[attachments]";

/** A file waiting in the composer to go with the next message. */
export type Attachment = { path: string; name: string; size: number };

/** A passage from the knowledge library, picked with `@kb` for the next message. */
export type KbPick = {
  id: number;
  title: string;
  source: string;
  section: string | null;
  line_start: number;
  line_end: number;
  summary: string;
  content: string;
  tokens: number;
  match_type: string;
};

/** Where picked knowledge sits in a message. */
export const KB_OPEN = "[knowledge]";
export const KB_CLOSE = "[/knowledge]";
/** Starts each passage's header line (a passage's own "###" headings are not titles). */
const KB_HEAD = "### 📚 ";

/** The message with picked knowledge after what the human wrote. */
export function withKnowledge(text: string, picks: KbPick[]): string {
  if (!picks.length) return text;
  const parts = picks.map((p) => {
    const where = [p.source, p.section ? `§ ${p.section}` : "", `lines ${p.line_start}-${p.line_end}`].filter(Boolean).join(" · ");
    const title = (p.title || "(untitled)").replace(/\s+/g, " ");
    return `${KB_HEAD}${title} — ${where}\n${p.content.trim()}`;
  });
  const block = `${KB_OPEN}\nThe human attached these passages from the knowledge library:\n\n${parts.join("\n\n")}\n${KB_CLOSE}`;
  return text.trim() ? `${text}\n\n${block}` : block;
}

/** A message split into what the human wrote and the titles of the knowledge it carries. */
export function splitKnowledge(text: string): { text: string; titles: string[] } {
  const at = text.startsWith(`${KB_OPEN}\n`) ? 0 : text.indexOf(`\n\n${KB_OPEN}\n`);
  if (at < 0) return { text, titles: [] };
  const end = text.lastIndexOf(KB_CLOSE);
  const block = text.slice(at, end < at ? undefined : end);
  const titles = block
    .split("\n")
    .filter((l) => l.startsWith(KB_HEAD))
    .map((l) => l.slice(KB_HEAD.length).split(" — ")[0]);
  const rest = end < at ? "" : text.slice(end + KB_CLOSE.length);
  return { text: (text.slice(0, at) + rest).trim(), titles };
}

/** A stored prompt split into what the human wrote and the files they attached. */
export function splitAttachments(prompt: string): { text: string; files: string[] } {
  const at = prompt.indexOf(`\n\n${ATTACH_MARK}\n`);
  const head = prompt.startsWith(`${ATTACH_MARK}\n`) ? 0 : at;
  if (head < 0) return { text: prompt, files: [] };
  const list = prompt.slice(head).replace(/^\s*\[attachments\]\n/, "");
  const files = list
    .split("\n")
    .map((l) => l.replace(/^- /, "").trim())
    .filter(Boolean);
  return { text: prompt.slice(0, head), files };
}

/** The message as the core will store it: the text, then its files. */
function withAttachments(text: string, files: string[]): string {
  if (!files.length) return text;
  return `${text}\n\n${ATTACH_MARK}\n${files.map((f) => `- ${f}`).join("\n")}`;
}

/** First line of a turn that hands the conductor a worker's permission question. */
export const PERMISSION_PREFIX = "[worker-permission]";

/** First line of the turn that carries the human's answer to a decision card. */
export const DECISION_PREFIX = "[decision]";

/** A question the conductor put to the human, as the core keeps it. */
export type DecisionOption = { label: string; detail: string; id?: string };
export type DecisionStatus = "open" | "decided" | "dismissed";
export type Decision = {
  id: number;
  track: string;
  /** The conductor run that asked. */
  run: string | null;
  question: string;
  context: string;
  options: DecisionOption[];
  recommended: number | null;
  allow_other: boolean;
  status: DecisionStatus;
  /** Index of the chosen option; null for an answer in the human's own words. */
  choice: number | null;
  answer: string | null;
  note: string;
  created_at: number;
  decided_at: number | null;
  /** Set when the card is an agent's permission question. */
  permission: { session: string; request: string } | null;
};

/** What the user asked for; `system` follows the OS. */
export type ThemePref = "system" | "dark" | "light";

/** Conversation text size. */
export type ChatFont = "s" | "m" | "l";

/** Interface typeface. */
export type UiFont = "sans" | "mono" | "system" | "serif";

/** Zoom bounds in percent; steps of 10, as Ctrl+= / Ctrl+- move. */
export const ZOOM_MIN = 50;
export const ZOOM_MAX = 200;
export const ZOOM_STEP = 10;

/** Settings sections, in the settings column. */
export type SettingsSection = "overview" | "appearance" | "chat" | "agents" | "knowledge" | "remote" | "about";

/** Mirrors `workspace::Entry`: one file or folder, path relative to the track folder. */
export type WsEntry = { path: string; name: string; dir: boolean; size: number };

/** Mirrors `workspace::FileContent`. */
export type WsFile = {
  path: string;
  name: string;
  size: number;
  kind: "markdown" | "html" | "text" | "image" | "binary" | "large";
  ext: string;
  text: string | null;
  data_url: string | null;
  /** The text is the file's exact contents, so it can be edited and saved. */
  editable: boolean;
};

/** An edit not yet saved: its text, and the file's text it started from. */
export type WsDraft = { text: string; base: string };

/** Line breaks as one `\n`. */
function lf(text: string): string {
  return text.includes("\r") ? text.replace(/\r\n/g, "\n") : text;
}

/** Mirrors `workspace::CHANGED_ON_DISK`. */
const CHANGED_ON_DISK = "changed on disk";

/** Mirrors `workspace::Change`. */
export type WsChange = { path: string; code: string; untracked: boolean; from: string | null };

/** Mirrors `workspace::GitStatus`. */
export type WsGit = { repo: boolean; branch: string; changes: WsChange[] };

/** What the side panel shows: git changes, the file tree, or an open file. */
export type PanelTab = "changes" | "files" | "file";

/** Mirrors the `app_info` command. */
export type AppInfo = {
  version: string;
  db_path: string;
  logs_dir: string;
  adapters_dir: string;
  workspace: string;
  runs: number;
};

const scheme = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

/** Mirrors the `agent_download` event payload. */
export type DownloadProgress = {
  agent: AgentId;
  phase: "downloading" | "unpacking" | "done";
  received: number;
  total: number | null;
};

/** Everything above the membrane plus, per run, the detail kept below it. */
class Store {
  /** Effective theme class on <html>. */
  theme = $state<"dk" | "lt">("dk");
  /** The stored preference behind `theme`. */
  themePref = $state<ThemePref>("system");
  view = $state<View>("track");
  /** First-run setup, shown in front of the shell. */
  setupOpen = $state(false);
  /** Setup step: 1 agents, 2 appearance. */
  setupStep = $state<1 | 2>(1);
  /** Left rail folded to icons. Persisted. */
  railCollapsed = $state(false);
  /** Second column (tracks and their workers) shown. Persisted. */
  trackListOpen = $state(true);
  /** Which settings section is open. */
  settingsSection = $state<SettingsSection>("overview");
  /** Worker whose session is open in the main area (view === "worker"). */
  openWorker = $state("");
  /** Conversation text size. Persisted. */
  chatFont = $state<ChatFont>("s");
  /** Interface typeface. Persisted. */
  uiFont = $state<UiFont>("sans");
  /** Interface language preference. Persisted; "system" follows the OS. */
  langPref = $state<LangPref>("system");
  /** Native webview zoom in percent. Persisted. */
  zoom = $state(100);
  info = $state<AppInfo | null>(null);
  /** Column widths in px: the rail (expanded), the tracks column, the side panel. Persisted. */
  railWidth = $state(200);
  trackListWidth = $state(264);
  panelWidth = $state(460);

  /** The bottom terminal panel: shown, its height, and whether it was ever
   *  opened (it stays mounted after that so shells survive folding). */
  termOpen = $state(false);
  termHeight = $state(280);
  termMounted = $state(false);

  /** Artifacts of every kind, most recently touched first, and the one open. */
  artifacts = $state<ArtifactInfo[]>([]);
  artifact = $state("");
  /** Replaced whole by each board the core sends; never changed in place. */
  designDoc = $state.raw<DesignDoc>({ ...EMPTY_DOC });
  /** Where each link or document card's text stands, for the open board. */
  designExtracts = $state<Record<string, { state: "done" | "failed" | "reading"; error?: string }>>({});

  async loadExtracts(id = this.artifact) {
    if (!id) return;
    try {
      const list = await invoke<{ card: string; state: "done" | "failed" | "reading"; error?: string }[]>("design_extracts", { id });
      if (id === this.artifact) this.designExtracts = Object.fromEntries(list.map((x) => [x.card, { state: x.state, error: x.error }]));
    } catch {
      // Cosmetic.
    }
  }

  async retryExtract(card: string) {
    const id = this.artifact;
    if (!id) return;
    this.designExtracts = { ...this.designExtracts, [card]: { state: "reading" } };
    try {
      await invoke("design_extract_retry", { id, card });
    } catch (err) {
      this.lastError = String(err);
    }
  }
  /** The design whose board `designDoc` holds. */
  designLoaded = $state("");
  /** Width of the conversation beside a design's board. Persisted. */
  artifactChatWidth = $state(460);
  /** The designs column shown. Persisted. */
  designListOpen = $state(true);

  setArtifactChatWidth(px: number, persist = false) {
    this.artifactChatWidth = Math.min(760, Math.max(340, Math.round(px)));
    if (persist) this.persistWidth("artifact_chat_width", this.artifactChatWidth);
  }

  async setDesignList(open: boolean) {
    this.designListOpen = open;
    try {
      await invoke("set_setting", { key: "designlist", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Items picked on the board; they go with the next message. */
  designSelected = $state<string[]>([]);
  /** The open artifact's agent session. */
  artifactSession = $state<{ open: boolean; busy: boolean }>({ open: false, busy: false });

  /** What is typed but not sent, per conversation; the box shows the one on screen. */
  messageDrafts: Record<string, string> = {};

  /** Knowledge picked with `@kb`, waiting in each conversation's composer. */
  kbPickedBy = $state<Record<string, KbPick[]>>({});

  get kbPicked(): KbPick[] {
    return this.kbPickedBy[this.chatKey] ?? [];
  }

  /** Add passages to the next message (each once). */
  pickKnowledge(picks: KbPick[]) {
    const key = this.chatKey;
    const now = this.kbPickedBy[key] ?? [];
    const fresh = picks.filter((p) => !now.some((x) => x.id === p.id));
    this.kbPickedBy = { ...this.kbPickedBy, [key]: [...now, ...fresh] };
  }

  unpickKnowledge(id: number) {
    const key = this.chatKey;
    this.kbPickedBy = { ...this.kbPickedBy, [key]: (this.kbPickedBy[key] ?? []).filter((p) => p.id !== id) };
  }

  /** Take the picked knowledge for the message going out now. */
  private takeKnowledge(key: string): KbPick[] {
    // (Put back with restoreKnowledge when the message does not go out.)
    const picks = this.kbPickedBy[key] ?? [];
    if (picks.length) this.kbPickedBy = { ...this.kbPickedBy, [key]: [] };
    return picks;
  }

  private restoreKnowledge(key: string, picks: KbPick[]) {
    if (picks.length && !(this.kbPickedBy[key] ?? []).length) this.kbPickedBy = { ...this.kbPickedBy, [key]: picks };
  }

  /** Files waiting in each conversation's composer (a track's, an artifact's). */
  attachmentsBy = $state<Record<string, Attachment[]>>({});
  /** Design briefs being written out to attach; the composer waits for them. */
  attaching = $state(0);

  /** The conversation on screen, as a key for what waits in its composer. */
  get chatKey(): string {
    return this.chatArtifact ? artifactKey(this.artifact) : `track:${this.track}`;
  }

  /** Files waiting to go with the next message of the conversation on screen. */
  get attachments(): Attachment[] {
    return this.attachmentsBy[this.chatKey] ?? [];
  }

  set attachments(list: Attachment[]) {
    this.attachmentsBy = { ...this.attachmentsBy, [this.chatKey]: list };
  }

  /** Every decision of every track, oldest first. */
  decisions = $state<Decision[]>([]);
  /** The decision whose answer is on its way. */
  answering = $state<number | null>(null);

  /** The side panel on the track: the working folder as files and changes. Persisted. */
  panelOpen = $state(false);
  panelTab = $state<PanelTab>("files");
  /** Open file tabs, relative paths, in opening order; `activeFile` is the one shown. */
  openFiles = $state<string[]>([]);
  activeFile = $state("");
  /** The tree as a drawer over an open file, as Kiro has it; closed until asked for. */
  panelTree = $state(false);
  tree = $state<WsEntry[]>([]);
  treeLoading = $state(false);
  git = $state<WsGit | null>(null);
  gitLoading = $state(false);
  /** The change whose diff is shown. */
  diffPath = $state("");
  diffs = $state<Record<string, string>>({});
  files = $state<Record<string, WsFile>>({});
  /** Markdown files render as a preview unless the raw text is asked for. */
  rawMarkdown = $state<Record<string, boolean>>({});
  /**
   * Unsaved edits, by `track\0path`, so they outlive switching tracks and
   * reloading the file: the base they started from is kept with them.
   */
  drafts = $state<Record<string, WsDraft>>({});
  /** Files whose save found them changed on disk, by `track\0path`. */
  saveConflicts = $state<Record<string, boolean>>({});
  /** Files being saved now, by `track\0path`. */
  saving = $state<Record<string, boolean>>({});
  /** The file whose close was asked while it had unsaved edits; a second close discards. */
  closeAsked = $state("");

  private persistWidth(key: string, value: number) {
    invoke("set_setting", { key, value: String(value) }).catch((err) => {
      this.lastError = String(err);
    });
  }

  setRailWidth(px: number, persist = false) {
    this.railWidth = Math.min(320, Math.max(160, Math.round(px)));
    if (persist) this.persistWidth("rail_width", this.railWidth);
  }

  /** Width of the designs column. Persisted. */
  designListWidth = $state(240);
  /** Width of the settings column. Persisted. */
  settingsNavWidth = $state(232);

  setDesignListWidth(px: number, persist = false) {
    this.designListWidth = Math.min(480, Math.max(200, Math.round(px)));
    if (persist) this.persistWidth("designlist_width", this.designListWidth);
  }

  setSettingsNavWidth(px: number, persist = false) {
    this.settingsNavWidth = Math.min(420, Math.max(200, Math.round(px)));
    if (persist) this.persistWidth("settings_nav_width", this.settingsNavWidth);
  }

  setTrackListWidth(px: number, persist = false) {
    this.trackListWidth = Math.min(480, Math.max(200, Math.round(px)));
    if (persist) this.persistWidth("tracklist_width", this.trackListWidth);
  }

  setPanelWidth(px: number, persist = false) {
    const max = Math.max(360, Math.floor(window.innerWidth * 0.7));
    this.panelWidth = Math.min(max, Math.max(320, Math.round(px)));
    if (persist) this.persistWidth("panel_width", this.panelWidth);
  }

  /** Show or hide the side panel; persisted. Opening refreshes it. */
  async setPanel(open: boolean) {
    this.panelOpen = open;
    if (open) void this.refreshWorkspace();
    try {
      await invoke("set_setting", { key: "panel", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  // ----- the track header above the conversation: open or folded away -----

  /**
   * Tracks whose header is folded to one line, by id. A track not listed is
   * open: the first time anyone sees a track, they should see what it is
   * for. Persisted per track, so one long brief folded away stays folded
   * without hiding the next track's.
   */
  headerFolded = $state<Record<string, boolean>>({});

  /** Whether a track's header is folded to one line. */
  isHeaderFolded(track = this.track): boolean {
    return !!this.headerFolded[track];
  }

  /** Fold a track's header away, or open it again; persisted. */
  setHeaderFolded(track: string, folded: boolean) {
    const { [track]: _was, ...rest } = this.headerFolded;
    this.headerFolded = folded ? { ...rest, [track]: true } : rest;
    invoke("set_setting", { key: HEADERS_SETTING, value: JSON.stringify(this.headerFolded) }).catch(tracing);
  }

  private async restoreHeaders() {
    try {
      const raw = await invoke<string | null>("get_setting", { key: HEADERS_SETTING });
      const parsed: unknown = raw ? JSON.parse(raw) : {};
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return;
      const out: Record<string, boolean> = {};
      for (const [id, folded] of Object.entries(parsed as Record<string, unknown>)) if (folded === true) out[id] = true;
      this.headerFolded = out;
    } catch (err) {
      tracing(err);
    }
  }

  /** Forget what the panel loaded; the next open track fills it again. */
  private clearWorkspace() {
    this.tree = [];
    this.git = null;
    this.diffs = {};
    this.diffPath = "";
    this.files = {};
    this.openFiles = [];
    this.activeFile = "";
    if (this.panelTab === "file") this.panelTab = "files";
  }

  /** Reload the tree, the git status and every open file. */
  async refreshWorkspace() {
    if (!this.track) return;
    await Promise.all([this.loadTree(), this.loadGit(), ...this.openFiles.map((p) => this.loadFile(p))]);
    if (this.diffPath) await this.loadDiff(this.diffPath);
  }

  async loadTree() {
    const track = this.track;
    if (!track) return;
    this.treeLoading = true;
    try {
      const entries = await invoke<WsEntry[]>("workspace_tree", { track });
      if (this.track === track) this.tree = entries;
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.treeLoading = false;
    }
  }

  async loadGit() {
    const track = this.track;
    if (!track) return;
    this.gitLoading = true;
    try {
      const git = await invoke<WsGit>("workspace_git_status", { track });
      if (this.track === track) {
        this.git = git;
        if (this.diffPath && !git.changes.some((c) => c.path === this.diffPath)) this.diffPath = "";
      }
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.gitLoading = false;
    }
  }

  /** Show one change's diff in the changes tab. */
  async loadDiff(path: string) {
    const track = this.track;
    const change = this.git?.changes.find((c) => c.path === path);
    if (!track || !change) return;
    this.diffPath = path;
    try {
      const text = await invoke<string>("workspace_git_diff", { track, path, untracked: change.untracked });
      if (this.track === track) this.diffs = { ...this.diffs, [path]: text };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  private async loadFile(path: string) {
    const track = this.track;
    if (!track) return;
    try {
      const file = await invoke<WsFile>("workspace_read", { track, path });
      if (this.track === track) this.files = { ...this.files, [path]: file };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Open a file in its own panel tab and show it. */
  /** A line to show in a file once it is open (a content search hit). */
  gotoLine = $state<{ path: string; line: number; seq: number } | null>(null);

  async openFile(path: string, line?: number) {
    if (line) {
      // The line is in the source: a markdown or HTML file shows it, not its preview.
      this.rawMarkdown = { ...this.rawMarkdown, [path]: true };
      this.gotoLine = { path, line, seq: (this.gotoLine?.seq ?? 0) + 1 };
    }
    if (!this.openFiles.includes(path)) this.openFiles = [...this.openFiles, path];
    this.activeFile = path;
    this.panelTab = "file";
    this.panelOpen = true;
    await this.loadFile(path);
  }

  private draftKey(path: string): string {
    return `${this.track}\0${path}`;
  }

  /**
   * The text to edit: the unsaved draft, else the file as loaded. Line
   * breaks are `\n`, as a textarea gives them back; saving restores the
   * file's own (CRLF stays CRLF).
   */
  textOf(path: string): string {
    return this.drafts[this.draftKey(path)]?.text ?? lf(this.files[path]?.text ?? "");
  }

  isDirty(path: string): boolean {
    return !!this.drafts[this.draftKey(path)];
  }

  isSaving(path: string): boolean {
    return !!this.saving[this.draftKey(path)];
  }

  hasConflict(path: string): boolean {
    return !!this.saveConflicts[this.draftKey(path)];
  }

  /** Keep an edit; back to the file's text, it is no longer a draft. */
  setDraft(path: string, text: string) {
    const key = this.draftKey(path);
    const base = this.drafts[key]?.base ?? this.files[path]?.text ?? "";
    if (text === lf(base)) {
      const { [key]: _gone, ...rest } = this.drafts;
      this.drafts = rest;
    } else {
      this.drafts = { ...this.drafts, [key]: { text, base } };
    }
    if (this.closeAsked === path) this.closeAsked = "";
  }

  /**
   * Write a draft to disk. When the file changed on disk since the edit
   * began, nothing is written and the file is marked in conflict, unless
   * `force`.
   */
  async saveFile(path: string, force = false) {
    const track = this.track;
    const key = this.draftKey(path);
    const draft = this.drafts[key];
    if (!track || !draft || this.saving[key]) return;
    this.saving = { ...this.saving, [key]: true };
    try {
      const file = await invoke<WsFile>("workspace_write", { track, path, text: draft.text, base: draft.base, force });
      // Edits typed while it saved stay a draft over the new base.
      const now = this.drafts[key];
      const { [key]: _saved, ...rest } = this.drafts;
      this.drafts = now && now.text !== draft.text ? { ...rest, [key]: { text: now.text, base: file.text ?? "" } } : rest;
      const { [key]: _c, ...conflicts } = this.saveConflicts;
      this.saveConflicts = conflicts;
      if (this.track === track) {
        this.files = { ...this.files, [path]: file };
        void this.loadGit();
      }
    } catch (err) {
      if (String(err).includes(CHANGED_ON_DISK)) this.saveConflicts = { ...this.saveConflicts, [key]: true };
      else this.lastError = String(err);
    } finally {
      const { [key]: _s, ...rest } = this.saving;
      this.saving = rest;
    }
  }

  /** Drop an unsaved edit and show the file as it is on disk. */
  async discardDraft(path: string) {
    const key = this.draftKey(path);
    const { [key]: _d, ...drafts } = this.drafts;
    this.drafts = drafts;
    const { [key]: _c, ...conflicts } = this.saveConflicts;
    this.saveConflicts = conflicts;
    await this.loadFile(path);
  }

  closeFile(path: string) {
    // Unsaved edits: the first close asks, the second discards.
    if (this.isDirty(path) && this.closeAsked !== path) {
      this.closeAsked = path;
      return;
    }
    if (this.closeAsked === path) this.closeAsked = "";
    const key = this.draftKey(path);
    const { [key]: _d, ...drafts } = this.drafts;
    this.drafts = drafts;
    const i = this.openFiles.indexOf(path);
    this.openFiles = this.openFiles.filter((p) => p !== path);
    const { [path]: _gone, ...rest } = this.files;
    this.files = rest;
    if (this.activeFile === path) {
      const next = this.openFiles[Math.min(i, this.openFiles.length - 1)];
      if (next) this.activeFile = next;
      else {
        this.activeFile = "";
        this.panelTab = "files";
      }
    }
  }

  /** Select a file in the system file manager. */
  async revealFile(path: string) {
    if (!this.track) return;
    try {
      await invoke("workspace_reveal", { track: this.track, path });
    } catch (err) {
      this.lastError = String(err);
    }
  }
  /** Every run of every track; views filter by `track`. */
  runs = $state<Run[]>([]);
  /** Every track, oldest first. */
  tracks = $state<Track[]>([]);
  /** Id of the track in the main area. Persisted. */
  track = $state("");
  /** Track whose tag dialog is open; "" means closed. */
  tagDialog = $state("");
  /** Slash commands the conductor's session offered, by track id. */
  commands = $state<Record<string, SlashCommand[]>>({});
  /** The last list seen per agent id, kept as a setting so the composer can complete before a session opens. */
  commandCache = $state<Record<string, SlashCommand[]>>({});

  /** Commands to complete in the composer: this track's session's, else the agent's last known. */
  /** The conversation on screen is an artifact's, not a track's conductor. */
  get chatArtifact(): boolean {
    return this.view === "design" && !!this.artifact;
  }

  /** The turns of the conversation on screen, oldest first. */
  get chatRuns(): Run[] {
    return this.chatArtifact ? this.artifactRuns : this.trackRuns.filter((r) => r.session === "conductor");
  }

  /** Decision cards of the conversation on screen; artifacts have none. */
  get chatDecisions(): Decision[] {
    if (this.chatArtifact) {
      const key = artifactKey(this.artifact);
      return this.decisions.filter((d) => d.track === key);
    }
    return this.trackDecisions;
  }

  get slashCommands(): SlashCommand[] {
    if (this.chatArtifact) return this.commandCache[this.currentArtifact?.agent ?? ""] ?? [];
    return this.commands[this.track] ?? this.commandCache[this.agent] ?? [];
  }

  /** The conductor session per track: open, busy. Read on track select, never opened by it. */
  conductor = $state<Record<string, ConductorState>>({});
  /** Track whose conductor is being opened right now. */
  conductorOpening = $state("");
  /** A cancel is on its way to the conductor. */
  cancelling = $state(false);

  get conductorState(): ConductorState {
    return this.conductor[this.track] ?? { open: false, busy: false, agent: null, commands: [] };
  }

  private takeConductorState(track: string, state: ConductorState) {
    this.conductor = { ...this.conductor, [track]: state };
    if (state.commands.length) this.rememberCommands(track, state.commands);
  }

  /** Whether a track's conductor session is open (active). */
  isActive(track: string): boolean {
    return !!this.conductor[track]?.open;
  }

  /** Ask how every conductor is, without touching any. Tracks not listed are closed. */
  async refreshConductor() {
    try {
      const list = await invoke<[string, ConductorState][]>("conductor_states");
      const next: Record<string, ConductorState> = {};
      for (const [track, state] of list) {
        next[track] = state;
        if (state.commands.length) this.rememberCommands(track, state.commands);
      }
      this.conductor = next;
    } catch (err) {
      tracing(err);
    }
  }

  /** Open a track's conductor session on purpose (activate): its commands become known and the first message is quick. */
  async openConductor(track = this.track) {
    if (!track || this.conductorOpening) return;
    this.conductorOpening = track;
    try {
      const state = await invoke<ConductorState>("conductor_open", { track });
      this.takeConductorState(track, state);
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.conductorOpening = "";
    }
  }

  /**
   * Stop the conductor's turn in flight, as Ctrl+C would; the run ends as
   * cancelled, keeping what it had said so far. Messages waiting in the line
   * are not thrown away: they go out as the next turn.
   */
  async cancelConductor() {
    if (this.chatArtifact) {
      this.cancelling = true;
      await this.artifactCancel();
      this.cancelling = false;
      return;
    }
    const track = this.track;
    if (!track || !this.busy) return;
    this.cancelling = true;
    try {
      await invoke("conductor_cancel", { track });
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.cancelling = false;
    }
  }

  /**
   * Which workers have an open agent session, per track. Workers with no
   * open session are not in here; the list itself comes from the runs, which
   * outlive sessions.
   */
  workerSessions = $state<Record<string, WorkerState[]>>({});
  /**
   * Turns waiting for a conductor: reports (and decision answers) it could
   * not take, kept in the record until it can. Read from the core, not
   * counted from the `parked` events — an event can be missed, and the
   * whole point of parking is that it survives the app closing.
   */
  parkedItems = $state<WaitingItem[]>([]);

  /** How many are waiting, per track, for the list's mark. */
  parked = $derived.by(() => {
    const out: Record<string, number> = {};
    for (const p of this.parkedItems) out[p.track] = (out[p.track] ?? 0) + 1;
    return out;
  });

  /** What is waiting for the conversation on screen. Artifacts have none. */
  get chatParked(): WaitingItem[] {
    return this.chatArtifact ? [] : this.parkedItems.filter((p) => p.track === this.track);
  }

  /** A worker's live session, if it has one. */
  workerSession(track: string, name: string): WorkerState | undefined {
    return this.workerSessions[track]?.find((w) => w.name === name);
  }

  /** Ask which worker sessions are open, without touching any. Tracks not listed have none. */
  async refreshWorkerSessions() {
    try {
      const list = await invoke<[string, WorkerState[]][]>("worker_sessions");
      this.workerSessions = Object.fromEntries(list);
    } catch (err) {
      tracing(err);
    }
  }

  /**
   * Stop a worker's turn in flight. The agent is really stopped, not just
   * hidden: its run ends and stays in the record marked stopped, and what it
   * had got to still reaches the conductor as a report.
   */
  async cancelWorker(track: string, name: string): Promise<string> {
    try {
      await invoke("worker_cancel", { track, worker: name });
    } catch (err) {
      return String(err);
    }
    await this.refreshWorkerSessions();
    return "";
  }

  /** Close a worker's session. Its record and memory stay. */
  async closeWorker(track: string, name: string): Promise<string> {
    try {
      await invoke<boolean>("worker_close", { track, worker: name });
    } catch (err) {
      return String(err);
    }
    await this.refreshWorkerSessions();
    return "";
  }

  /** Close every finished worker session of a track. Returns the names closed. */
  async tidyWorkers(track: string): Promise<{ closed: string[]; error: string }> {
    try {
      const closed = await invoke<string[]>("workers_tidy", { track });
      await this.refreshWorkerSessions();
      return { closed, error: "" };
    } catch (err) {
      return { closed: [], error: String(err) };
    }
  }

  /**
   * Delete a worker's record: its runs and the session the app would have
   * resumed. Its folder and every file in it stay untouched. Returns why the
   * core refused, or "" when it is gone.
   */
  async deleteWorker(track: string, name: string): Promise<string> {
    try {
      await invoke<number>("worker_delete", { track, worker: name });
    } catch (err) {
      return String(err);
    }
    this.runs = this.runs.filter((r) => !(r.track === track && r.session === name));
    this.workerSessions = { ...this.workerSessions, [track]: (this.workerSessions[track] ?? []).filter((w) => w.name !== name) };
    if (this.view === "worker" && this.track === track && this.openWorker === name) {
      this.openWorker = "";
      this.view = "track";
    }
    return "";
  }

  /** What is waiting for every conductor. The truth; the event is a nudge. */
  async loadParked() {
    try {
      this.parkedItems = await invoke<WaitingItem[]>("parked_deliveries");
    } catch (err) {
      tracing(err);
    }
  }

  /**
   * A track's waiting list changed: something was put aside, or something
   * waiting finally went in. The event says which track; the list itself is
   * read back from the core, so a missed event cannot leave the screen
   * saying one thing while the record says another.
   *
   * (The bell is not used: it is for decision cards waiting on an answer.
   * A report that is waiting shows in the conversation and on the track.)
   */
  takeParked(_w: WaitingDelivery) {
    void this.loadParked();
  }

  /** Close a track's conductor session (deactivate); it resumes with its memory next time. */
  async closeConductor(track = this.track) {
    if (!track) return;
    try {
      const state = await invoke<ConductorState>("conductor_close", { track });
      this.takeConductorState(track, state);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  private rememberCommands(track: string, list: SlashCommand[]) {
    this.commands = { ...this.commands, [track]: list };
    const agent = this.tracks.find((t) => t.id === track)?.agent;
    if (!agent) return;
    this.commandCache = { ...this.commandCache, [agent]: list };
    invoke("set_setting", { key: `commands:${agent}`, value: JSON.stringify(list) }).catch((err) => {
      this.lastError = String(err);
    });
  }
  /** The user's tags, each with a colour; persisted as the `tags` setting. */
  tagPool = $state<TagDef[]>([]);
  /** The wall clock, refreshed every half minute, so "today" and "yesterday" stay right. */
  now = $state(Date.now());
  /** The track list's filter, sort and fold. Persisted. */
  trackFilter = $state<TrackFilter>({ ...DEFAULT_TRACK_FILTER });

  /** Change part of the list's filter; persisted. */
  async setTrackFilter(patch: Partial<TrackFilter>) {
    this.trackFilter = { ...this.trackFilter, ...patch };
    try {
      await invoke("set_setting", { key: "tracks_filter", value: JSON.stringify(this.trackFilter) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** A tag's colour, or the neutral label colour for one not in the pool. */
  tagColor(name: string): string {
    return this.tagPool.find((t) => t.name === name)?.color ?? "var(--lab)";
  }

  private async persistTags() {
    try {
      await invoke("set_setting", { key: "tags", value: JSON.stringify(this.tagPool) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Make a tag with a colour drawn at random, avoiding ones already in use while possible. */
  async createTag(name: string): Promise<TagDef | undefined> {
    const clean = name.trim();
    if (!clean) return undefined;
    const existing = this.tagPool.find((t) => t.name === clean);
    if (existing) return existing;
    const used = new Set(this.tagPool.map((t) => t.color));
    const free = TRACK_COLORS.filter((c) => !used.has(c));
    const pick = free.length ? free : TRACK_COLORS;
    const tag = { name: clean, color: pick[Math.floor(Math.random() * pick.length)] };
    this.tagPool = [...this.tagPool, tag];
    await this.persistTags();
    return tag;
  }

  /** Drop a tag from the pool and from every track carrying it. */
  async deleteTag(name: string) {
    this.tagPool = this.tagPool.filter((t) => t.name !== name);
    await this.persistTags();
    for (const tr of this.tracks) {
      if (tr.tags.includes(name)) await this.updateTrack(tr.id, { tags: tr.tags.filter((x) => x !== name) });
    }
    for (const d of this.artifacts) {
      if (d.tags.includes(name)) await this.updateArtifact(d.id, { tags: d.tags.filter((x) => x !== name) });
    }
  }
  // ----- errors the human should see, wherever they happened -----
  //
  // `lastError` stays as the way to raise one: the fifty-odd call sites
  // that assign to it keep working untouched, which matters while other
  // people are editing the files they live in. Setting it to "" still
  // means "forget the last failure", as the handful of sites that do that
  // intend. The rules are in `errors.ts`; `ErrorToasts` shows the list.

  /** Errors on screen now, oldest first, newest last. */
  errors = $state<AppError[]>([]);
  private errorSeq = 0;
  /**
   * How many stay on screen. `addError` folds a repeat into the one
   * before it, but two faults taking turns (A, B, A, B) never fold, and
   * the stack has nowhere to go but up the screen. The oldest give way;
   * the log keeps all of them.
   */
  private static readonly SHOWN = 8;

  /** Raise an error for the human. Empty text raises nothing. */
  raise(text: string) {
    if (!text.trim()) return;
    const said = text.trim();
    // Two places, for two readers. The console is for whoever has it open
    // now; the app's log is for the bug report afterwards, since a toast is
    // gone in twenty seconds at the outside. Both get every occurrence,
    // including the repeats `addError` folds into a count.
    console.error(`[divixi] ${said}`);
    // Dropped on the floor if it fails, always. An error raised because an
    // error could not be recorded is a loop with nothing in it for anyone.
    void invoke("ui_log", { level: "error", message: said }).catch(() => {});
    this.errors = addError(this.errors, text, ++this.errorSeq, Date.now()).slice(-Store.SHOWN);
  }

  /** Take one error off the screen (the human read it, or closed it). */
  dismissError(id: number) {
    this.errors = dropError(this.errors, id);
  }

  /** Clear the screen of errors at once. */
  dismissErrors() {
    this.errors = [];
  }

  /** The newest error on screen, or "" — and the way every caller raises one. */
  get lastError(): string {
    return this.errors.at(-1)?.text ?? "";
  }

  set lastError(text: string) {
    // Raising only. Clearing used to live here too — `lastError = ""` at
    // the head of an operation, meaning "forget the last failure" — and
    // it threw away *every* error, including ones raised on screens the
    // caller knows nothing about. An error has its own life now (it
    // expires, it can be dismissed, hovering it stops the clock), so no
    // operation needs to reach in and end it early. Blank raises nothing:
    // `raise` ignores it.
    this.raise(text);
  }

  restored = $state(false);

  /** Last detection result; null until setup has run once. */
  agents = $state<AgentStatus[] | null>(null);
  /** Agent id workers are opened on. */
  agent = $state<AgentId>("claude_code");
  detecting = $state(false);
  /** Agent ids with a login or download in flight. */
  working = $state<Record<string, string>>({});
  /** Live download progress per agent id, while a download runs. */
  downloads = $state<Record<string, DownloadProgress>>({});

  get currentTrack(): Track | undefined {
    return this.tracks.find((t) => t.id === this.track);
  }

  /** Add files to the next message by path; folders and missing paths are skipped. */
  async attach(paths: string[], key = this.chatKey) {
    const waiting = () => this.attachmentsBy[key] ?? [];
    const fresh = paths.filter((p) => !waiting().some((a) => a.path === p));
    if (!fresh.length) return;
    try {
      const stats = await invoke<Attachment[]>("file_stats", { paths: fresh });
      // Into the conversation it was meant for, even if another is on screen now.
      const add = stats.filter((s) => !waiting().some((a) => a.path === s.path));
      this.attachmentsBy = { ...this.attachmentsBy, [key]: [...waiting(), ...add] };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The system file picker, opened in the track's folder. */
  async pickAttachments() {
    try {
      const paths = await invoke<string[]>("pick_files", { start: local ? (this.currentTrack?.cwd ?? null) : null });
      await this.attach(await bring(paths));
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** A pasted image (or file) has no path: keep it in the app's folder, then attach it. */
  async attachBlob(blob: Blob, name: string) {
    try {
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).replace(/^data:[^,]*,/, ""));
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(blob);
      });
      const path = await invoke<string>("save_attachment", { name, data });
      await this.attach([path]);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  detach(path: string) {
    this.attachments = this.attachments.filter((a) => a.path !== path);
  }

  /** A path inside the current track's folder, relative with "/", or null outside it. */
  relativeToTrack(path: string): string | null {
    // An artifact has no folder panel to open files in.
    if (this.chatArtifact) return null;
    const root = this.currentTrack?.cwd;
    if (!root) return null;
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
    const r = norm(root);
    const p = norm(path);
    const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
    if (p.length <= r.length + 1 || !same(p.slice(0, r.length), r) || p[r.length] !== "/") return null;
    return p.slice(r.length + 1);
  }

  /** A track-relative path ("src/a.ts") as an absolute one, in the folder's own separators. */
  absoluteInTrack(rel: string): string {
    const root = this.currentTrack?.cwd ?? "";
    const sep = root.includes("\\") ? "\\" : "/";
    return `${root.replace(/[\\/]+$/, "")}${sep}${rel.split("/").join(sep)}`;
  }

  // ----- artifacts -----

  /** The designs, most recently touched first. */
  get designs(): ArtifactInfo[] {
    return this.artifacts.filter((a) => a.kind === "design").sort((a, b) => b.updated_at - a.updated_at);
  }

  /**
   * Attach a design to the next message of a track: its board as a brief
   * (goals, constraints, questions, notes) and, when it has ink, a picture
   * of it — written out as files and attached like any other.
   */
  async attachDesign(id: string, labels: Record<string, string>) {
    const a = this.artifacts.find((x) => x.id === id);
    if (!a) return;
    const key = this.chatKey;
    this.attaching += 1;
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      const markdown = briefOf(a.title, doc, labels);
      const paths = await invoke<string[]>("export_artifact", { id, markdown, image: boardPng(doc) });
      await this.attach(paths, key);
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.attaching -= 1;
    }
  }

  get currentArtifact(): ArtifactInfo | undefined {
    return this.artifacts.find((d) => d.id === this.artifact);
  }

  /** The open artifact's conversation, oldest first. */
  get artifactRuns(): Run[] {
    const key = artifactKey(this.artifact);
    return this.runs.filter((r) => r.track === key);
  }

  get artifactBusy(): boolean {
    return this.artifactRuns.some((r) => r.status === "running" || r.status === "connecting");
  }

  async loadArtifacts() {
    try {
      this.artifacts = await invoke<ArtifactInfo[]>("list_artifacts", { kind: null });
    } catch (err) {
      this.lastError = String(err);
      // The list is unchanged, so it cannot speak for what is gone.
      return;
    }
    this.pruneQueued("artifact:", new Set(this.artifacts.map((a) => artifactKey(a.id))));
  }

  /** Show the designs: the last one open, else the newest. */
  async showDesigns() {
    if (this.view === "design") {
      await this.setDesignList(!this.designListOpen);
      return;
    }
    this.view = "design";
    if (!this.designListOpen) void this.setDesignList(true);
    if (!this.artifacts.length) await this.loadArtifacts();
    const pick = this.designs.find((d) => d.id === this.artifact) ?? this.designs[0];
    if (pick) await this.openArtifact(pick.id);
  }

  async openArtifact(id: string) {
    if (this.artifact !== id) this.designSelected = [];
    this.artifact = id;
    this.view = "design";
    void this.refreshArtifactSession();
    if (this.designLoaded !== id) {
      this.designLoaded = "";
      this.designDoc = { ...EMPTY_DOC };
    }
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      // An event may have brought a newer board meanwhile; keep that one.
      if (this.artifact === id && !(this.designLoaded === id && this.designDoc.version > doc.version)) {
        this.designDoc = doc;
        this.designLoaded = id;
      }
      void this.loadExtracts(id);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  async createDesign(title: string) {
    const agent = this.readyAgents.some((a) => a.kind === this.agent) ? this.agent : this.readyAgents[0]?.kind;
    if (!agent) {
      this.lastError = "no agent is ready";
      return;
    }
    try {
      const d = await invoke<ArtifactInfo>("create_artifact", { kind: "design", title, agent });
      this.artifacts = [d, ...this.artifacts];
      await this.openArtifact(d.id);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  async updateArtifact(id: string, patch: { title?: string; agent?: string; config?: OptionConfig; color?: string; tags?: string[] }) {
    try {
      const d = await invoke<ArtifactInfo>("update_artifact", { id, ...patch });
      this.artifacts = this.artifacts.map((x) => (x.id === id ? d : x));
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Delete an artifact. Returns why the core refused, or "" when it is gone. */
  async deleteArtifact(id: string): Promise<string> {
    try {
      await invoke("delete_artifact", { id });
    } catch (err) {
      return String(err);
    }
    this.artifacts = this.artifacts.filter((d) => d.id !== id);
    this.runs = this.runs.filter((r) => r.track !== artifactKey(id));
    this.setQueued(this.queued.filter((q) => q.key !== artifactKey(id)));
    if (this.artifact === id) {
      const next = this.designs[0];
      if (next) await this.openArtifact(next.id);
      else {
        this.artifact = "";
        this.designDoc = { ...EMPTY_DOC };
      }
    }
    return "";
  }

  /** Edit the open board. The core answers with the new board as an event. */
  async designApply(ops: DesignOp[]): Promise<DesignResult[]> {
    const id = this.artifact;
    if (!id || !ops.length) return [];
    try {
      const res = await invoke<DesignResult[]>("design_apply", { id, ops });
      const bad = res.find((r) => !r.ok);
      if (bad) this.lastError = bad.error ?? "the board edit did not go through";
      return res;
    } catch (err) {
      this.lastError = String(err);
      return [];
    }
  }

  /** Files from disk onto the open board, as cards from (x, y). */
  async designAddFiles(paths: string[], x: number, y: number) {
    const id = this.artifact;
    if (!id || !paths.length) return;
    try {
      const res = await invoke<DesignResult[]>("design_add_files", { id, paths, x, y });
      const bad = res.filter((r) => !r.ok);
      if (bad.length) this.lastError = bad.map((r) => r.error).join("\n");
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** A pasted image onto the open board. */
  async designAddBlob(blob: Blob, name: string, x: number, y: number) {
    const id = this.artifact;
    if (!id) return;
    try {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let binary = "";
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      const res = await invoke<DesignResult[]>("design_add_blob", { id, name, data: btoa(binary), x, y });
      const bad = res.find((r) => !r.ok);
      if (bad) this.lastError = bad.error ?? "the file was not added";
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Undo the human's last board edit, or redo with `again`. */
  async designUndo(again: boolean) {
    if (!this.artifact) return;
    try {
      await invoke<boolean>("design_undo", { id: this.artifact, again });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Pick files and put them on the open board at (x, y). */
  async designPickFiles(x: number, y: number) {
    try {
      const paths = await invoke<string[]>("pick_files", { start: null });
      await this.designAddFiles(await bring(paths), x, y);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Where a card shows a reference file of the open design. */
  boardFileUrl(src: string): string {
    return `${boardBase()}${encodeURIComponent(this.artifact)}/${encodeURIComponent(src.replace(/^files\//, ""))}`;
  }

  async designOpenFile(src: string) {
    try {
      await invoke("design_open_file", { id: this.artifact, src });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  async openUrl(url: string) {
    try {
      await invoke("open_url", { url });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The board's element, so a drop from the system lands on it rather than in the composer. */
  designBoardEl: HTMLElement | null = null;

  /** Whether a drop at a window position (physical pixels) is over the board. */
  overDesignBoard(pos: { x: number; y: number }): boolean {
    const el = this.designBoardEl;
    if (!el || this.view !== "design") return false;
    const r = el.getBoundingClientRect();
    const x = pos.x / devicePixelRatio;
    const y = pos.y / devicePixelRatio;
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }

  async designReview(changes: number[], keep: boolean) {
    if (!this.artifact || !changes.length) return;
    try {
      await invoke("design_review", { id: this.artifact, changes, keep });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Take what changed on a board. It applies to the open board when that
   *  is the version it was made on; a board that missed a change (or is
   *  still loading) is read again whole. */
  takeDesign(delta: DesignDelta) {
    const id = delta.id;
    const d = this.artifacts.find((x) => x.id === id);
    if (d) d.updated_at = Date.now();
    if (id !== this.artifact) return;
    // A new or changed card may have text to read now.
    void this.loadExtracts(id);
    if (this.designLoaded === id && delta.base === this.designDoc.version) {
      this.designDoc = withDelta(this.designDoc, delta);
    } else if (!(this.designLoaded === id && delta.version <= this.designDoc.version)) {
      void this.reloadDesign(id);
    }
  }

  /** Read the open board whole again, keeping whichever is newer. */
  private async reloadDesign(id: string) {
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      if (this.artifact === id && !(this.designLoaded === id && this.designDoc.version >= doc.version)) {
        this.designDoc = doc;
        this.designLoaded = id;
      }
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** One message to the artifact's agent: the composer's text and files, the
   *  picture of the board when it has ink, and the items picked on it. */
  private async dispatchArtifact(out: Queued): Promise<boolean> {
    const { target: id, agent, text: typed, files, picks, selected } = out;
    // As in `dispatchTrack`: the whole thing, text-building included, is
    // inside the try. `withKnowledge` and `boardPng` both reach into data
    // that may have come back from a setting.
    try {
      const typedWithKb = withKnowledge(typed, picks);
      // The board as it stands now, but only if it is still the one on
      // screen: a message queued here must not carry another's picture.
      const image = this.artifact === id ? boardPng(this.designDoc) : null;
      const text = withAttachments(typedWithKb, files);
      this.runs.push({
        id: `pending-${Date.now()}`,
        fromQueued: out.id,
        track: artifactKey(id),
        session: ARTIFACT_SESSION,
        agent,
        prompt: text,
        status: "connecting",
        startedAt: Date.now(),
        message: "",
        plan: [],
        toolCount: 0,
        loaded: true,
        thought: "",
        tools: [],
        transcript: [],
        segments: [],
      });
      const run = await invoke<string>("artifact_prompt", { id, text: typedWithKb, image, selected, lang: i18n.lang, files });
      const r = this.runs.find((x) => x.fromQueued === out.id);
      if (r) r.id = run;
      this.setQueued(dropQueued(this.queued, out.id));
      void this.refreshArtifactSession();
      return true;
    } catch (err) {
      this.forgetRun(out.id);
      if (notNow(err)) return false;
      this.returnToComposer(out, err);
      return true;
    }
  }

  async refreshArtifactSession() {
    const id = this.artifact;
    if (!id) return;
    try {
      const s = await invoke<{ open: boolean; busy: boolean }>("artifact_state", { id });
      if (this.artifact === id) this.artifactSession = s;
    } catch {
      // Cosmetic.
    }
  }

  async artifactCancel() {
    if (this.artifact) await invoke("artifact_cancel", { id: this.artifact }).catch(() => {});
  }

  /** Take a decision the core sent or returned, new or changed. */
  upsertDecision(d: Decision) {
    const i = this.decisions.findIndex((x) => x.id === d.id);
    if (i >= 0) this.decisions[i] = d;
    else this.decisions.push(d);
    // A new card waits for the human: the one thing the bell is for.
    if (i < 0 && d.status === "open") {
      const track = this.tracks.find((x) => x.id === d.track)?.name ?? "";
      notifyDecision(d.id, t(d.permission ? "notify.permission" : "notify.decision", { track }), d.question, d.track);
    }
    // Answered or set aside: nothing is waiting, so the notice goes.
    if (d.status !== "open") resolveDecisions([d.id]);
  }

  /** The current track's decisions, oldest first. */
  get trackDecisions(): Decision[] {
    return this.decisions.filter((d) => d.track === this.track);
  }

  /** How many of a track's decisions wait on the human. */
  openDecisions(track: string): number {
    return this.decisions.filter((d) => d.track === track && d.status === "open").length;
  }

  /** Answer a decision card; the core hands the answer to the conductor. */
  async answerDecision(id: number, choice: number | null, answer: string | null, note: string): Promise<boolean> {
    this.answering = id;
    try {
      this.upsertDecision(await invoke<Decision>("answer_decision", { id, choice, answer, note }));
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    } finally {
      this.answering = null;
    }
  }

  /** Set a decision aside; the conductor is not told. */
  async dismissDecision(id: number) {
    try {
      this.upsertDecision(await invoke<Decision>("dismiss_decision", { id }));
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The current track's runs, oldest first. */
  get trackRuns(): Run[] {
    return this.runs.filter((r) => r.track === this.track);
  }

  get activeRun(): Run | undefined {
    const runs = this.chatArtifact ? this.artifactRuns : this.trackRuns;
    return runs.find((r) => r.status === "running" || r.status === "connecting");
  }

  /**
   * Whether a conversation (`track:<id>` or `artifact:<id>`) has a turn in
   * flight. By key, not by what is on screen, so the waiting line of a track
   * the human has left is still pumped when its turn ends.
   */
  busyFor(key: string): boolean {
    const live = (r: Run) => r.status === "running" || r.status === "connecting";
    if (key.startsWith("artifact:")) return this.runs.some((r) => r.track === key && live(r));
    const track = key.slice("track:".length);
    return !!track && this.runs.some((r) => r.track === track && r.session === "conductor" && live(r));
  }

  /** The conversation on screen has a turn in flight; the composer shows a stop. */
  get busy(): boolean {
    return this.busyFor(this.chatKey);
  }

  /** Any track has a run in flight; the brand mark pulses. */
  get anyLive(): boolean {
    return this.runs.some((r) => r.status === "running" || r.status === "connecting");
  }

  /** Create a track and open it. */
  async createTrack(patch: TrackPatch): Promise<boolean> {
    try {
      const track = await invoke<Track>("create_track", { patch });
      this.tracks.push(track);
      await this.selectTrack(track.id);
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    }
  }

  /** Show a track in the main area; persisted so the app reopens on it. */
  async selectTrack(id: string) {
    const track = this.tracks.find((t) => t.id === id);
    if (!track) return;
    const changed = this.track !== id;
    this.track = id;
    this.view = "track";
    if (changed) {
      this.clearWorkspace();
      if (this.panelOpen) void this.refreshWorkspace();
      void this.refreshConductor();
      void this.refreshWorkerSessions();
    }
    if (this.readyAgents.some((a) => a.kind === track.agent)) this.agent = track.agent as AgentId;
    try {
      await invoke("set_setting", { key: "track", value: id });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Change a track's fields; the conductor picks up agent and option changes at its next message. */
  async updateTrack(id: string, patch: TrackPatch): Promise<boolean> {
    try {
      const next = await invoke<Track>("update_track", { id, patch });
      this.tracks = this.tracks.map((t) => (t.id === id ? next : t));
      if (id === this.track && this.readyAgents.some((a) => a.kind === next.agent)) this.agent = next.agent as AgentId;
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    }
  }

  /** Move the current track's conductor to another agent; it reopens there on the next message. */
  async setTrackAgent(agent: AgentId) {
    this.agent = agent;
    if (this.track) await this.updateTrack(this.track, { agent });
  }

  /** Delete a track with its runs and memory; the app moves to a neighbour or to creation. */
  /** Delete a track. Returns why the core refused, or "" when it is gone. */
  async deleteTrack(id: string): Promise<string> {
    try {
      await invoke("delete_track", { id });
    } catch (err) {
      return String(err);
    }
    resolveDecisions(this.decisions.filter((d) => d.track === id).map((d) => d.id));
    this.decisions = this.decisions.filter((d) => d.track !== id);
    this.tracks = this.tracks.filter((t) => t.id !== id);
    this.runs = this.runs.filter((r) => r.track !== id);
    this.setQueued(this.queued.filter((q) => q.key !== `track:${id}`));
    if (this.headerFolded[id]) this.setHeaderFolded(id, false);
    if (this.track === id) {
      const next = this.tracks.at(-1);
      if (next) await this.selectTrack(next.id);
      else {
        this.track = "";
        this.view = "new-track";
      }
    }
    return "";
  }

  /** Native folder picker; empty when cancelled. */
  async pickFolder(start: string): Promise<string> {
    try {
      return (await invoke<string | null>("pick_folder", { start })) ?? "";
    } catch (err) {
      this.lastError = String(err);
      return "";
    }
  }

  get readyAgents(): AgentStatus[] {
    return (this.agents ?? []).filter((a) => a.readiness === "ready");
  }

  get currentAgent(): AgentStatus | undefined {
    return this.agents?.find((a) => a.kind === this.agent);
  }

  /** The select options an agent advertised at detection (mode, model, effort, …). */
  optionsOf(agent: string): ConfigOption[] {
    return (this.agents?.find((a) => a.kind === agent)?.probe?.config_options ?? []).filter((o) => o.choices.length > 0);
  }

  /** The mode Orchestra picks for an agent when the track chooses none. */
  autonomousModeOf(agent: string): string {
    const status = this.agents?.find((a) => a.kind === agent);
    return status?.probe?.autonomous_mode ?? status?.probe?.config_options?.find((o) => o.category === "mode")?.current ?? "";
  }

  /** Value in effect for one of an agent's options: the track's choice, else the agent's current. */
  effective(agent: string, config: OptionConfig, option: ConfigOption): string {
    const chosen = config[option.id];
    if (chosen) return chosen;
    if (option.category === "mode") return this.autonomousModeOf(agent) || option.current;
    return option.current;
  }

  /** Human name of a choice, falling back to its id. */
  choiceName(option: ConfigOption | undefined, id: string): string {
    return option?.choices.find((c) => c.id === id)?.name ?? id;
  }

  /** Context accounting to show: the current track's conductor, live or its latest run that reported one. */
  get context(): Usage | undefined {
    const runs = this.chatRuns;
    const live = runs.find((r) => r.status === "running" || r.status === "connecting");
    return live?.usage ?? [...runs].reverse().find((r) => r.usage)?.usage;
  }

  /** The agent a role runs on in the current track. */
  roleAgent(role: Role): string {
    if (this.chatArtifact) return this.currentArtifact?.agent ?? this.agent;
    const track = this.currentTrack;
    if (!track) return this.agent;
    return role === "conductor" ? track.agent : workerAgentOf(track);
  }

  /** A role's chosen options in the current track. */
  roleConfig(role: Role): OptionConfig {
    if (this.chatArtifact) return this.currentArtifact?.config ?? {};
    const track = this.currentTrack;
    if (!track) return {};
    return role === "conductor" ? track.conductor_config : workerConfigOf(track);
  }

  /** Put a role on an agent. Takes effect at the next session. A worker
   *  moved onto the agent it already runs on keeps its options. */
  async setRoleAgent(role: Role, agent: string) {
    const d = this.chatArtifact ? this.currentArtifact : undefined;
    if (d) {
      // Options are in one agent's terms; another agent starts from its own.
      await this.updateArtifact(d.id, agent === d.agent ? { agent } : { agent, config: {} });
      return;
    }
    const track = this.currentTrack;
    if (!track) return;
    if (role === "conductor") {
      this.agent = agent as AgentId;
      await this.updateTrack(track.id, { agent });
    } else {
      const same = agent === workerAgentOf(track);
      await this.updateTrack(track.id, { worker_agent: agent, worker_config: same ? workerConfigOf(track) : {} });
    }
  }

  /** Set one of a role's options on the current track; empty clears it. Takes effect at the next session. */
  async setRoleOption(role: Role, id: string, value: string) {
    const d = this.chatArtifact ? this.currentArtifact : undefined;
    if (d) {
      const { [id]: _old, ...rest } = d.config;
      await this.updateArtifact(d.id, { config: value ? { ...rest, [id]: value } : rest });
      return;
    }
    const track = this.currentTrack;
    if (!track) return;
    const { [id]: _old, ...rest } = this.roleConfig(role);
    const next = value ? { ...rest, [id]: value } : rest;
    await this.updateTrack(track.id, role === "conductor" ? { conductor_config: next } : { worker_config: next });
  }

  /** Apply the preference to <html> and, for `system`, follow the OS. */
  private applyTheme() {
    const dark = this.themePref === "system" ? (scheme?.matches ?? true) : this.themePref === "dark";
    this.theme = dark ? "dk" : "lt";
    document.documentElement.className = this.theme;
  }

  /** Choose a theme; persisted, applied at once. */
  async setTheme(pref: ThemePref) {
    this.themePref = pref;
    this.applyTheme();
    try {
      await invoke("set_setting", { key: "theme", value: pref });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Title-bar shortcut: flip between the explicit themes. */
  toggleTheme() {
    void this.setTheme(this.theme === "dk" ? "light" : "dark");
  }

  /** Fold or unfold the rail; persisted. */
  async setRail(collapsed: boolean) {
    this.railCollapsed = collapsed;
    try {
      await invoke("set_setting", { key: "rail", value: collapsed ? "collapsed" : "expanded" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Show a worker's session: the conductor's messages and the worker's replies. */
  async openWorkerView(name: string) {
    this.openWorker = name;
    this.view = "worker";
    // Restored runs only carry their folded text; replay them for the full turn.
    await Promise.all(this.trackRuns.filter((r) => r.session === name && !r.loaded).map((r) => this.hydrate(r)));
  }

  /** The view settings were opened from, to go back to. */
  settingsFrom: View = "track";

  /** Open settings on a section. */
  openSettings(section: SettingsSection = "overview") {
    if (this.view !== "settings") this.settingsFrom = this.view;
    this.settingsSection = section;
    this.view = "settings";
    void this.loadInfo();
  }

  /** Close settings, back to the view they were opened from. */
  closeSettings() {
    this.view = this.settingsFrom === "settings" ? "track" : this.settingsFrom;
  }

  async loadInfo() {
    try {
      this.info = await invoke<AppInfo>("app_info");
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The language in effect. */
  get lang(): Lang {
    return i18n.lang;
  }

  /** Choose the interface language; persisted, applied at once. */
  async setLang(pref: LangPref) {
    this.langPref = pref;
    i18n.lang = pref === "system" ? systemLang() : pref;
    document.documentElement.lang = i18n.lang;
    try {
      await invoke("set_setting", { key: "language", value: pref });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Interface typeface; persisted, applied through a root attribute. */
  async setUiFont(font: UiFont) {
    this.uiFont = font;
    document.documentElement.dataset.uiFont = font;
    try {
      await invoke("set_setting", { key: "ui_font", value: font });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /**
   * Zoom the whole interface, like Ctrl+= / Ctrl+- in a browser. Uses the
   * webview's native zoom; if that is refused, CSS zoom on the root.
   */
  async setZoom(percent: number, persist = true) {
    const z = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(percent / ZOOM_STEP) * ZOOM_STEP));
    this.zoom = z;
    try {
      await getCurrentWebview().setZoom(z / 100);
      (document.documentElement.style as unknown as { zoom: string }).zoom = "";
    } catch {
      (document.documentElement.style as unknown as { zoom: string }).zoom = `${z}%`;
    }
    if (!persist) return;
    try {
      await invoke("set_setting", { key: "zoom", value: String(z) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Conversation text size; persisted, applied through a root attribute. */
  async setChatFont(size: ChatFont) {
    this.chatFont = size;
    document.documentElement.dataset.chatFont = size;
    try {
      await invoke("set_setting", { key: "chat_font", value: size });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Show or hide the track list column; persisted. */
  async setTerminal(open: boolean) {
    if (!inTauri) return;
    this.termOpen = open;
    if (open) this.termMounted = true;
    try {
      await invoke("set_setting", { key: "terminal", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  setTermHeight(px: number, persist = false) {
    const max = Math.max(160, window.innerHeight - 220);
    this.termHeight = Math.min(max, Math.max(120, Math.round(px)));
    if (persist) this.persistWidth("terminal_height", this.termHeight);
  }

  async setTrackList(open: boolean) {
    this.trackListOpen = open;
    try {
      await invoke("set_setting", { key: "tracklist", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Rebuild the timeline, agent list and preferences from the store. Called once at startup. */
  async restore() {
    scheme?.addEventListener("change", () => {
      if (this.themePref === "system") this.applyTheme();
    });
    try {
      const [summaries, tracks, savedTrack, agents, theme, rail, tracklist, chatFont, panelWidth, railWidth, trackListWidth, uiFont, zoom, language, panel, tagsJson, ...commandJson] =
        await Promise.all([
        invoke<RunSummary[]>("list_runs"),
        invoke<Track[]>("list_tracks"),
        invoke<string | null>("get_setting", { key: "track" }),
        invoke<AgentStatus[] | null>("agent_statuses"),
        invoke<string | null>("get_setting", { key: "theme" }),
        invoke<string | null>("get_setting", { key: "rail" }),
        invoke<string | null>("get_setting", { key: "tracklist" }),
        invoke<string | null>("get_setting", { key: "chat_font" }),
        invoke<string | null>("get_setting", { key: "panel_width" }),
        invoke<string | null>("get_setting", { key: "rail_width" }),
        invoke<string | null>("get_setting", { key: "tracklist_width" }),
        invoke<string | null>("get_setting", { key: "ui_font" }),
        invoke<string | null>("get_setting", { key: "zoom" }),
        invoke<string | null>("get_setting", { key: "language" }),
        invoke<string | null>("get_setting", { key: "panel" }),
        invoke<string | null>("get_setting", { key: "tags" }),
        ...AGENT_IDS.map((id) => invoke<string | null>("get_setting", { key: `commands:${id}` })),
      ]);
      try {
        const saved = await invoke<string | null>("get_setting", { key: "tracks_filter" });
        if (saved) this.trackFilter = { ...DEFAULT_TRACK_FILTER, ...(JSON.parse(saved) as Partial<TrackFilter>) };
      } catch {
        this.trackFilter = { ...DEFAULT_TRACK_FILTER };
      }
      // Messages the human wrote but never sent, from the last time the app
      // was open. They come back held; nothing is sent by opening the app.
      await this.restoreQueue();
      await this.restoreHeaders();
      setInterval(() => (this.now = Date.now()), 30_000);
      const cache: Record<string, SlashCommand[]> = {};
      AGENT_IDS.forEach((id, i) => {
        try {
          const list = commandJson[i] ? (JSON.parse(commandJson[i] as string) as SlashCommand[]) : [];
          if (Array.isArray(list)) cache[id] = list;
        } catch {
          // A stale cache is no cache.
        }
      });
      this.commandCache = cache;
      if (language === "system" || language === "ko" || language === "en") this.langPref = language;
      i18n.lang = this.langPref === "system" ? systemLang() : this.langPref;
      document.documentElement.lang = i18n.lang;
      if (uiFont === "sans" || uiFont === "mono" || uiFont === "system" || uiFont === "serif") this.uiFont = uiFont;
      document.documentElement.dataset.uiFont = this.uiFont;
      const z = Number(zoom);
      if (Number.isFinite(z) && z >= ZOOM_MIN && z <= ZOOM_MAX && z !== 100) void this.setZoom(z, false);
      const w = Number(panelWidth);
      if (Number.isFinite(w) && w > 0) this.setPanelWidth(w);
      this.panelOpen = panel === "open";
      try {
        const pool = tagsJson ? (JSON.parse(tagsJson) as unknown) : [];
        if (Array.isArray(pool)) this.tagPool = pool.filter((t) => t && typeof t.name === "string" && typeof t.color === "string");
      } catch {
        this.tagPool = [];
      }
      // Tags on tracks that predate the pool (or lost it) still get a colour.
      for (const tr of tracks) for (const tag of tr.tags) if (!this.tagPool.some((t) => t.name === tag)) void this.createTag(tag);
      const rw = Number(railWidth);
      if (Number.isFinite(rw) && rw > 0) this.setRailWidth(rw);
      const tw = Number(trackListWidth);
      if (Number.isFinite(tw) && tw > 0) this.setTrackListWidth(tw);
      if (theme === "system" || theme === "dark" || theme === "light") this.themePref = theme;
      this.applyTheme();
      this.railCollapsed = rail === "collapsed";
      this.trackListOpen = tracklist !== "closed";
      if (chatFont === "s" || chatFont === "m" || chatFont === "l") this.chatFont = chatFont;
      document.documentElement.dataset.chatFont = this.chatFont;
      // The core closes runs left live by a previous process, so nothing
      // restored can be in flight.
      this.runs = summaries.map(fromSummary);
      this.tracks = tracks;
      this.pruneQueued("track:", new Set(tracks.map((tr) => `track:${tr.id}`)));
      void this.loadArtifacts();
      try {
        this.decisions = await invoke<Decision[]>("list_decisions", { track: null });
        // Cards answered while this webview was away leave no notice behind.
        keepOnlyOpen(this.decisions.filter((d) => d.status === "open").map((d) => d.id));
      } catch (err) {
        this.lastError = String(err);
      }
      try {
        const [term, termHeight, artifactChat, designList, designListWidth, settingsNavWidth] = await Promise.all([
          invoke<string | null>("get_setting", { key: "terminal" }),
          invoke<string | null>("get_setting", { key: "terminal_height" }),
          invoke<string | null>("get_setting", { key: "artifact_chat_width" }),
          invoke<string | null>("get_setting", { key: "designlist" }),
          invoke<string | null>("get_setting", { key: "designlist_width" }),
          invoke<string | null>("get_setting", { key: "settings_nav_width" }),
        ]);
        const dc = Number(artifactChat);
        if (Number.isFinite(dc) && dc > 0) this.setArtifactChatWidth(dc);
        const dl = Number(designListWidth);
        if (Number.isFinite(dl) && dl > 0) this.setDesignListWidth(dl);
        const sn = Number(settingsNavWidth);
        if (Number.isFinite(sn) && sn > 0) this.setSettingsNavWidth(sn);
        this.designListOpen = designList !== "closed";
        const h = Number(termHeight);
        if (Number.isFinite(h) && h > 0) this.setTermHeight(h);
        if (term === "open") void this.setTerminal(true);
      } catch {
        // Defaults are fine.
      }
      this.agents = agents;
      this.pickDefaultAgent();
      // Reopen on the track that was open, else the newest; none means the
      // first screen is creating one.
      const current = tracks.find((t) => t.id === savedTrack) ?? tracks.at(-1);
      if (current) {
        this.track = current.id;
        if (this.readyAgents.some((a) => a.kind === current.agent)) this.agent = current.agent as AgentId;
        if (this.panelOpen) void this.refreshWorkspace();
        void this.refreshConductor();
        void this.refreshWorkerSessions();
      } else {
        this.view = "new-track";
      }
      // Reports a conductor could not take before the app last closed are
      // still waiting; the track list says so.
      void this.loadParked();
      // Setup comes first when nothing has been detected yet, or when the
      // last detection left nothing to run workers on.
      if (local && (agents === null || this.readyAgents.length === 0)) {
        this.setupStep = 1;
        this.setupOpen = true;
      }
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.restored = true;
    }
  }

  /** Keep the selected agent on one that is ready. */
  private pickDefaultAgent() {
    const ready = this.readyAgents;
    if (ready.some((a) => a.kind === this.agent)) return;
    if (ready.length) this.agent = ready[0].kind;
  }

  /** Launch and probe every agent. Several seconds. */
  async detect() {
    if (this.detecting) return;
    this.detecting = true;
    try {
      this.agents = await invoke<AgentStatus[]>("detect_agents");
      this.pickDefaultAgent();
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.detecting = false;
    }
  }

  /** Run the agent's own ACP login flow. May open a browser. */
  async login(agent: AgentId, method?: string) {
    await this.workOn(agent, t("agents.loggingIn"), () => invoke<AgentStatus>("login_agent", { agent, method: method ?? null }));
  }

  /** Fetch the agent's ACP server (Antigravity). Progress arrives as events. */
  async download(agent: AgentId) {
    this.downloads = { ...this.downloads, [agent]: { agent, phase: "downloading", received: 0, total: null } };
    try {
      await this.workOn(agent, t("agents.downloading"), () => invoke<AgentStatus>("download_agent", { agent }));
    } finally {
      const { [agent]: _done, ...rest } = this.downloads;
      this.downloads = rest;
    }
  }

  /** Fold a progress event in. */
  progress(p: DownloadProgress) {
    if (!(p.agent in this.downloads)) return;
    this.downloads = { ...this.downloads, [p.agent]: p };
  }

  private async workOn(agent: AgentId, label: string, op: () => Promise<AgentStatus>) {
    if (this.working[agent]) return;
    this.working = { ...this.working, [agent]: label };
    try {
      const next = await op();
      this.agents = (this.agents ?? []).map((a) => (a.kind === agent ? next : a));
      if (!this.agents.some((a) => a.kind === agent)) this.agents = [...this.agents, next];
      this.pickDefaultAgent();
    } catch (err) {
      this.lastError = String(err);
    } finally {
      const { [agent]: _done, ...rest } = this.working;
      this.working = rest;
    }
  }

  /** Replay a run's event log into its below-membrane fields. */
  async hydrate(run: Run) {
    try {
      const events = await invoke<StoredEvent[]>("run_events", { run: run.id });
      // Replaying folds the message back together; start from empty so the
      // summary text is not doubled.
      run.message = "";
      run.thought = "";
      run.tools = [];
      run.plan = [];
      run.transcript = [];
      run.segments = [];
      for (const e of events) fold(run, e.at_ms, e.event);
      run.loaded = true;
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /**
   * Send a message to the conductor on the selected agent (or, in a design,
   * to the artifact's agent). The conductor decides whether to answer or to
   * open workers; worker runs arrive as `agent` events with their own run
   * ids and are added when first seen.
   *
   * Sent during a turn, the message waits its turn instead of being refused:
   * it joins the conversation at once with a "queued" badge and goes out by
   * itself when the turn ends, stopped or not. See `pumpQueue`.
   */
  async send(prompt: string) {
    const key = this.chatKey;
    const out = this.compose(prompt, key);
    if (!out) return;
    // A turn is in flight, or messages are already waiting: this one takes
    // its place in the line and goes out by itself when the turn ends. The
    // composer never closes, so the human can keep typing either way.
    // (Messages held from a previous run of the app are not in that line;
    // they wait on the human, so they do not hold this one up.)
    if (this.busyFor(key) || liveQueuedFor(this.queued, key).length) {
      this.setQueued(pushQueued(this.queued, out));
      this.startPump();
      return;
    }
    try {
      await this.dispatch(out);
    } catch (err) {
      // Same reasoning as the pump's catch, for the path that never
      // reached the line: the composer is already empty by now.
      this.setQueued(pushQueued(this.queued, { ...out, held: true }));
      this.raise(String(err));
    }
  }

  // ----- the waiting line: messages sent while a turn was in flight -----

  /** Messages waiting for their conversation's turn to end, oldest first. */
  queued = $state<Queued[]>([]);
  private queueSeq = 0;
  /** Conversations whose next message is on its way out right now. */
  private handing = new Set<string>();
  /** A slow beat that pumps the line even if an event was missed. */
  private pumpTimer: ReturnType<typeof setInterval> | null = null;
  /** A write of the line to its setting, waiting to be coalesced. */
  private queueSaving: ReturnType<typeof setTimeout> | null = null;

  /** The waiting messages of the conversation on screen. */
  get chatQueue(): Queued[] {
    const sent = this.chatRuns.map((r) => r.fromQueued).filter((x): x is string => !!x);
    return stillWaiting(this.queued, this.chatKey, sent);
  }

  /**
   * Change the line and write it down. Everything that touches `queued`
   * goes through here: what the human typed is theirs, and closing the app
   * must not be enough to lose it.
   */
  private setQueued(list: Queued[]) {
    this.queued = list;
    if (this.queueSaving !== null) return;
    // A pump can move several messages in one tick; one write covers them
    // all. No longer than that: the window where closing the app would
    // lose what was just typed should be as near to nothing as it can be.
    this.queueSaving = setTimeout(() => {
      this.queueSaving = null;
      const value = this.queued.length ? JSON.stringify(this.queued) : "";
      invoke("set_setting", { key: QUEUE_SETTING, value }).catch(tracing);
    }, 0);
  }

  /**
   * Read the line back at startup. Everything comes back held: it sits in
   * its conversation marked unsent, with a send button, and goes nowhere
   * until the human presses it. Sending by itself would be a surprise —
   * the app may have been shut for days, the folder has moved on, and the
   * message would spend an agent turn nobody asked for just now.
   */
  private async restoreQueue() {
    try {
      const list = parseQueued(await invoke<string | null>("get_setting", { key: QUEUE_SETTING }));
      // Ids are unique already (they carry the moment they were made); this
      // only keeps the counter ahead of what came back.
      this.queueSeq = list.length;
      this.queued = list;
    } catch (err) {
      tracing(err);
    }
  }

  /**
   * Drop waiting messages whose conversation is gone: a message with
   * nowhere to show would sit in the setting for good, written back on
   * every change. Called once the tracks are known and again once the
   * artifacts are, each pruning only the keys it can speak for — a list
   * that has not loaded yet must not condemn anything.
   */
  private pruneQueued(prefix: "track:" | "artifact:", alive: Set<string>) {
    const stale = this.queued.filter((q) => q.key.startsWith(prefix) && !alive.has(q.key));
    if (stale.length) this.setQueued(this.queued.filter((q) => !stale.includes(q)));
  }

  /** Send a held message after all: it rejoins the line and goes out. */
  releaseQueued(id: string) {
    if (!this.queued.some((q) => q.id === id)) return;
    this.setQueued(released(this.queued, id));
    this.startPump();
    void this.pumpQueue();
  }

  /**
   * Take what the composer holds — the typed text, its files, the knowledge
   * picked, the board items — as one message to send now or in a moment.
   * The composer is left empty, as it is on a plain send.
   */
  private compose(prompt: string, key: string): Queued | null {
    const text = prompt.trim();
    const files = this.attachments.map((a) => a.path);
    const artifact = this.chatArtifact;
    const selected = artifact ? [...this.designSelected] : [];
    const target = artifact ? this.artifact : this.track;
    if (!target) return null;
    if (artifact && !this.currentArtifact) return null;
    if (!text && !files.length && !this.kbPicked.length && !selected.length) return null;
    this.attachments = [];
    const picks = this.takeKnowledge(key);
    return {
      id: `q${++this.queueSeq}-${Date.now()}`,
      key,
      target,
      agent: artifact ? (this.currentArtifact?.agent ?? this.agent) : this.agent,
      text,
      files,
      picks,
      selected,
      at: Date.now(),
    };
  }

  /** Take a waiting message back out of the line before it goes. */
  cancelQueued(id: string) {
    const item = this.queued.find((q) => q.id === id);
    if (!item) return;
    this.setQueued(dropQueued(this.queued, id));
    // What it carried goes back to the composer, when nothing waits there.
    if (item.files.length && !(this.attachmentsBy[item.key] ?? []).length) void this.attach(item.files, item.key);
    this.restoreKnowledge(item.key, item.picks);
    if (!liveQueued(this.queued).length) this.stopPump();
  }

  private startPump() {
    if (this.pumpTimer !== null) return;
    this.pumpTimer = setInterval(() => void this.pumpQueue(), 400);
  }

  private stopPump() {
    if (this.pumpTimer === null) return;
    clearInterval(this.pumpTimer);
    this.pumpTimer = null;
  }

  /**
   * Send the oldest waiting message of every conversation that is free. The
   * beat is a safety net; `apply` calls this the moment a turn ends, so a
   * queued message usually goes out at once. Held messages are passed over:
   * they wait on the human, and the beat stops when only they are left.
   */
  async pumpQueue() {
    if (!liveQueued(this.queued).length) {
      this.stopPump();
      return;
    }
    for (const key of queuedKeys(this.queued)) {
      if (this.handing.has(key) || this.busyFor(key)) continue;
      const { next, rest } = nextQueued(this.queued, key);
      if (!next) continue;
      this.handing.add(key);
      this.setQueued(rest);
      void this.dispatch(next)
        .then((sent) => {
          // The core was still answering (a worker report got in first):
          // back to the front of the line, to try again on the next beat.
          if (!sent) {
            this.setQueued(unshiftQueued(this.queued, next));
            this.startPump();
          }
        })
        .catch((err) => {
          // Nothing should reach here — `dispatch` catches its own — but a
          // message out of the line and out of the store is a sentence the
          // human wrote and will never see again. Held, so it waits on
          // them rather than retrying straight back into the same fault.
          this.setQueued(unshiftQueued(this.queued, { ...next, held: true }));
          this.raise(String(err));
        })
        .finally(() => this.handing.delete(key));
    }
  }

  /** Put one message on its way. `false` means the core was busy: try again. */
  private async dispatch(out: Queued): Promise<boolean> {
    return out.key.startsWith("artifact:") ? this.dispatchArtifact(out) : this.dispatchTrack(out);
  }

  /** What a message that did not go out leaves behind in the composer. */
  private returnToComposer(out: Queued, err: unknown) {
    if (out.files.length && !(this.attachmentsBy[out.key] ?? []).length) void this.attach(out.files, out.key);
    this.restoreKnowledge(out.key, out.picks);
    this.lastError = String(err);
  }

  private async dispatchTrack(out: Queued): Promise<boolean> {
    const { target: track, agent, text: typed, files, picks } = out;
    // Everything is inside the try, including building the text. A saved
    // message can come back with a passage the app cannot read, and
    // `withKnowledge` reaching into it would throw out here, past every
    // catch, taking what the human wrote with it.
    try {
      const typedWithKb = withKnowledge(typed, picks);
      const text = withAttachments(typedWithKb, files);

      // Show the message the moment Enter is pressed. The run gets its real
      // id when the core answers; until then it carries a pending one, and
      // events that arrive first are routed to it by `apply`.
      this.runs.push({
        id: `pending-${Date.now()}`,
        fromQueued: out.id,
        track,
        session: "conductor",
        agent,
        prompt: text,
        status: "connecting",
        startedAt: Date.now(),
        message: "",
        plan: [],
        toolCount: 0,
        loaded: true,
        thought: "",
        tools: [],
        transcript: [],
        segments: [],
      });

      const id = await invoke<string>("conductor_prompt", { track, prompt: typedWithKb, agent, lang: i18n.lang, files });
      // By `fromQueued`, never by the id we set: `apply` may have handed
      // this run to another turn while we waited.
      const run = this.runs.find((r) => r.fromQueued === out.id);
      if (run) {
        run.id = id;
        run.track = track;
        run.session = "conductor";
        run.agent = agent;
        run.prompt = text;
      }
      // On its way: out of the line for good, however it got back in.
      this.setQueued(dropQueued(this.queued, out.id));
      // The conductor now runs on this agent; keep the track's record in step.
      const tr = this.tracks.find((t) => t.id === track);
      if (tr && tr.agent !== agent) tr.agent = agent;
      return true;
    } catch (err) {
      this.forgetRun(out.id);
      if (notNow(err)) return false;
      this.returnToComposer(out, err);
      return true;
    }
  }

  /** Take back the turn we made for a message that did not go. */
  private forgetRun(queued: string) {
    this.runs = this.runs.filter((r) => r.fromQueued !== queued);
  }

  /** Fold one live agent event into the run it belongs to, creating worker runs on first sight. */
  apply(env: Envelope) {
    let run = this.runs.find((r) => r.id === env.run);
    if (!run && (env.session === "conductor" || env.session === ARTIFACT_SESSION)) {
      // The conductor (or artifact agent) turn we just sent, still waiting for its id.
      run = this.runs.find((r) => r.track === env.track && r.session === env.session && r.id.startsWith("pending-"));
      if (run) run.id = env.run;
    }
    if (!run) {
      // A worker the conductor opened, or a report turn Orchestra injected:
      // the core registered it, we have not. Show it now; the prompt text
      // comes with the summary right after.
      run = {
        id: env.run,
        track: env.track,
        session: env.session,
        agent: this.tracks.find((t) => t.id === env.track)?.agent ?? this.agent,
        prompt: "",
        status: "connecting",
        startedAt: Date.now(),
        message: "",
        plan: [],
        toolCount: 0,
        loaded: true,
        thought: "",
        tools: [],
        transcript: [],
        segments: [],
      };
      this.runs.push(run);
      void this.refreshRun(env.run);
    }
    if (env.event.kind === "commands" && env.session === "conductor") this.rememberCommands(env.track, env.event.commands);
    if (env.session === ARTIFACT_SESSION && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshArtifactSession();
    }
    if (env.session === "conductor" && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshConductor();
    }
    // A worker began or ended a turn: the list's dots and its menu follow.
    if (env.session !== "conductor" && env.session !== ARTIFACT_SESSION && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshWorkerSessions();
    }
    fold(run, env.at_ms, env.event);
    // The conductor answering, or its turn failing, makes no notice: the
    // bell is only for decision cards, and the conversation shows both
    // where the human is already looking.
    // A finished run may have written files: the open panel catches up.
    if (this.panelOpen && env.track === this.track && (env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshWorkspace();
    }
    // The turn is over (answered, failed, or stopped by the human): whatever
    // was typed meanwhile goes out now, in the order it was typed.
    if (liveQueued(this.queued).length && (env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.pumpQueue();
    }
  }

  /** Fill a worker run's prompt and agent from the store once it exists there. */
  private async refreshRun(id: string) {
    try {
      const s = await invoke<RunSummary | null>("get_run", { id });
      const run = this.runs.find((r) => r.id === id);
      if (s && run) {
        run.prompt = s.prompt;
        run.agent = s.agent;
        run.startedAt = s.started_at;
        if (s.prompt.startsWith(REPORT_PREFIX)) this.reportsArrived++;
      }
    } catch {
      // Cosmetic; the next restore fills it in.
    }
  }

  /** A conductor handing over to another agent, by track, while its note is written. */
  handoffs = $state<Record<string, { from: string; to: string }>>({});

  takeHandoff(h: { track: string; from: string; to: string; writing: boolean }) {
    const { [h.track]: _gone, ...rest } = this.handoffs;
    this.handoffs = h.writing ? { ...rest, [h.track]: { from: h.from, to: h.to } } : rest;
  }

  /** Worker reports by run id, as far as they have been read. */
  reports = $state<Record<string, WorkerReport>>({});
  /** Moves when a report reaches a conductor, so cards still waiting ask again. */
  reportsArrived = $state(0);
  private reportsAsked = new Set<string>();

  async loadReport(run: string) {
    // One question in flight per run; a miss is asked again when a report arrives.
    if (this.reportsAsked.has(run)) return;
    this.reportsAsked.add(run);
    try {
      const report = await invoke<WorkerReport | null>("worker_report", { run });
      if (report) this.reports = { ...this.reports, [run]: report };
    } catch {
      // Cosmetic: the card stays away.
    } finally {
      this.reportsAsked.delete(run);
    }
  }

  /** Workers' unmerged changes, by `track/worker`. */
  workerChanges = $state<Record<string, WorkerChanges>>({});

  async loadWorkerChanges(track: string, worker: string) {
    try {
      const c = await invoke<WorkerChanges>("worker_changes", { track, worker });
      this.workerChanges = { ...this.workerChanges, [`${track}/${worker}`]: c };
    } catch {
      // Cosmetic: no changes section.
    }
  }

  async workerFileDiff(track: string, worker: string, path: string): Promise<string> {
    try {
      return await invoke<string>("worker_file_diff", { track, worker, path });
    } catch (err) {
      return String(err);
    }
  }

  /** Bring a worker's changes into the track folder; the panel catches up. */
  async mergeWorker(track: string, worker: string): Promise<WorkerMerged> {
    const merged = await invoke<WorkerMerged>("worker_merge", { track, worker });
    await this.loadWorkerChanges(track, worker);
    if (merged.files.length && this.panelOpen && this.track === track) void this.refreshWorkspace();
    return merged;
  }

  async discardWorker(track: string, worker: string) {
    await invoke("worker_discard", { track, worker });
    await this.loadWorkerChanges(track, worker);
  }

  /** Worker names seen in a track, in first-seen order, excluding the conductor. */
  workerNamesIn(track: string): string[] {
    const seen: string[] = [];
    for (const r of this.runs) if (r.track === track && r.session !== "conductor" && !seen.includes(r.session)) seen.push(r.session);
    return seen;
  }

  get workerNames(): string[] {
    return this.workerNamesIn(this.track);
  }
}

/** Pure state transition: the same fold serves live events and replay. */
function fold(run: Run, ms: number, ev: AgentEvent) {
  switch (ev.kind) {
    case "connected":
      push(run, ms, "connect", `protocol ${ev.protocol} · loadSession=${ev.load_session}`, "ok");
      break;
    case "started":
      run.status = "running";
      run.sessionId = ev.session_id;
      run.cwd = ev.cwd;
      push(run, ms, "session", `${ev.session_id}  cwd=${ev.cwd}`, "out");
      break;
    case "message": {
      run.message += ev.text;
      const last = run.segments.at(-1);
      if (last && last.kind === "text") last.text += ev.text;
      else run.segments.push({ kind: "text", text: ev.text });
      break;
    }
    case "thought": {
      run.thought += ev.text;
      const last = run.segments.at(-1);
      if (last && last.kind === "thought") last.text += ev.text;
      else run.segments.push({ kind: "thought", text: ev.text });
      break;
    }
    case "tool_call": {
      const tool: Tool = { id: ev.id, title: ev.title, toolKind: ev.tool_kind, status: ev.status };
      run.tools.push(tool);
      run.segments.push({ kind: "tool", tool });
      run.toolCount = run.tools.length;
      push(run, ms, ev.tool_kind, ev.title, "dim");
      break;
    }
    case "tool_update": {
      // An update that only names files moves nothing here.
      if (!ev.status) break;
      const tool = run.tools.find((t) => t.id === ev.id);
      if (tool) tool.status = ev.status;
      const seg = run.segments.find((s) => s.kind === "tool" && s.tool.id === ev.id);
      if (seg && seg.kind === "tool") seg.tool.status = ev.status;
      break;
    }
    case "plan":
      run.plan = ev.entries;
      push(run, ms, "plan", `${ev.entries.length} entries`, "dim");
      break;
    case "commands":
      push(run, ms, "commands", `${ev.commands.length} slash commands`, "dim");
      break;
    case "usage": {
      const raw = ev.raw as { used?: number; size?: number; cost?: { amount?: number; currency?: string } } | null;
      if (raw && typeof raw.used === "number" && typeof raw.size === "number") {
        run.usage = {
          used: raw.used,
          size: raw.size,
          cost: raw.cost?.amount ?? run.usage?.cost,
          currency: raw.cost?.currency ?? run.usage?.currency,
        };
      }
      break;
    }
    case "permission":
      push(run, ms, "asks", `${ev.tool_kind} · ${ev.title}`, "warn");
      break;
    case "finished":
      run.status = "done";
      run.stopReason = ev.stop_reason;
      run.durationMs = ms;
      push(run, ms, "done", ev.stop_reason, "ok");
      break;
    case "failed":
      run.status = "failed";
      run.error = ev.error;
      run.durationMs = ms;
      push(run, ms, "error", ev.error, "warn");
      break;
  }
}

function fromSummary(s: RunSummary): Run {
  return {
    id: s.id,
    track: s.track,
    session: s.session,
    agent: s.agent,
    prompt: s.prompt,
    status: s.status,
    sessionId: s.session_id ?? undefined,
    cwd: s.cwd,
    startedAt: s.started_at,
    durationMs: s.duration_ms ?? undefined,
    message: s.output,
    plan: s.plan,
    toolCount: s.tool_count,
    stopReason: s.stop_reason ?? undefined,
    error: s.error ?? undefined,
    loaded: false,
    thought: "",
    tools: [],
    transcript: [],
    segments: [],
  };
}

function push(run: Run, ms: number, label: string, text: string, tone: Tone) {
  run.transcript.push({ ms, label, text, tone });
}

/** A quiet log for failures that have a visible fallback. */
function tracing(err: unknown) {
  console.warn(err);
}

/** Short mono label for an agent id. */
export function agentLabel(id: string): string {
  switch (id) {
    case "claude_code":
      return "claude-code";
    case "codex":
      return "codex";
    case "copilot":
      return "copilot";
    case "antigravity":
      return "antigravity";
    default:
      return id;
  }
}

export const store = new Store();

/** Subscribe once; the core emits one `agent` event per agent event. */
export async function connectEvents() {
  await Promise.all([
    listen<Envelope>("agent", (e) => store.apply(e.payload)),
    listen<DownloadProgress>("agent_download", (e) => store.progress(e.payload)),
    listen<Decision>("decision", (e) => store.upsertDecision(e.payload)),
    listen<{ track: string; from: string; to: string; writing: boolean }>("conductor_handoff", (e) => store.takeHandoff(e.payload)),
    listen<WaitingDelivery>("parked", (e) => store.takeParked(e.payload)),
    listen<DesignDelta>("design", (e) => store.takeDesign(e.payload)),
    listen<string>("design_extract", (e) => {
      if (e.payload === store.artifact) void store.loadExtracts(e.payload);
    }),
  ]);
}
