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
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n.svelte";

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

  $effect(() => {
    if (store.info === null) store.loadInfo();
  });
</script>

<aside class="col">
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
      <button class="btn" onclick={() => (store.view = "track")}>{t("settings.close")}</button>
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
      <AgentList />
    {:else if store.settingsSection === "about"}
      <div class="rows mono kv">
        <div class="row"><span class="dim">version</span><span>{store.info?.version ?? "…"}</span></div>
        <div class="row"><span class="dim">db</span><span class="path">{store.info?.db_path ?? "…"}</span></div>
        <div class="row"><span class="dim">adapters</span><span class="path">{store.info?.adapters_dir ?? "…"}</span></div>
        <div class="row"><span class="dim">workspace</span><span class="path">{store.info?.workspace ?? "…"}</span></div>
        <div class="row"><span class="dim">runs</span><span>{store.info?.runs ?? "…"}</span></div>
      </div>
      <p class="note">{t("settings.aboutNote")}</p>
    {/if}
  </div>
</section>

<style>
  .col {
    width: 232px;
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
    gap: 8px;
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
