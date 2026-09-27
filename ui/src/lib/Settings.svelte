<script lang="ts">
  import {
    store,
    agentLabel,
    ZOOM_MIN,
    ZOOM_MAX,
    ZOOM_STEP,
    type SettingsSection,
    type ChatFont,
    type UiFont,
  } from "./store.svelte";

  const uiFonts: { id: UiFont; label: string; family: string }[] = [
    { id: "sans", label: "Sans", family: '"IBM Plex Sans", system-ui, sans-serif' },
    { id: "mono", label: "Mono", family: '"IBM Plex Mono", ui-monospace, monospace' },
    { id: "system", label: "System", family: 'system-ui, "Segoe UI", "Malgun Gothic", sans-serif' },
    { id: "serif", label: "Serif", family: '"Newsreader", Georgia, serif' },
  ];
  import AgentList from "./AgentList.svelte";
  import ThemePicker from "./ThemePicker.svelte";
  import LangPicker from "./LangPicker.svelte";
  import KnowledgeSettings from "./KnowledgeSettings.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";
  import { invoke, inTauri, local } from "./ipc.svelte";
  import RemoteHosts from "./RemoteHosts.svelte";
  import RemoteServer from "./RemoteServer.svelte";
  import { onMount } from "svelte";

  /** Minutes before an unused conductor or worker session is closed (0: never). */
  let idleMinutes = $state(30);

  /** Mirrors `logging::Level`: how much is being logged, and what set it. */
  type LogLevel = { filter: string; preset: string; source: string };
  let level = $state<LogLevel | null>(null);

  onMount(() => {
    invoke<string | null>("get_setting", { key: "sessions.idle_minutes" })
      .then((v) => {
        const n = Number(v);
        if (v !== null && Number.isFinite(n)) idleMinutes = n;
      })
      .catch(() => {});
    // The log is this PC's own, as is the folder it lives in.
    if (local) invoke<LogLevel>("log_level").then((l) => (level = l)).catch(() => {});
    // Which release this app is, for the update card. Reads a string compiled
    // into the binary; asks GitHub nothing (src-tauri/src/update.rs).
    if (local) invoke<string | null>("update_release").then((r) => (release = r)).catch(() => {});
  });
  function saveIdle() {
    const n = Math.max(0, Math.min(1440, Math.round(Number(idleMinutes) || 0)));
    idleMinutes = n;
    invoke("set_setting", { key: "sessions.idle_minutes", value: String(n) }).catch((err) => (store.lastError = String(err)));
  }

  /** The settings column. A group is a label; an entry opens a pane. */
  type Entry = { id: SettingsSection; icon: IconName; label: string; blurb: string };
  const groups = $derived<{ label: string; entries: Entry[] }[]>([
    {
      label: "",
      entries: [{ id: "overview", icon: "overview", label: t("settings.overview"), blurb: t("settings.overviewBlurb") }],
    },
    {
      label: t("settings.group.prefs"),
      entries: [
        { id: "appearance", icon: "look", label: t("settings.appearance"), blurb: t("settings.appearanceBlurb") },
        { id: "chat", icon: "chat", label: t("settings.chat"), blurb: t("settings.chatBlurb") },
        { id: "agents", icon: "agents", label: t("settings.agents"), blurb: t("settings.agentsBlurb") },
        { id: "knowledge", icon: "book", label: t("settings.knowledge"), blurb: t("settings.knowledgeBlurb") },
        // Remote instances are opened from the desktop app.
        ...(inTauri ? [{ id: "remote" as SettingsSection, icon: "remote" as IconName, label: t("settings.remote"), blurb: t("settings.remoteBlurb") }] : []),
      ],
    },
    {
      label: t("settings.group.system"),
      entries: [{ id: "about", icon: "info", label: t("settings.about"), blurb: t("settings.aboutBlurb") }],
    },
  ]);

  const current = $derived(groups.flatMap((g) => g.entries).find((e) => e.id === store.settingsSection)!);

  /** The status dot beside each agent: ready, wants attention, broken, or idle. */
  const READINESS_COLORS: Record<string, string> = {
    ready: "var(--ok)",
    needs_login: "var(--warn)",
    needs_download: "var(--warn)",
    error: "var(--acct)",
  };
  function readinessColor(readiness: string): string {
    return READINESS_COLORS[readiness] ?? "var(--idle)";
  }

  const fonts = $derived<{ id: ChatFont; label: string; px: string }[]>([
    { id: "s", label: t("settings.size.s"), px: "11px" },
    { id: "m", label: t("settings.size.m"), px: "13px" },
    { id: "l", label: t("settings.size.l"), px: "15px" },
  ]);

  /** The diagnostics report, kept on screen so it is read before it is pasted. */
  let report = $state("");
  let collecting = $state(false);
  let copied = $state(false);

  /** The levels the backend offers, in the order they read in. */
  const levels = $derived([
    { id: "quiet", label: t("settings.level.quiet") },
    { id: "info", label: t("settings.level.info") },
    { id: "debug", label: t("settings.level.debug") },
    { id: "trace", label: t("settings.level.trace") },
  ]);

  /** Takes effect at once: no restart, and the run being debugged is kept. */
  async function setLevel(id: string) {
    try {
      level = await invoke<LogLevel>("log_level_set", { level: id });
    } catch (err) {
      store.lastError = t("settings.levelFailed", { why: String(err) });
    }
  }

  /** What put the level in force, in this language. */
  function levelFrom(source: string): string {
    if (source === "env") return t("settings.levelFrom.env");
    if (source === "setting") return t("settings.levelFrom.setting");
    return t("settings.levelFrom.default");
  }

  async function openLogs() {
    try {
      await invoke("logs_open");
    } catch (err) {
      store.lastError = t("settings.openLogsFailed", { why: String(err) });
    }
  }

  /** Collect the report, show it, and put it on the clipboard. */
  async function copyReport() {
    collecting = true;
    copied = false;
    try {
      report = await invoke<string>("diagnostics_report");
    } catch (err) {
      store.lastError = t("settings.reportFailed", { why: String(err) });
      collecting = false;
      return;
    }
    collecting = false;
    try {
      await navigator.clipboard.writeText(report);
      copied = true;
    } catch {
      // The report is on screen either way, so it can be copied by hand.
      store.lastError = t("settings.copyFailed");
    }
  }

  // ----- updates -----

  /**
   * What `update_check` answers. Every outcome is a named case, failures
   * included, so there is a line to show for each of them and none of them
   * can go by unsaid. Mirrors `Check` in src-tauri/src/update.rs.
   */
  type Check =
    | { kind: "dev_build" }
    | { kind: "up_to_date"; current: string }
    | { kind: "ahead"; current: string; latest: string }
    | { kind: "update"; current: string; latest: string; url: string }
    | { kind: "offline"; detail: string }
    | { kind: "rate_limited"; detail: string }
    | { kind: "no_release" }
    | { kind: "failed"; detail: string }
    | { kind: "bad_build"; current: string };

  /** The release this build came from, or null for a development build. */
  let release = $state<string | null>(null);
  /** The last check's answer, or null before the button has been pressed. */
  let check = $state<Check | null>(null);
  let checking = $state(false);

  /** The cases that are something wrong rather than an answer. */
  const WRONG = new Set(["offline", "rate_limited", "no_release", "failed", "bad_build"]);

  /**
   * Ask GitHub, on this press and on no other occasion.
   *
   * There is deliberately no check at startup and none on a timer: the README
   * promises divixi sends nothing anywhere on its own, and a request made
   * without being asked for would be the one exception. See the module header
   * in src-tauri/src/update.rs.
   */
  async function checkUpdate() {
    checking = true;
    check = null;
    try {
      check = await invoke<Check>("update_check");
    } catch (err) {
      // The command names a case for every failure it knows, so getting here
      // means the call itself never landed. Still shown, not swallowed.
      check = { kind: "failed", detail: String(err) };
    }
    checking = false;
  }

  /** The release's page in this PC's browser. Nothing is downloaded here. */
  async function openRelease(url: string) {
    try {
      await invoke("open_url", { url });
    } catch (err) {
      store.lastError = t("settings.updateOpenFailed", { why: String(err) });
    }
  }

  $effect(() => {
    if (store.info === null) store.loadInfo();
  });
