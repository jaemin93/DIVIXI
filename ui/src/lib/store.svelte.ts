import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { i18n, systemLang, t, type Lang, type LangPref } from "./i18n.svelte";

/** Mirrors `orchestra_core::LaneEvent` — serde tags it with `kind`. */
export type LaneEvent =
  | { kind: "connected"; protocol: string; load_session: boolean }
  | { kind: "started"; session_id: string; cwd: string }
  | { kind: "message"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "tool_call"; id: string; title: string; tool_kind: string; status: string }
  | { kind: "tool_update"; id: string; status: string }
  | { kind: "plan"; entries: string[] }
  | { kind: "usage"; raw: unknown }
  | { kind: "finished"; stop_reason: string }
  | { kind: "failed"; error: string };

export type Envelope = {
  track: string;
  lane: string;
  run: string;
  at_ms: number;
  event: LaneEvent;
};

/** Session options chosen for one role: `option id → value id`, in the agent's own terms. */
export type OptionConfig = Record<string, string>;

/** Mirrors `orchestra_store::TrackInfo`: one conductor, its lanes, one folder. */
export type Track = {
  id: string;
  name: string;
  intent: string;
  cwd: string;
  /** Agent the conductor runs on. */
  agent: string;
  conductor_config: OptionConfig;
  /** Agent lanes run on; empty means the conductor's. */
  worker_agent: string;
  worker_config: OptionConfig;
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
  color: string;
  tags: string[];
}>;

/** A tag the user made: its name and the colour it was dealt. */
export type TagDef = { name: string; color: string };

/** Colours a track can carry in the list; muted enough for both themes. */
export const TRACK_COLORS = ["#7aa2f7", "#73daca", "#9ece6a", "#e0af68", "#ff9e64", "#f7768e", "#bb9af7", "#c0caf5"];

/** The agent a track's lanes run on. */
export function laneAgentOf(track: Track): string {
  return track.worker_agent || track.agent;
}

/** Who a session setting is for. */
export type Role = "conductor" | "worker";

