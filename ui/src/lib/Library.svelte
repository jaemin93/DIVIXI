<script lang="ts">
  import { store, agentLabel, type LibrarySection, type McpServerDef } from "./store.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The library: what every agent shares. MCP servers go to sessions over
   * ACP; skills are copied into the agents' own skill folders. Two columns,
   * like settings: a section list, then the section.
   */
  type Entry = { id: LibrarySection; icon: IconName; label: string; blurb: string };
  const entries = $derived<Entry[]>([
    { id: "mcp", icon: "bot", label: t("library.mcp"), blurb: t("library.mcpBlurb") },
    { id: "skills", icon: "library", label: t("library.skills"), blurb: t("library.skillsBlurb") },
  ]);
  const current = $derived(entries.find((e) => e.id === store.librarySection) ?? entries[0]);

  const agentIds = $derived((store.agents ?? []).map((a) => a.kind));

  // ----- MCP form -----
  let editing = $state<McpServerDef | null>(null);
  let isNew = $state(false);
  let argsText = $state("");
  let envText = $state("");
  let headersText = $state("");
  let confirmMcp = $state("");

  function blank(): McpServerDef {
    return { name: "", transport: "stdio", command: "", args: [], env: [], url: "", headers: [], enabled: true, agents: [] };
  }

  function startEdit(def: McpServerDef | null) {
    isNew = def === null;
    editing = def ? structuredClone($state.snapshot(def)) : blank();
    argsText = editing.args.join("\n");
    envText = editing.env.map(([k, v]) => `${k}=${v}`).join("\n");
    headersText = editing.headers.map(([k, v]) => `${k}: ${v}`).join("\n");
    confirmMcp = "";
  }

  function pairs(text: string, sep: string): [string, string][] {
    return text
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean)
      .map((l) => {
        const i = l.indexOf(sep);
        return i < 0 ? [l, ""] : [l.slice(0, i).trim(), l.slice(i + sep.length).trim()];
      });
  }

  async function saveMcp(e: Event) {
    e.preventDefault();
    if (!editing) return;
    const def: McpServerDef = {
      ...editing,
      name: editing.name.trim(),
      args: argsText.split("\n").map((l) => l.trim()).filter(Boolean),
      env: pairs(envText, "="),
      headers: pairs(headersText, ":"),
    };
    const ok = await store.saveMcp(def);
    if (ok) editing = null;
  }

  function toggleAgent(list: string[], id: string): string[] {
    return list.includes(id) ? list.filter((a) => a !== id) : [...list, id];
  }

  async function toggleEnabled(def: McpServerDef) {
    await store.saveMcp({ ...structuredClone($state.snapshot(def)), enabled: !def.enabled });
  }

  function summary(def: McpServerDef): string {
    return def.transport === "http" ? def.url : [def.command, ...def.args].join(" ");
  }

  // ----- skills -----
  let syncTo = $state<string[]>([]);
  let creating = $state(false);
  let skillName = $state("");
  let skillDesc = $state("");
  let confirmSkill = $state("");

  $effect(() => {
    // Default sync targets: every ready agent, once known.
    if (syncTo.length === 0 && store.readyAgents.length) syncTo = store.readyAgents.map((a) => a.kind);
  });

  async function createSkill(e: Event) {
    e.preventDefault();
    const ok = await store.createSkill(skillName.trim(), skillDesc.trim());
    if (ok !== undefined) {
      creating = false;
      skillName = "";
      skillDesc = "";
    }
  }

  /** Folder name of a skill path, which is what commands take. */
  function folderOf(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? "";
  }
</script>

