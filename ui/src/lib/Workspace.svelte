<script lang="ts">
  import { store, type WsEntry } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import Markdown from "./Markdown.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { highlight, languageFor } from "./highlight";
  import { t } from "./i18n.svelte";

  /**
   * The track's working folder beside the conversation, after Kiro Crew's
   * right panel: a tab strip (changes, files, then each open file), the
   * git changes with a diff, the file tree with a filter, and an open file
   * shown as a markdown preview, coloured code or an image, with the tree
   * kept beside it.
   */
  let filter = $state("");
  let folded = $state<Record<string, boolean>>({});

  const activeFile = $derived(store.files[store.activeFile]);

  /** The tree as nested rows: every entry, with its depth and whether a folded ancestor hides it. */
  type Row = WsEntry & { depth: number; visible: boolean };
  const rows = $derived.by((): Row[] => {
    const q = filter.trim().toLowerCase();
    const out: Row[] = [];
    for (const e of store.tree) {
      const parts = e.path.split("/");
      const depth = parts.length - 1;
      if (q) {
        // A filter flattens the tree to matching files with their folders shown for context.
        if (e.dir) continue;
        if (!e.path.toLowerCase().includes(q)) continue;
        out.push({ ...e, depth: 0, visible: true });
        continue;
      }
      let hidden = false;
      for (let i = 1; i < parts.length; i++) {
        if (folded[parts.slice(0, i).join("/")] !== false && !(parts.slice(0, i).join("/") in folded)) {
          // Folders start folded unless opened, except the top level.
          hidden = hidden || i > 1 || false;
        }
        if (folded[parts.slice(0, i).join("/")]) hidden = true;
      }
      out.push({ ...e, depth, visible: !hidden });
    }
    return out;
  });

  function isOpen(path: string): boolean {
    // Top-level folders start open; deeper ones closed.
    return path in folded ? !folded[path] : !path.includes("/");
  }

  function toggle(path: string) {
    folded = { ...folded, [path]: isOpen(path) };
  }

  /** Whether every ancestor folder of a row is open. */
  function shown(row: Row): boolean {
    const parts = row.path.split("/");
    for (let i = 1; i < parts.length; i++) {
      if (!isOpen(parts.slice(0, i).join("/"))) return false;
    }
    return true;
  }

  function ext(name: string): string {
    const i = name.lastIndexOf(".");
    return i > 0 ? name.slice(i + 1).toLowerCase() : "";
  }

  function kb(n: number): string {
    return n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;
  }

  /** Short label for a porcelain code. */
  function codeLabel(code: string): string {
    if (code === "??") return "new";
    const c = code.trim()[0] ?? "";
    return { M: "mod", A: "add", D: "del", R: "ren", C: "copy", U: "conf", T: "type" }[c] ?? code.trim();
  }

  function codeTone(code: string): string {
    if (code === "??" || code.includes("A")) return "var(--ok)";
    if (code.includes("D")) return "var(--acct)";
    return "var(--warn)";
  }

  const diffHtml = $derived(store.diffPath ? highlight(store.diffs[store.diffPath] ?? "", "diff") : "");
  const codeHtml = $derived(activeFile?.text != null ? highlight(activeFile.text, languageFor(activeFile.ext)) : "");
  const crumbs = $derived(store.activeFile ? store.activeFile.split("/") : []);
  const raw = $derived(!!store.rawMarkdown[store.activeFile]);
</script>