</script>

<aside class="col" style="width: {store.settingsNavWidth}px">
  <SplitHandle edge="right" width={store.settingsNavWidth} min={200} max={420} reset={232} label={t("settings.navWidth")} onchange={(px, persist) => store.setSettingsNavWidth(px, persist)} />
  <div class="head"><span class="title serif">{t("settings.title")}</span></div>
  <div class="nav">
    {#each groups as g (g.label)}
      {#if g.label}<div class="mlab glabel">{g.label}</div>{/if}
      {#each g.entries as e (e.id)}
        <button class="entry" class:on={store.settingsSection === e.id} onclick={() => (store.settingsSection = e.id)}>
          <Icon name={e.icon} />
          <span class="label">{e.label}</span>
        </button>
      {/each}
    {/each}
  </div>
</aside>

<section class="pane">
  <div class="inner">
    <div class="top">
      <div>
        <div class="mlab">{t("settings.crumb", { section: current.label })}</div>
        <h1 class="serif">{current.label}</h1>
        <p class="blurb">{current.blurb}</p>
      </div>
      <span class="grow"></span>
      <button class="btn" onclick={() => store.closeSettings()}>{t("settings.close")}</button>
    </div>

    {#if store.settingsSection === "overview"}
      <div class="tiles">
        <div class="tile">
          <div class="mlab-sm">{t("settings.tile.agents")}</div>
          <div class="big mono"><span class:ok={store.readyAgents.length > 0}>{store.readyAgents.length}</span> / {store.agents?.length ?? 0}</div>
          <div class="sub">{t("settings.tile.ready")}</div>
        </div>
        <div class="tile">
          <div class="mlab-sm">{t("settings.tile.runs")}</div>
          <div class="big mono">{store.info?.runs ?? store.runs.length}</div>
          <div class="sub">{t("settings.tile.runsNote")}</div>
        </div>
        <div class="tile">
          <div class="mlab-sm">{t("settings.tile.default")}</div>
          <div class="big mono">{agentLabel(store.agent)}</div>
          <div class="sub">{t("settings.tile.defaultNote")}</div>
        </div>
        <div class="tile">
          <div class="mlab-sm">{t("settings.tile.version")}</div>
          <div class="big mono">{store.info?.version ?? "…"}</div>
          <div class="sub">divixi</div>
        </div>
      </div>

      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("settings.card.agents")}</span>
          <span class="grow"></span>
          <button class="btn" onclick={() => (store.settingsSection = "agents")}>{t("settings.card.configure")}</button>
        </div>
        <div class="rows mono">
          {#each store.agents ?? [] as a (a.kind)}
            <div class="row">
              <span class="dot" style="background: {readinessColor(a.readiness)}"></span>
              <span>{agentLabel(a.kind)}</span>
              <span class="grow"></span>
              <span class="dim">{a.readiness.replace("_", " ")}</span>
            </div>
          {/each}
          {#if !store.agents}<div class="row dim">{t("agents.notYet")}</div>{/if}
        </div>
      </div>

      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("settings.card.store")}</span>
          <span class="grow"></span>
          <button class="btn" onclick={() => (store.settingsSection = "about")}>{t("settings.card.details")}</button>
        </div>
        <div class="rows mono">
          <div class="row"><span class="dim">db</span><span class="path">{store.info?.db_path ?? "…"}</span></div>
          <div class="row"><span class="dim">workspace</span><span class="path">{store.info?.workspace ?? "…"}</span></div>
        </div>
      </div>
    {:else if store.settingsSection === "appearance"}
      <div class="group">
        <div class="gtitle">{t("settings.view")}</div>
        <div class="card pad">
          <div class="ftitle">{t("settings.theme")}</div>
          <p class="fnote">{t("settings.themeNote")}</p>
          <ThemePicker />
          <div class="ftitle top">{t("lang.label")}</div>
          <p class="fnote">{t("lang.blurb")}</p>
          <LangPicker />
        </div>
      </div>

      <div class="group">
        <div class="gtitle">{t("settings.zoomAndFont")}</div>
        <div class="card pad">
          <div class="ftitle">{t("settings.zoom")}</div>
          <p class="fnote">{t("settings.zoomNote", { min: ZOOM_MIN, max: ZOOM_MAX })}</p>
          <div class="zoom">
            <button class="btn sq" onclick={() => store.setZoom(store.zoom - ZOOM_STEP)} disabled={store.zoom <= ZOOM_MIN} aria-label={t("settings.zoomOut")}>−</button>
            <span class="mono zval" aria-live="polite">{store.zoom}%</span>
            <button class="btn sq" onclick={() => store.setZoom(store.zoom + ZOOM_STEP)} disabled={store.zoom >= ZOOM_MAX} aria-label={t("settings.zoomIn")}>+</button>
            {#if store.zoom !== 100}
              <button class="btn" onclick={() => store.setZoom(100)}>{t("settings.zoomReset")}</button>
            {/if}
          </div>

          <div class="ftitle top">{t("settings.font")}</div>
          <p class="fnote">{t("settings.fontNote")}</p>
          <div class="seg" role="radiogroup" aria-label={t("settings.fontLabel")}>
            {#each uiFonts as f (f.id)}
              <button
                class="segopt"
                class:on={store.uiFont === f.id}
                role="radio"
                aria-checked={store.uiFont === f.id}
                style="font-family: {f.family}"
                onclick={() => store.setUiFont(f.id)}
              >
                {f.label}
              </button>
            {/each}
          </div>
        </div>
      </div>
    {:else if store.settingsSection === "chat"}
      <div class="picker" role="radiogroup" aria-label={t("settings.chatSize")}>
        {#each fonts as f (f.id)}
          <button class="opt" class:on={store.chatFont === f.id} role="radio" aria-checked={store.chatFont === f.id} onclick={() => store.setChatFont(f.id)}>
            <span class="sample serif" style="font-size: {f.px}">{t("settings.chatSample")}</span>
            <span class="mono label">{f.label} · {f.px}</span>
          </button>
        {/each}
      </div>
      <p class="note">{t("settings.chatNote")}</p>
    {:else if store.settingsSection === "agents"}
      <div class="actions">
        <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>
          {store.detecting ? t("settings.detecting") : t("settings.redetect")}
        </button>
        <button class="btn" onclick={() => { store.setupStep = 1; store.setupOpen = true; }}>{t("settings.openSetup")}</button>
      </div>
      <p class="note">
        {t("settings.agentsNote")}
      </p>
      <label class="idle">
        <span class="mlab-sm">{t("settings.idle")}</span>
        <input class="mono" type="number" min="0" max="1440" step="5" bind:value={idleMinutes} onchange={saveIdle} />
        <span class="dim">{t("settings.idleUnit")}</span>
      </label>
      <p class="note">{t("settings.idleNote")}</p>
      <AgentList />
    {:else if store.settingsSection === "remote"}
      <RemoteServer />
      <RemoteHosts />
    {:else if store.settingsSection === "knowledge"}
      <KnowledgeSettings />
    {:else if store.settingsSection === "about"}
      <div class="rows mono kv">
        <div class="row"><span class="dim">version</span><span>{store.info?.version ?? "…"}</span></div>
        <div class="row"><span class="dim">db</span><span class="path">{store.info?.db_path ?? "…"}</span></div>
        <div class="row"><span class="dim">logs</span><span class="path">{store.info?.logs_dir ?? "…"}</span></div>
        <div class="row"><span class="dim">adapters</span><span class="path">{store.info?.adapters_dir ?? "…"}</span></div>
        <div class="row"><span class="dim">workspace</span><span class="path">{store.info?.workspace ?? "…"}</span></div>
        <div class="row"><span class="dim">runs</span><span>{store.info?.runs ?? "…"}</span></div>
      </div>
      <p class="note">{t("settings.aboutNote")}</p>

      <!-- This PC's own app is the one an update would replace, so the card is
           local-only like the log folder and the diagnostics above it. A
           remote instance is updated where it runs. -->
      {#if local}
        <div class="group">
          <div class="gtitle">{t("settings.updates")}</div>
          <div class="card pad">
            <div class="ftitle">{t("settings.updateCheck")}</div>
            <p class="fnote">{t("settings.updateNote")}</p>
            <p class="fnote now mono">
              {release === null ? t("settings.updateDevBuild") : t("settings.updateRelease", { tag: release })}
            </p>
            <div class="actions">
              <button class="btn" disabled={checking} onclick={checkUpdate}>
                {checking ? t("settings.updateChecking") : t("settings.updateCheck")}
              </button>
            </div>
            {#if check}
              {@const c = check}
              {@const wrong = WRONG.has(c.kind)}
              <!-- A failure interrupts (role=alert); an answer is announced
                   when the reader gets to it (role=status). The pair is kept
                   in step, as in CrashBanner and ErrorToasts. -->
              <div class="ures" role={wrong ? "alert" : "status"} aria-live={wrong ? "assertive" : "polite"}>
                {#if c.kind === "dev_build"}
                  <p class="fnote warn">{t("settings.updateDevNote")}</p>
                {:else if c.kind === "up_to_date"}
                  <p class="fnote ok">{t("settings.updateLatest", { tag: c.current })}</p>
                {:else if c.kind === "ahead"}
                  <p class="fnote">{t("settings.updateAhead", { tag: c.current, latest: c.latest })}</p>
                {:else if c.kind === "update"}
                  <p class="fnote found">{t("settings.updateFound", { tag: c.latest, current: c.current })}</p>
                  <div class="actions">
                    <button class="btn" onclick={() => openRelease(c.url)}>{t("settings.updateOpen")}</button>
                  </div>
                  <p class="fnote warn">{t("settings.updateUnsigned")}</p>
                {:else if c.kind === "offline"}
                  <p class="fnote bad">{t("settings.updateOffline")}</p>
                  <p class="fnote why mono">{c.detail}</p>
                {:else if c.kind === "rate_limited"}
                  <p class="fnote bad">{t("settings.updateRateLimited")}</p>
                  <p class="fnote why mono">{c.detail}</p>
                {:else if c.kind === "no_release"}
                  <p class="fnote bad">{t("settings.updateNoRelease")}</p>
                {:else if c.kind === "bad_build"}
                  <p class="fnote bad">{t("settings.updateBadBuild", { tag: c.current })}</p>
                {:else}
                  <p class="fnote bad">{t("settings.updateFailed")}</p>
                  <p class="fnote why mono">{c.detail}</p>
                {/if}
              </div>
            {/if}
          </div>
        </div>

        <div class="group">
          <div class="gtitle">{t("settings.diag")}</div>
          <div class="card pad">
            <div class="ftitle">{t("settings.logs")}</div>
            <p class="fnote">{t("settings.logsNote")}</p>
            <div class="actions">
              <button class="btn" onclick={openLogs}>{t("settings.openLogs")}</button>
            </div>

            <div class="ftitle top">{t("settings.level")}</div>
            <p class="fnote">{t("settings.levelNote")}</p>
            <div class="seg" role="radiogroup" aria-label={t("settings.level")}>
              {#each levels as l (l.id)}
                <button class="segopt" class:on={level?.preset === l.id} role="radio" aria-checked={level?.preset === l.id} onclick={() => setLevel(l.id)}>
                  {l.label}
                </button>
              {/each}
            </div>
            {#if level}
              <p class="fnote now mono">{t("settings.levelNow", { filter: level.filter, from: levelFrom(level.source) })}</p>
            {/if}
            {#if level && (level.preset === "debug" || level.preset === "trace")}
              <p class="fnote warn">{t("settings.levelDebugNote")}</p>
            {/if}

            <div class="ftitle top">{t("settings.report")}</div>
            <p class="fnote">{t("settings.reportNote")}</p>
            <div class="actions">
              <button class="btn" disabled={collecting} onclick={copyReport}>
                {collecting ? t("settings.reportMaking") : t("settings.copyReport")}
              </button>
              {#if copied}<span class="copied" aria-live="polite">{t("settings.reportCopied")}</span>{/if}
            </div>
            {#if report}<pre class="report mono">{report}</pre>{/if}
          </div>
        </div>
      {/if}
    {/if}
  </div>
</section>

<style>
  .idle {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
  }

  .idle input {
    width: 80px;
    height: 30px;
    padding: 0 8px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
  }

  .idle .dim {
    font-size: 12px;
    color: var(--dim);
  }

  .col {
    position: relative;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
  }

  .title {
    font-size: 18px;
    color: var(--hi);
  }

  .nav {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 0 12px;
  }

  .glabel {
    padding: 18px 16px 8px;
  }

  .entry {
    width: 100%;
    height: 36px;
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 0 16px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 13px;
    color: var(--dim);
  }

  .entry:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .entry.on {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg);
  }

  .inner {
    max-width: 780px;
    padding: 34px 40px 56px;
  }

  .top {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding-bottom: 22px;
  }

  h1 {
    margin: 8px 0 0;
    font-weight: 400;
    font-size: 30px;
    line-height: 1.1;
    color: var(--hi);
  }

  .blurb {
    margin: 8px 0 0;
    font-size: 13px;
    color: var(--dim);
  }

  .grow {
    flex: 1;
  }

  .note {
    margin: 14px 0 22px;
    font-size: 13px;
    line-height: 1.65;
    color: var(--dim);
    max-width: 560px;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .copied {
    font-size: 12px;
    color: var(--ok);
  }

  /* The filter in force, under its buttons. */
  .fnote.now {
    margin: 10px 0 0;
    font-size: 11px;
    color: var(--lab);
  }

  /* Turning the level up records more than the app's own lines. */
  .fnote.warn {
    margin: 8px 0 0;
    color: var(--warn);
  }

  /* The update check's answer, under the button that asked for it. */
  .ures {
    margin-top: 14px;
  }

  .ures .fnote {
    margin: 0 0 8px;
  }

  .ures .fnote:last-child {
    margin-bottom: 0;
  }

  .fnote.ok {
    color: var(--ok);
  }

  /* A release is out: the one line in the card worth reading first. */
  .fnote.found {
    color: var(--hi);
  }

  .fnote.bad {
    color: var(--acct);
  }

  /* What GitHub or the network actually said. English, like the diagnostics
     report: it is quoted into an issue, not read for comfort. */
  .fnote.why {
    font-size: 11px;
    color: var(--lab);
    word-break: break-word;
  }

  /* The report as it will be pasted: scrolls, wraps nothing, selectable. */
  .report {
    margin: 12px 0 0;
    max-height: 320px;
    overflow: auto;
    padding: 12px 14px;
    border: 1px solid var(--line);
    background: var(--inp);
    font-size: 11px;
    line-height: 1.6;
    color: var(--txt);
    user-select: text;
  }

  /* Overview tiles: hairline boxes, one number each. */
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    margin-bottom: 18px;
  }

  .tile {
    border: 1px solid var(--line);
    background: var(--card);
    padding: 14px 16px 12px;
  }

  .big {
    margin-top: 8px;
    font-size: 22px;
    color: var(--hi);
  }

  .big .ok {
    color: var(--ok);
  }

  .sub {
    margin-top: 4px;
    font-size: 11px;
    color: var(--lab);
  }

  .card {
    border: 1px solid var(--line);
    background: var(--card);
    margin-bottom: 12px;
  }

  .cardhead {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }

  .ctitle {
    font-size: 14px;
    color: var(--hi);
  }

  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 16px;
    font-size: 11px;
    color: var(--txt);
    border-top: 1px solid var(--lineq);
  }

  .card .row:first-child {
    border-top: 0;
  }

  .kv {
    border: 1px solid var(--line);
    background: var(--card);
  }

  .kv .row:first-child {
    border-top: 0;
  }

  .kv .dim {
    width: 90px;
    flex-shrink: 0;
  }

  .dim {
    color: var(--lab);
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .group {
    margin-bottom: 26px;
  }

  .gtitle {
    font-size: 15px;
    color: var(--hi);
    margin-bottom: 10px;
  }

  .card.pad {
    padding: 18px 20px 20px;
  }

  .ftitle {
    font-size: 14px;
    color: var(--hi);
  }

  .ftitle.top {
    margin-top: 24px;
  }

  .fnote {
    margin: 6px 0 12px;
    font-size: 12px;
    line-height: 1.6;
    color: var(--dim);
    max-width: 600px;
  }

  .zoom {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sq {
    width: 36px;
    height: 36px;
    padding: 0;
    font-size: 14px;
    letter-spacing: 0;
  }

  .zval {
    min-width: 64px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    font-size: 13px;
    color: var(--hi);
  }

  /* Segmented control: hairline box, accent underline on the chosen one. */
  .seg {
    display: inline-flex;
    border: 1px solid var(--line);
  }

  .segopt {
    height: 40px;
    padding: 0 18px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    border-bottom: 2px solid transparent;
    color: var(--dim);
    font-size: 14px;
  }

  .segopt:last-child {
    border-right: 0;
  }

  .segopt:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .segopt.on {
    color: var(--hi);
    background: var(--sel);
    border-bottom-color: var(--acc);
  }

  /* Font-size picker: same shape as the theme picker. */
  .picker {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    border: 1px solid var(--line);
  }

  .opt {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: 18px 16px 14px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    border-bottom: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .opt:last-child {
    border-right: 0;
  }

  .opt:hover {
    background: var(--sel);
  }

  .opt.on {
    border-bottom-color: var(--acc);
    color: var(--hi);
    background: var(--sel);
  }

  .sample {
    color: var(--txt);
    line-height: 1.3;
  }

  .label {
    font-size: 10px;
    letter-spacing: 0.18em;
  }
</style>