<aside class="col">
  <div class="head"><span class="title serif">{t("library.title")}</span></div>
  <div class="nav">
    {#each entries as e (e.id)}
      <button class="entry" class:on={store.librarySection === e.id} onclick={() => (store.librarySection = e.id)}>
        <Icon name={e.icon} />
        <span class="label">{e.label}</span>
      </button>
    {/each}
  </div>
</aside>

<section class="pane">
  <div class="inner">
    <div class="top">
      <div>
        <div class="mlab">{t("library.crumb", { section: current.label })}</div>
        <h1 class="serif">{current.label}</h1>
        <p class="blurb">{current.blurb}</p>
      </div>
      <span class="grow"></span>
      <button class="btn" disabled={store.libraryLoading} onclick={() => store.loadLibrary()}>{t("library.refresh")}</button>
      <button class="btn" onclick={() => (store.view = store.currentTrack ? "track" : "new-track")}>{t("library.close")}</button>
    </div>

    {#if store.lastError}<div class="mono err">{store.lastError}</div>{/if}

    {#if store.librarySection === "mcp"}
      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("library.mcp")}</span>
          <span class="mono count">{store.mcpServers.length}</span>
          <span class="grow"></span>
          <button class="btn" onclick={() => startEdit(null)}>{t("library.add")}</button>
        </div>

        {#if editing && isNew}
          {@render mcpForm()}
        {/if}

        <div class="rows">
          {#each store.mcpServers as def (def.name)}
            {#if editing && !isNew && editing.name === def.name}
              {@render mcpForm()}
            {:else}
              <div class="row">
                <button class="sw" class:on={def.enabled} onclick={() => toggleEnabled(def)} title={def.enabled ? t("library.enabled") : t("library.disabled")} aria-pressed={def.enabled}>
                  <span class="knob"></span>
                </button>
                <span class="col2">
                  <span class="line">
                    <span class="mono id" class:off={!def.enabled}>{def.name}</span>
                    <span class="mono tag">{def.transport}</span>
                    <span class="mono dim">{def.agents.length ? def.agents.map(agentLabel).join(" · ") : t("library.allAgents")}</span>
                  </span>
                  <span class="mono desc">{summary(def)}</span>
                </span>
                {#if confirmMcp === def.name}
                  <button class="btn" onclick={() => (confirmMcp = "")}>{t("library.cancel")}</button>
                  <button class="btn danger" onclick={() => { store.deleteMcp(def.name); confirmMcp = ""; }}>{t("library.confirmDelete")}</button>
                {:else}
                  <button class="btn" onclick={() => startEdit(def)}>{t("library.edit")}</button>
                  <button class="btn" onclick={() => (confirmMcp = def.name)}>{t("library.delete")}</button>
                {/if}
              </div>
            {/if}
          {/each}
          {#if store.mcpServers.length === 0 && !editing}
            <div class="row dim mono">{t("library.none")}</div>
          {/if}
        </div>
      </div>

      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("library.agentOwn")}</span>
        </div>
        <p class="note">{t("library.agentOwnNote")}</p>
        <div class="rows">
          {#each store.agentMcp as m (m.agent + "/" + m.name)}
            <div class="row">
              <span class="mono agent">{agentLabel(m.agent)}</span>
              <span class="col2">
                <span class="line">
                  <span class="mono id">{m.name}</span>
                  <span class="mono tag">{m.transport}</span>
                </span>
                <span class="mono desc" title={m.source}>{m.transport === "http" ? m.url : [m.command, ...m.args].join(" ")}</span>
              </span>
              {#if m.in_library}
                <span class="mono dim">{t("library.inLibrary")}</span>
              {:else}
                <button class="btn" onclick={() => store.importMcp(m.agent, m.name)}>{t("library.import")}</button>
              {/if}
            </div>
          {/each}
          {#if store.agentMcp.length === 0}
            <div class="row dim mono">{t("library.none")}</div>
          {/if}
        </div>
      </div>
    {:else}
      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("library.skills")}</span>
          <span class="mono count">{store.skills.length}</span>
          <span class="grow"></span>
          {#if store.info?.skills_dir}
            <button class="btn" onclick={() => store.reveal(store.info!.skills_dir)}>{t("library.openFolder")}</button>
          {/if}
          <button class="btn" onclick={() => (creating = !creating)}>{t("library.newSkill")}</button>
        </div>
        {#if store.info?.skills_dir}
          <div class="mono path"><span class="dim">{t("library.libraryDir")}</span> {store.info.skills_dir}</div>
        {/if}

        {#if creating}
          <form class="form" onsubmit={createSkill}>
            <label class="field">
              <span class="mlab-sm">{t("library.skillName")}</span>
              <input class="mono" type="text" bind:value={skillName} maxlength="64" spellcheck="false" />
            </label>
            <label class="field">
              <span class="mlab-sm">{t("library.skillDesc")}</span>
              <input type="text" bind:value={skillDesc} maxlength="200" />
            </label>
            <div class="actions">
              <span class="grow"></span>
              <button class="btn" type="button" onclick={() => (creating = false)}>{t("library.cancel")}</button>
              <button class="btn btn-acc" type="submit" disabled={!skillName.trim()}>{t("library.create")}</button>
            </div>
          </form>
        {/if}

        <div class="rows">
          {#each store.skills as s (s.path)}
            <div class="row">
              <span class="col2">
                <span class="line">
                  <span class="mono id">{s.name}</span>
                  {#each s.installed as a (a)}<span class="mono tag ok">{agentLabel(a)}</span>{/each}
                  {#each s.foreign as a (a)}<span class="mono tag warn" title={t("library.foreignOn")}>{agentLabel(a)}</span>{/each}
                  {#if s.installed.length === 0 && s.foreign.length === 0}<span class="mono dim">{t("library.notInstalled")}</span>{/if}
                </span>
                <span class="desc">{s.description || s.path}</span>
              </span>
              <button class="btn" onclick={() => store.reveal(s.path)}>{t("library.openFolder")}</button>
              {#if confirmSkill === s.path}
                <button class="btn" onclick={() => (confirmSkill = "")}>{t("library.cancel")}</button>
                <button class="btn danger" onclick={() => { store.removeSkill(folderOf(s.path)); confirmSkill = ""; }}>{t("library.confirmDelete")}</button>
              {:else}
                <button class="btn" onclick={() => (confirmSkill = s.path)}>{t("library.delete")}</button>
              {/if}
            </div>
          {/each}
          {#if store.skills.length === 0}
            <div class="row dim mono">{t("library.none")}</div>
          {/if}
        </div>

        <div class="syncbar">
          <span class="mlab-sm">{t("library.syncTo")}</span>
          {#each agentIds as id (id)}
            <label class="check mono">
              <input type="checkbox" checked={syncTo.includes(id)} onchange={() => (syncTo = toggleAgent(syncTo, id))} />
              {agentLabel(id)}
            </label>
          {/each}
          <span class="grow"></span>
          <button class="btn btn-acc" disabled={store.skills.length === 0 || syncTo.length === 0} onclick={() => store.syncSkills(syncTo)}>{t("library.sync")}</button>
        </div>
        {#if store.syncReport}
          <div class="mono report">
            {t("library.syncDone", { copied: store.syncReport.copied.length, removed: store.syncReport.removed.length, skipped: store.syncReport.skipped.length })}
            {#if store.syncReport.skipped.length}
              <span class="dim"> · {store.syncReport.skipped.join(", ")} · {t("library.skipNote")}</span>
            {/if}
          </div>
        {/if}
      </div>

      <div class="card">
        <div class="cardhead">
          <span class="ctitle">{t("library.agentOwnSkills")}</span>
        </div>
        <p class="note">{t("library.agentOwnSkillsNote")}</p>
        <div class="rows">
          {#each store.agentSkills as s (s.path)}
            <div class="row">
              <span class="mono agent">{agentLabel(s.agent)}</span>
              <span class="col2">
                <span class="line">
                  <span class="mono id">{s.name}</span>
                  {#if s.managed}<span class="mono tag">{t("library.managed")}</span>{/if}
                </span>
                <span class="desc" title={s.path}>{s.description || s.path}</span>
              </span>
              {#if s.in_library}
                <span class="mono dim">{t("library.inLibrary")}</span>
              {:else}
                <button class="btn" onclick={() => store.importSkill(s.agent, folderOf(s.path))}>{t("library.import")}</button>
              {/if}
            </div>
          {/each}
          {#if store.agentSkills.length === 0}
            <div class="row dim mono">{t("library.none")}</div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</section>

{#snippet mcpForm()}
  {#if editing}
    <form class="form" onsubmit={saveMcp}>
      <div class="two">
        <label class="field">
          <span class="mlab-sm">{t("library.name")}</span>
          <input class="mono" type="text" bind:value={editing.name} placeholder={t("library.namePh")} disabled={!isNew} spellcheck="false" />
        </label>
        <div class="field">
          <span class="mlab-sm">{t("library.transport")}</span>
          <div class="seg" role="radiogroup">
            <button type="button" class="segopt mono" class:on={editing.transport === "stdio"} role="radio" aria-checked={editing.transport === "stdio"} onclick={() => (editing!.transport = "stdio")}>{t("library.stdio")}</button>
            <button type="button" class="segopt mono" class:on={editing.transport === "http"} role="radio" aria-checked={editing.transport === "http"} onclick={() => (editing!.transport = "http")}>{t("library.http")}</button>
          </div>
        </div>
      </div>
      {#if editing.transport === "stdio"}
        <label class="field">
          <span class="mlab-sm">{t("library.command")}</span>
          <input class="mono" type="text" bind:value={editing.command} placeholder={t("library.commandPh")} spellcheck="false" />
          <span class="note">{t("library.commandNote")}</span>
        </label>
        <div class="two">
          <label class="field">
            <span class="mlab-sm">{t("library.args")}</span>
            <textarea class="mono" rows="4" bind:value={argsText} spellcheck="false"></textarea>
          </label>
          <label class="field">
            <span class="mlab-sm">{t("library.env")}</span>
            <textarea class="mono" rows="4" bind:value={envText} spellcheck="false"></textarea>
          </label>
        </div>
      {:else}
        <label class="field">
          <span class="mlab-sm">{t("library.url")}</span>
          <input class="mono" type="text" bind:value={editing.url} placeholder="https://…/mcp" spellcheck="false" />
        </label>
        <label class="field">
          <span class="mlab-sm">{t("library.headers")}</span>
          <textarea class="mono" rows="3" bind:value={headersText} spellcheck="false"></textarea>
        </label>
      {/if}
      <div class="field">
        <span class="mlab-sm">{t("library.agentsFor")}</span>
        <div class="checks">
          {#each agentIds as id (id)}
            <label class="check mono">
              <input type="checkbox" checked={editing.agents.includes(id)} onchange={() => (editing!.agents = toggleAgent(editing!.agents, id))} />
              {agentLabel(id)}
            </label>
          {/each}
        </div>
      </div>
      <div class="actions">
        <span class="grow"></span>
        <button class="btn" type="button" onclick={() => (editing = null)}>{t("library.cancel")}</button>
        <button class="btn btn-acc" type="submit" disabled={!editing.name.trim()}>{t("library.save")}</button>
      </div>
    </form>
  {/if}
{/snippet}

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
    color: var(--dim);
    font-size: 13px;
    text-align: left;
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
    max-width: 860px;
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

  .blurb,
  .note {
    margin: 9px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--dim);
    max-width: 640px;
  }

  .note {
    padding: 0 16px 10px;
    font-size: 11px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .err {
    font-size: 11px;
    color: var(--acct);
    margin-bottom: 14px;
    word-break: break-word;
  }

  .card {
    border: 1px solid var(--line);
    background: var(--card);
    margin-bottom: 18px;
  }

  .cardhead {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }

  .ctitle {
    font-size: 13px;
    color: var(--hi);
  }

  .count {
    font-size: 10px;
    color: var(--lab);
  }

  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    font-size: 12px;
    color: var(--txt);
    border-top: 1px solid var(--lineq);
  }

  .row:first-child {
    border-top: 0;
  }

  .row.dim {
    color: var(--lab);
    font-size: 11px;
  }

  .col2 {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .id {
    font-size: 12px;
    color: var(--hi);
  }

  .id.off {
    color: var(--lab);
    text-decoration: line-through;
  }

  .tag {
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--lab);
    border: 1px solid var(--line);
    padding: 1px 5px;
  }

  .tag.ok {
    color: var(--ok);
    border-color: var(--ok);
  }

  .tag.warn {
    color: var(--warn);
    border-color: var(--warn);
  }

  .dim {
    font-size: 10px;
    color: var(--lab);
  }

  .desc {
    font-size: 11px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .agent {
    width: 92px;
    flex-shrink: 0;
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--dim);
  }

  .path {
    padding: 8px 16px;
    font-size: 10px;
    color: var(--txt);
    border-bottom: 1px solid var(--lineq);
    word-break: break-all;
  }

  /* On/off: a hairline track with a square knob. */
  .sw {
    width: 30px;
    height: 16px;
    flex-shrink: 0;
    padding: 2px;
    background: var(--inp);
    border: 1px solid var(--line);
    display: flex;
    align-items: center;
  }

  .knob {
    width: 10px;
    height: 10px;
    background: var(--idle);
    transition: transform 120ms linear;
  }

  .sw.on {
    border-color: var(--acc);
  }

  .sw.on .knob {
    background: var(--acc);
    transform: translateX(14px);
  }

  .btn.danger:not(:disabled) {
    color: var(--acct);
    border-color: var(--acct);
  }

  .form {
    padding: 6px 16px 14px;
    border-bottom: 1px solid var(--lineq);
  }

  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 0 4px;
  }

  input,
  textarea {
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 10px;
    outline: none;
  }

  input[type="text"] {
    height: 32px;
  }

  textarea {
    padding: 8px 10px;
    resize: vertical;
    line-height: 1.5;
  }

  input.mono,
  textarea.mono {
    font-family: var(--mono);
    font-size: 11px;
  }

  input:focus,
  textarea:focus {
    border-color: var(--acc);
  }

  input:disabled {
    color: var(--lab);
  }

  .seg {
    display: flex;
    border: 1px solid var(--line);
    height: 32px;
  }

  .segopt {
    flex: 1;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    color: var(--dim);
    font-size: 10px;
    letter-spacing: 0.08em;
  }

  .segopt:last-child {
    border-right: 0;
  }

  .segopt.on {
    color: var(--hi);
    background: var(--sel);
  }

  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
  }

  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--dim);
  }

  .check input {
    accent-color: var(--acc);
    margin: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 12px;
  }

  .syncbar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 16px;
    border-top: 1px solid var(--line);
  }

  .report {
    padding: 8px 16px 12px;
    font-size: 10px;
    color: var(--txt);
  }
</style>