<aside style="width: {store.panelWidth}px">
  <SplitHandle
    edge="left"
    width={store.panelWidth}
    min={320}
    max={Math.max(360, Math.floor(window.innerWidth * 0.7))}
    reset={460}
    label={t("ws.width")}
    onchange={(px, persist) => store.setPanelWidth(px, persist)}
  />

  <!-- tab strip -->
  <div class="tabs">
    <button class="tab icon" class:on={store.panelTab === "changes"} title={t("ws.changes")} onclick={() => { store.panelTab = "changes"; store.loadGit(); }}>
      <Icon name="changes" size={14} />
      {#if store.git?.changes.length}<span class="mono badge">{store.git.changes.length}</span>{/if}
    </button>
    <button class="tab icon" class:on={store.panelTab === "files"} title={t("ws.files")} onclick={() => { store.panelTab = "files"; if (!store.tree.length) store.loadTree(); }}>
      <Icon name="files" size={14} />
    </button>
    <div class="filetabs">
      {#each store.openFiles as path (path)}
        <div class="tab file" class:on={store.panelTab === "file" && store.activeFile === path}>
          <button class="fname mono" onclick={() => { store.activeFile = path; store.panelTab = "file"; }} title={path}>{path.split("/").at(-1)}</button>
          <button class="x" onclick={() => store.closeFile(path)} aria-label={t("ws.closeFile")}><Icon name="close" size={10} /></button>
        </div>
      {/each}
    </div>
    <span class="grow"></span>
    <button class="tab icon" title={t("ws.refresh")} disabled={store.treeLoading || store.gitLoading} onclick={() => store.refreshWorkspace()}>
      <Icon name="refresh" size={14} />
    </button>
    <button class="tab icon" title={t("ws.close")} onclick={() => store.setPanel(false)}>
      <Icon name="panel" size={14} />
    </button>
  </div>

  {#if store.panelTab === "changes"}
    <div class="body">
      {#if !store.git}
        <div class="mono empty">{store.gitLoading ? t("ws.loading") : t("ws.none")}</div>
      {:else if !store.git.repo}
        <div class="mono empty">{t("ws.noRepo")}</div>
      {:else}
        <div class="gitline mono">
          <span class="dim">{t("ws.branch")}</span> {store.git.branch || "—"}
          <span class="grow"></span>
          <span class="dim">{t("ws.changed", { n: store.git.changes.length })}</span>
        </div>
        {#if store.git.changes.length === 0}
          <div class="mono empty">{t("ws.clean")}</div>
        {/if}
        <div class="changes" class:short={!!store.diffPath}>
          {#each store.git.changes as c (c.path)}
            <button class="row" class:on={store.diffPath === c.path} onclick={() => store.loadDiff(c.path)} title={c.from ? `${c.from} → ${c.path}` : c.path}>
              <span class="mono status" style="color: {codeTone(c.code)}">{codeLabel(c.code)}</span>
              <span class="mono cpath">{c.path}</span>
            </button>
          {/each}
        </div>
        {#if store.diffPath}
          <div class="diffhead">
            <span class="mono cpath">{store.diffPath}</span>
            <span class="grow"></span>
            <button class="btn sm" onclick={() => store.openFile(store.diffPath)}>{t("ws.open")}</button>
            <button class="btn sm" onclick={() => (store.diffPath = "")}>{t("ws.closeDiff")}</button>
          </div>
          <pre class="src diff hljs">{@html diffHtml}</pre>
        {/if}
      {/if}
    </div>
  {:else if store.panelTab === "files"}
    {@render tree(false)}
  {:else}
    <div class="filehead">
      <nav class="crumbs mono">
        {#each crumbs as c, i (i)}
          {#if i > 0}<span class="sep">›</span>{/if}
          <span class:last={i === crumbs.length - 1}>{c}</span>
        {/each}
      </nav>
      <span class="grow"></span>
      {#if activeFile?.kind === "markdown"}
        <button class="btn sm" onclick={() => (store.rawMarkdown = { ...store.rawMarkdown, [store.activeFile]: !raw })}>{raw ? t("ws.preview") : t("ws.raw")}</button>
      {/if}
      <button class="btn sm" onclick={() => store.revealFile(store.activeFile)}>{t("ws.reveal")}</button>
      <button class="tab icon" class:on={store.panelTree} title={t("ws.toggleTree")} onclick={() => (store.panelTree = !store.panelTree)}>
        <Icon name="files" size={14} />
      </button>
    </div>
    <div class="split">
      <div class="content">
        {#if !activeFile}
          <div class="mono empty">{t("ws.loading")}</div>
        {:else if activeFile.kind === "markdown" && !raw}
          <div class="mdwrap"><Markdown source={activeFile.text ?? ""} /></div>
        {:else if activeFile.kind === "markdown" || activeFile.kind === "text"}
          <pre class="src hljs">{@html codeHtml}</pre>
        {:else if activeFile.kind === "image"}
          <div class="imgwrap"><img src={activeFile.data_url} alt={activeFile.name} /></div>
        {:else}
          <div class="mono empty">{activeFile.kind === "binary" ? t("ws.binary") : t("ws.large")} · {kb(activeFile.size)}</div>
        {/if}
      </div>
      {#if store.panelTree}
        <div class="side">{@render tree(true)}</div>
      {/if}
    </div>
  {/if}
</aside>

<!-- The file tree, with its filter. `compact` drops the file sizes. -->
{#snippet tree(compact: boolean)}
  <div class="treewrap">
    <div class="treebar">
      <input class="filter" type="text" bind:value={filter} placeholder={t("ws.filter")} aria-label={t("ws.filter")} spellcheck="false" />
    </div>
    <div class="tree">
      {#if store.tree.length === 0}
        <div class="mono empty">{store.treeLoading ? t("ws.loading") : t("ws.emptyDir")}</div>
      {/if}
      {#each rows as r (r.path)}
        {#if filter.trim() || shown(r)}
          {#if r.dir}
            <button class="node" style="padding-left: {10 + r.depth * 14}px" onclick={() => toggle(r.path)}>
              <span class="mono chev">{isOpen(r.path) ? "▾" : "▸"}</span>
              <span class="name">{r.name}</span>
            </button>
          {:else}
            <button class="node file" class:on={store.activeFile === r.path && store.panelTab === "file"} style="padding-left: {filter.trim() ? 10 : 24 + r.depth * 14}px" onclick={() => store.openFile(r.path)} title={r.path}>
              <span class="mono ext">{ext(r.name) || "·"}</span>
              <span class="name">{filter.trim() ? r.path : r.name}</span>
              {#if !compact}<span class="mono size">{kb(r.size)}</span>{/if}
            </button>
          {/if}
        {/if}
      {/each}
    </div>
  </div>
{/snippet}

<style>
  aside {
    position: relative;
    flex-shrink: 0;
    border-left: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .tabs {
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--line);
    padding-left: 8px;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    background: transparent;
    border: 0;
    border-bottom: 1px solid transparent;
    color: var(--dim);
  }

  .tab.icon {
    width: 34px;
    justify-content: center;
    padding: 0;
    position: relative;
  }

  .tab:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .tab.on {
    color: var(--hi);
    border-bottom-color: var(--acc);
  }

  .tab:disabled {
    opacity: 0.5;
  }

  .badge {
    position: absolute;
    top: 6px;
    right: 3px;
    font-size: 8px;
    color: var(--acct);
  }

  .filetabs {
    display: flex;
    min-width: 0;
    overflow-x: auto;
    border-left: 1px solid var(--line);
    margin-left: 4px;
  }

  .tab.file {
    padding: 0 4px 0 10px;
    gap: 4px;
    max-width: 180px;
    border-right: 1px solid var(--lineq);
  }

  .fname {
    background: transparent;
    border: 0;
    color: inherit;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0;
  }

  .x {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .x:hover {
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .empty {
    padding: 18px 14px;
    font-size: 11px;
    color: var(--lab);
  }

  .gitline {
    display: flex;
    gap: 8px;
    padding: 10px 14px;
    font-size: 11px;
    color: var(--txt);
    border-bottom: 1px solid var(--lineq);
  }

  .dim {
    color: var(--lab);
  }

  .changes {
    overflow-y: auto;
    flex-shrink: 0;
  }

  .changes.short {
    max-height: 38%;
    border-bottom: 1px solid var(--line);
  }

  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 14px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    color: var(--dim);
    text-align: left;
  }

  .row:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .row.on {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .status {
    width: 32px;
    flex-shrink: 0;
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .cpath {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .diffhead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--lineq);
    color: var(--dim);
  }

  .btn.sm {
    height: 24px;
    padding: 0 8px;
  }

  pre.src {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 12px 14px;
    overflow: auto;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.6;
    color: var(--body);
    background: transparent;
    white-space: pre;
    tab-size: 4;
  }

  .filehead {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 14px;
    border-bottom: 1px solid var(--line);
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
  }

  .crumbs .last {
    color: var(--hi);
  }

  .sep {
    color: var(--lab);
  }

  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .content {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
  }

  .side {
    width: 42%;
    min-width: 180px;
    flex-shrink: 0;
    border-left: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .mdwrap {
    padding: 18px 22px 28px;
    font-size: var(--chat-fs);
    line-height: 1.7;
    color: var(--txt);
  }

  .imgwrap {
    padding: 18px;
  }

  .imgwrap img {
    max-width: 100%;
    display: block;
    border: 1px solid var(--line);
  }

  .treewrap {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .treebar {
    flex-shrink: 0;
    padding: 8px 10px 4px;
  }

  .filter {
    width: 100%;
    height: 28px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 11px;
    padding: 0 9px;
    outline: none;
  }

  .filter:focus {
    border-color: var(--acc);
  }

  .tree {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 2px 0 10px;
  }

  .node {
    width: 100%;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding-right: 10px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    color: var(--dim);
    text-align: left;
    font-size: 12px;
    white-space: nowrap;
  }

  .node:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .node.on {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .chev {
    width: 10px;
    font-size: 9px;
    color: var(--lab);
  }

  .ext {
    width: 26px;
    flex-shrink: 0;
    font-size: 9px;
    letter-spacing: 0.06em;
    color: var(--lab);
    text-transform: uppercase;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .size {
    margin-left: auto;
    font-size: 9px;
    color: var(--lab);
  }
</style>
