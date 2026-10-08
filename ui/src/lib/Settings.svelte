<script lang="ts">
  import {
    store,
    agentLabel,
    UPDATE_WRONG,
    UPDATE_DOWNLOAD_WRONG,
    updateMb as mb,
    updatePercent as percent,
    updateProgressText as progressText,
    updateWrongText,
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
  import PhoneAccess from "./PhoneAccess.svelte";
  import { onMount } from "svelte";
  import { customFilter, type LogLevel } from "./logLevel";

  /** Minutes before an unused conductor or worker session is closed (0: never). */
  let idleMinutes = $state(30);

  let level = $state<LogLevel | null>(null);
  /** Spelled out only when no preset button is lit for it. */
  const customLevel = $derived(customFilter(level));

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
    if (local) void store.loadRelease();
    // And whether the check at startup is on, for the switch on that card.
    // `checkAtStartup` reads it too; this keeps the card right after the
    // switch has been changed elsewhere.
    if (local) void store.loadUpdateAuto();
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
  //
  // The state lives in the store, because the banner outside every view shows
  // the same thing (UpdateBanner.svelte). What is here is how this card draws
  // it. Mirrors src-tauri/src/update.rs.

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
        <button class="entry" class:on={store.settingsSection === e.id} title={e.label} aria-label={e.label} onclick={() => (store.settingsSection = e.id)}>
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
          <div class="sub">DIVIXI</div>
        </div>
      </div>

      {#if local}<PhoneAccess />{/if}

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

      <!-- This PC's own app is the one an update would replace, so the card is
           local-only like the log folder and the diagnostics above it. A
           remote instance is updated where it runs. -->
      {#if local}
        <div class="group">
          <div class="gtitle">{t("settings.updates")}</div>
          <div class="card pad">
            <div class="ftitle">{t("settings.updateCheck")}</div>
            <p class="fnote now mono">
              {store.release === null ? t("settings.updateDevBuild") : t("settings.updateRelease", { tag: store.release })}
            </p>
            <div class="actions">
              <button class="btn" disabled={store.updateChecking} onclick={() => store.checkUpdate()}>
                {store.updateChecking ? t("settings.updateChecking") : t("settings.updateCheck")}
              </button>
            </div>

            <!-- The check at startup, which is the only one nobody pressed.
                 Off here means the app asks GitHub nothing on its own; the
                 button above keeps working either way. -->
            <div class="swrow">
              <div class="ftitle grow">{t("settings.updateAuto")}</div>
              <button
                class="toggle"
                class:on={store.updateAuto}
                role="switch"
                aria-checked={store.updateAuto}
                aria-label={t("settings.updateAuto")}
                onclick={() => store.setUpdateAuto(!store.updateAuto)}><span></span></button
              >
            </div>

            {#if store.updateCheck}
              {@const c = store.updateCheck}
              {@const wrong = UPDATE_WRONG.has(c.kind)}
              <!-- A failure interrupts (role=alert); an answer is announced
                   when the reader gets to it (role=status). The pair is kept
                   in step, as in CrashBanner and ErrorToasts. -->
              <div class="ures" role={wrong ? "alert" : "status"} aria-live={wrong ? "assertive" : "polite"}>
                {#if c.kind === "dev_build"}
                  <p class="fnote">{t("settings.updateDevStatus")}</p>
                {:else if c.kind === "up_to_date"}
                  <p class="fnote ok">{t("settings.updateLatest", { tag: c.current })}</p>
                {:else if c.kind === "ahead"}
                  <p class="fnote">{t("settings.updateAheadStatus", { latest: c.latest })}</p>
                {:else if c.kind === "update"}
                  <p class="fnote found">{t("settings.updateFound", { tag: c.latest, current: c.current })}</p>
                  <div class="actions">
                    <button class="btn" onclick={() => openRelease(c.url)}>{t("settings.updateOpen")}</button>
                    <!-- Windows only today. On macOS and Linux the release
                         page above stays the whole of it: the app says which
                         platforms it fetches for in one function
                         (`installer` in src-tauri/src/update.rs) and this
                         button follows that answer. -->
                    {#if c.can_download && !store.updateReady}
                      <button class="btn" disabled={store.updateProgress !== null} onclick={() => store.downloadUpdate()}>
                        {store.updateGot && UPDATE_DOWNLOAD_WRONG.has(store.updateGot.kind) ? t("settings.updateRetry") : t("settings.updateDownload")}
                      </button>
                    {/if}
                    {#if store.updateProgress !== null}
                      <button class="btn" onclick={() => store.cancelUpdate()}>{t("settings.updateCancel")}</button>
                    {/if}
                  </div>
                  {#if c.can_download}
                    <p class="fnote">{t("settings.updateDownloadNote")}</p>
                  {:else}
                    <p class="fnote">{t("settings.updateManualOnly")}</p>
                  {/if}
                  <p class="fnote warn">{t("settings.updateUnsigned")}</p>

                  {#if store.updateProgress}
                    {@const p = store.updateProgress}
                    {@const pct = percent(p)}
                    <div class="dl">
                      <div class="bar" class:indeterminate={pct === null}>
                        <div class="fill" style="width: {pct ?? 30}%"></div>
                      </div>
                      <div class="mono dltext" role="status" aria-live="polite">{progressText(p)}</div>
                    </div>
                  {/if}

                  {#if store.updateGot}
                    {@const g = store.updateGot}
                    {@const broke = UPDATE_DOWNLOAD_WRONG.has(g.kind)}
                    <div class="ures" role={broke ? "alert" : "status"} aria-live={broke ? "assertive" : "polite"}>
                      {#if g.kind === "ready"}
                        <p class="fnote ok">{t("settings.updateVerified", { name: g.name, size: mb(g.size) })}</p>
                        <p class="fnote why mono">{t("settings.updateAt", { path: g.path })}</p>
                        <div class="actions">
                          <button class="btn btn-acc" onclick={() => store.openInstaller()}>{t("settings.updateOpenInstaller")}</button>
                          <button class="btn" onclick={() => store.revealUpdate()}>{t("settings.updateShowFolder")}</button>
                        </div>
                        <p class="fnote">{t("settings.updateBeforeInstall")}</p>
                        <p class="fnote warn">{t("settings.updateWarnUnsigned")}</p>
                      {:else if g.kind === "cancelled"}
                        <p class="fnote">{t("settings.updateCancelled")}</p>
                      {:else if g.kind === "busy"}
                        <p class="fnote">{t("settings.updateBusy")}</p>
                      {:else if g.kind === "nothing"}
                        <p class="fnote">{t("settings.updateNothing")}</p>
                      {:else if g.kind === "unsupported"}
                        <p class="fnote">{t("settings.updateManualOnly")}</p>
                      {:else}
                        <!-- Everything left is one of `UPDATE_DOWNLOAD_WRONG`,
                             and the banner has to say the same thing, so the
                             wording lives once in store.svelte.ts. -->
                        {@const w = updateWrongText(g)}
                        <p class="fnote bad">{w.say}</p>
                        {#if w.why}<p class="fnote why mono">{w.why}</p>{/if}
                      {/if}
                    </div>
                  {/if}
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
            <div class="actions">
              <button class="btn" onclick={openLogs}>{t("settings.openLogs")}</button>
            </div>

            <div class="ftitle top">{t("settings.level")}</div>
            <div class="seg" role="radiogroup" aria-label={t("settings.level")}>
              {#each levels as l (l.id)}
                <button class="segopt" class:on={level?.preset === l.id} role="radio" aria-checked={level?.preset === l.id} onclick={() => setLevel(l.id)}>
                  {l.label}
                </button>
              {/each}
            </div>
            {#if customLevel !== null}
              <p class="fnote now mono">{t("settings.levelCustom", { filter: customLevel })}</p>
            {/if}

            <div class="ftitle top">{t("settings.report")}</div>
            <p class="fnote">{t("settings.reportPaths")}</p>
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

  /* Which build this is, or which filter no preset stands for. */
  .fnote.now {
    margin: 10px 0 0;
    font-size: 11px;
    color: var(--lab);
  }

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

  /* A setting with a switch beside it: the words take the width, the switch
     keeps its size. Same switch as the knowledge settings use. */
  .swrow {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 18px;
    padding-top: 16px;
    border-top: 1px solid var(--line);
  }

  .toggle {
    width: 40px;
    height: 22px;
    border-radius: 11px;
    border: 1px solid var(--lines);
    background: var(--inp);
    position: relative;
    padding: 0;
    flex-shrink: 0;
  }

  .toggle span {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--dim);
    transition: left 0.15s;
  }

  .toggle.on {
    background: var(--acc);
    border-color: var(--acc);
  }

  .toggle.on span {
    left: 20px;
    background: var(--accon);
  }

  /* The installer download's bar. Same shape as the agent download's
     (AgentList.svelte): hairline track, accent fill, no radius. */
  .dl {
    margin: 12px 0 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .bar {
    height: 3px;
    background: var(--line);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--acc);
    transition: width 120ms linear;
  }

  /* No total to draw a fraction of: a sliver that moves, and the amount in
     words beside it. */
  .indeterminate .fill {
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(340%);
    }
  }

  .dltext {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--lab);
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
    margin-bottom: 26px;
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

  .ftitle + .actions,
  .ftitle + .seg {
    margin-top: 12px;
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

  /* A phone's width (Track's breakpoint): the sections fold to their icons,
     as the rail does, so the page beside them has the screen. */
  @media (max-width: 640px) {
    .col {
      width: 48px !important;
    }

    .col > :global([role="separator"]),
    .head,
    .glabel,
    .entry .label {
      display: none;
    }

    .entry {
      height: 44px;
      justify-content: center;
      padding: 0;
    }

    .inner {
      padding: 20px 16px 40px;
    }
  }
</style>