/** Mirrors `orchestra_store::RunSummary`: one run as the timeline sees it. */
export type RunSummary = {
  id: string;
  track: string;
  lane: string;
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
export type StoredEvent = { seq: number; at_ms: number; event: LaneEvent };

/** Mirrors `orchestra_agents::AgentKind` ids. */
export type AgentId = "claude_code" | "codex" | "copilot" | "antigravity";

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
  lane: string;
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

export type View = "track" | "settings" | "lane" | "new-track" | "edit-track";

/** Prefix of conductor prompts Orchestra injects itself (lane reports). Language-neutral. */
export const REPORT_PREFIX = "[lane-report]";

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
export type SettingsSection = "overview" | "appearance" | "chat" | "agents" | "about";

/** Mirrors `workspace::Entry`: one file or folder, path relative to the track folder. */
export type WsEntry = { path: string; name: string; dir: boolean; size: number };

/** Mirrors `workspace::FileContent`. */
export type WsFile = {
  path: string;
  name: string;
  size: number;
  kind: "markdown" | "text" | "image" | "binary" | "large";
  ext: string;
  text: string | null;
  data_url: string | null;
};

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
  /** Second column (tracks and their lanes) shown. Persisted. */
  trackListOpen = $state(true);
  /** Which settings section is open. */
  settingsSection = $state<SettingsSection>("overview");
  /** Lane whose session is open in the main area (view === "lane"). */
  openLane = $state("");
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

  /** The side panel on the track: the working folder as files and changes. Persisted. */
  panelOpen = $state(false);
  panelTab = $state<PanelTab>("files");
  /** Open file tabs, relative paths, in opening order; `activeFile` is the one shown. */
  openFiles = $state<string[]>([]);
  activeFile = $state("");
  /** Show the tree beside an open file, as Kiro does. */
  panelTree = $state(true);
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

  private persistWidth(key: string, value: number) {
    invoke("set_setting", { key, value: String(value) }).catch((err) => {
      this.lastError = String(err);
    });
  }

  setRailWidth(px: number, persist = false) {
    this.railWidth = Math.min(320, Math.max(160, Math.round(px)));
    if (persist) this.persistWidth("rail_width", this.railWidth);
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
  async openFile(path: string) {
    if (!this.openFiles.includes(path)) this.openFiles = [...this.openFiles, path];
    this.activeFile = path;
    this.panelTab = "file";
    this.panelOpen = true;
    await this.loadFile(path);
  }

  closeFile(path: string) {
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
  /** The user's tags, each with a colour; persisted as the `tags` setting. */
  tagPool = $state<TagDef[]>([]);

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
  }
  lastError = $state("");
  restored = $state(false);

  /** Last detection result; null until setup has run once. */
  agents = $state<AgentStatus[] | null>(null);
  /** Agent id lanes are opened on. */
  agent = $state<AgentId>("claude_code");
  detecting = $state(false);
  /** Agent ids with a login or download in flight. */
  working = $state<Record<string, string>>({});
  /** Live download progress per agent id, while a download runs. */
  downloads = $state<Record<string, DownloadProgress>>({});

  get currentTrack(): Track | undefined {
    return this.tracks.find((t) => t.id === this.track);
  }

  /** The current track's runs, oldest first. */
  get trackRuns(): Run[] {
    return this.runs.filter((r) => r.track === this.track);
  }

  get activeRun(): Run | undefined {
    return this.trackRuns.find((r) => r.status === "running" || r.status === "connecting");
  }

  /** The current track's conductor has a turn in flight; the composer waits. */
  get busy(): boolean {
    return this.trackRuns.some((r) => r.lane === "conductor" && (r.status === "running" || r.status === "connecting"));
  }

  /** Any track has a run in flight; the brand mark pulses. */
  get anyLive(): boolean {
    return this.runs.some((r) => r.status === "running" || r.status === "connecting");
  }

  /** Create a track and open it. */
  async createTrack(patch: TrackPatch): Promise<boolean> {
    this.lastError = "";
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
    this.lastError = "";
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
  async deleteTrack(id: string) {
    this.lastError = "";
    try {
      await invoke("delete_track", { id });
    } catch (err) {
      this.lastError = String(err);
      return;
    }
    this.tracks = this.tracks.filter((t) => t.id !== id);
    this.runs = this.runs.filter((r) => r.track !== id);
    if (this.track === id) {
      const next = this.tracks.at(-1);
      if (next) await this.selectTrack(next.id);
      else {
        this.track = "";
        this.view = "new-track";
      }
    }
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

  /** Context accounting to show: the live run's, else the latest run that reported one. */
  get context(): Usage | undefined {
    return this.activeRun?.usage ?? [...this.runs].reverse().find((r) => r.usage)?.usage;
  }

  /** The agent a role runs on in the current track. */
  roleAgent(role: Role): string {
    const track = this.currentTrack;
    if (!track) return this.agent;
    return role === "conductor" ? track.agent : laneAgentOf(track);
  }

  /** A role's chosen options in the current track. */
  roleConfig(role: Role): OptionConfig {
    const track = this.currentTrack;
    if (!track) return {};
    return role === "conductor" ? track.conductor_config : track.worker_config;
  }

  /** Put a role on an agent; for workers, empty means "same as the conductor". Takes effect at the next session. */
  async setRoleAgent(role: Role, agent: string) {
    const track = this.currentTrack;
    if (!track) return;
    if (role === "conductor") {
      this.agent = agent as AgentId;
      await this.updateTrack(track.id, { agent });
    } else {
      await this.updateTrack(track.id, { worker_agent: agent });
    }
  }

  /** Set one of a role's options on the current track; empty clears it. Takes effect at the next session. */
  async setRoleOption(role: Role, id: string, value: string) {
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

  /** Show a lane's session: the conductor's messages and the worker's replies. */
  async openLaneView(name: string) {
    this.openLane = name;
    this.view = "lane";
    // Restored runs only carry their folded text; replay them for the full turn.
    for (const run of this.trackRuns) {
      if (run.lane === name && !run.loaded) await this.hydrate(run);
    }
  }

  /** Open settings on a section. */
  openSettings(section: SettingsSection = "overview") {
    this.settingsSection = section;
    this.view = "settings";
    void this.loadInfo();
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
      const [summaries, tracks, savedTrack, agents, theme, rail, tracklist, chatFont, panelWidth, railWidth, trackListWidth, uiFont, zoom, language, panel, tagsJson] =
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
      ]);
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
      this.agents = agents;
      this.pickDefaultAgent();
      // Reopen on the track that was open, else the newest; none means the
      // first screen is creating one.
      const current = tracks.find((t) => t.id === savedTrack) ?? tracks.at(-1);
      if (current) {
        this.track = current.id;
        if (this.readyAgents.some((a) => a.kind === current.agent)) this.agent = current.agent as AgentId;
        if (this.panelOpen) void this.refreshWorkspace();
      } else {
        this.view = "new-track";
      }
      // Setup comes first when nothing has been detected yet, or when the
      // last detection left nothing to run lanes on.
      if (agents === null || this.readyAgents.length === 0) {
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
    this.lastError = "";
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
    this.lastError = "";
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
   * Send a message to the conductor on the selected agent. The conductor
   * decides whether to answer or to open lanes; lane runs arrive as
   * `lane` events with their own run ids and are added when first seen.
   */
  async send(prompt: string) {
    const text = prompt.trim();
    const track = this.track;
    if (!text || !track || this.busy) return;
    this.lastError = "";
    const agent = this.agent;

    // Show the message the moment Enter is pressed. The run gets its real id
    // when the core answers; until then it carries a pending id, and events
    // that arrive for the real id first are routed to it by `apply`.
    const pending: Run = {
      id: `pending-${Date.now()}`,
      track,
      lane: "conductor",
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
    };
    this.runs.push(pending);

    try {
      const id = await invoke<string>("conductor_prompt", { track, prompt: text, agent, lang: i18n.lang });
      const run = this.runs.find((r) => r === pending || r.id === id);
      if (run) {
        run.id = id;
        run.track = track;
        run.lane = "conductor";
        run.agent = agent;
        run.prompt = text;
      }
      // The conductor now runs on this agent; keep the track's record in step.
      const tr = this.tracks.find((t) => t.id === track);
      if (tr && tr.agent !== agent) tr.agent = agent;
    } catch (err) {
      this.runs = this.runs.filter((r) => r !== pending);
      this.lastError = String(err);
    }
  }

  /** Fold one live lane event into the run it belongs to, creating lane runs on first sight. */
  apply(env: Envelope) {
    let run = this.runs.find((r) => r.id === env.run);
    if (!run && env.lane === "conductor") {
      // The conductor turn we just sent, still waiting for its id.
      run = this.runs.find((r) => r.track === env.track && r.lane === "conductor" && r.id.startsWith("pending-"));
      if (run) run.id = env.run;
    }
    if (!run) {
      // A lane the conductor opened, or a report turn Orchestra injected:
      // the core registered it, we have not. Show it now; the prompt text
      // comes with the summary right after.
      run = {
        id: env.run,
        track: env.track,
        lane: env.lane,
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
    fold(run, env.at_ms, env.event);
    // A finished run may have written files: the open panel catches up.
    if (this.panelOpen && env.track === this.track && (env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshWorkspace();
    }
  }

  /** Fill a lane run's prompt and agent from the store once it exists there. */
  private async refreshRun(id: string) {
    try {
      const summaries = await invoke<RunSummary[]>("list_runs");
      const s = summaries.find((x) => x.id === id);
      const run = this.runs.find((r) => r.id === id);
      if (s && run) {
        run.prompt = s.prompt;
        run.agent = s.agent;
        run.startedAt = s.started_at;
      }
    } catch {
      // Cosmetic; the next restore fills it in.
    }
  }

  /** Lane names seen in a track, in first-seen order, excluding the conductor. */
  laneNamesIn(track: string): string[] {
    const seen: string[] = [];
    for (const r of this.runs) if (r.track === track && r.lane !== "conductor" && !seen.includes(r.lane)) seen.push(r.lane);
    return seen;
  }

  get laneNames(): string[] {
    return this.laneNamesIn(this.track);
  }
}

/** Pure state transition: the same fold serves live events and replay. */
function fold(run: Run, ms: number, ev: LaneEvent) {
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
    lane: s.lane,
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

/** Subscribe once; the core emits one `lane` event per lane event. */
export async function connectEvents() {
  await Promise.all([
    listen<Envelope>("lane", (e) => store.apply(e.payload)),
    listen<DownloadProgress>("agent_download", (e) => store.progress(e.payload)),
  ]);
}
