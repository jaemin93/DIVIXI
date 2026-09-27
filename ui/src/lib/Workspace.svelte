<script lang="ts">
  import { store, type WsEntry } from "./store.svelte";
  import { kb as knowledge } from "./knowledge.svelte";
  import Icon from "./Icon.svelte";
  import Markdown from "./Markdown.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import CodeEditor from "./CodeEditor.svelte";
  import { highlight, languageFor } from "./highlight";
  import { t } from "./i18n.svelte";
  import { invoke, previewBase, saveAs, local } from "./ipc.svelte";

  /**
   * The track's working folder beside the conversation, after Kiro Crew's
   * right panel: a tab strip (changes, files, then each open file), the
   * git changes with a diff, the file tree with a filter, and an open file
   * shown as a markdown preview, coloured code or an image. Over an open
   * file the tree opens as a drawer from the right, as in Kiro: the file
   * stays where it is underneath; a click beside the drawer or Esc closes it.
   */
  let filter = $state("");
  let folded = $state<Record<string, boolean>>({});
  /** The drawer shows the files or the git changes. */
  let drawerMode = $state<"files" | "changes">("files");
  /** The filter goes by file name or by what files contain. */
  let searchBy = $state<"name" | "content">("name");

  // ----- content search -----
  type Found = { path: string; line: number; text: string };
  let found = $state<Found[]>([]);
  let foundCut = $state(false);
  let searching = $state(false);
  let searchSeq = 0;
  $effect(() => {
    const q = filter.trim();
    const track = store.track;
    if (searchBy !== "content" || !q || !track) {
      found = [];
      foundCut = false;
      return;
    }
    // A moment after typing stops; only the newest answer lands.
    const seq = ++searchSeq;
    const timer = setTimeout(async () => {
      searching = true;
      try {
        const [lines, cut] = await invoke<[Found[], boolean]>("workspace_search", { track, query: q });
        if (seq === searchSeq) {
          found = lines;
          foundCut = cut;
        }
      } catch (err) {
        if (seq === searchSeq) store.lastError = String(err);
      } finally {
        if (seq === searchSeq) searching = false;
      }
    }, 300);
    return () => clearTimeout(timer);
  });
  /** Found lines grouped by file, files in the order found. */
  const foundByFile = $derived.by(() => {
    const groups: { path: string; lines: Found[] }[] = [];
    for (const f of found) {
      const last = groups.at(-1);
      if (last && last.path === f.path) last.lines.push(f);
      else groups.push({ path: f.path, lines: [f] });
    }
    return groups;
  });

  // ----- a file's right-click menu -----
  let menu = $state<{ path: string; x: number; y: number } | null>(null);
  function fileMenu(e: MouseEvent, path: string) {
    e.preventDefault();
    e.stopPropagation();
    // Kept inside the window.
    menu = { path, x: Math.min(e.clientX, window.innerWidth - 200), y: Math.min(e.clientY, window.innerHeight - 90) };
  }
  async function download(path: string) {
    menu = null;
    try {
      await saveAs(store.track, path);
    } catch (err) {
      store.lastError = String(err);
    }
  }
  function attachToMessage(path: string) {
    menu = null;
    void store.attach([store.absoluteInTrack(path)]);
  }

  /** The line with what was searched for marked, as safe HTML. */
  function marked(text: string): string {
    const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    const q = filter.trim();
    if (!q) return esc(text);
    const at = text.toLowerCase().indexOf(q.toLowerCase());
    if (at < 0) return esc(text);
    return esc(text.slice(0, at)) + "<mark>" + esc(text.slice(at, at + q.length)) + "</mark>" + esc(text.slice(at + q.length));
  }

  const activeFile = $derived(store.files[store.activeFile]);

  // Whether the open file is in the knowledge library; the book button adds it.
  let kbAdding = $state(false);
  $effect(() => {
    const track = store.track;
    const path = store.activeFile;
    if (track && path && store.panelTab === "file") void knowledge.checkViewer(track, path);
  });
  const inLibrary = $derived(!!knowledge.viewerSource && knowledge.viewerKey === `${store.track}\0${store.activeFile}`);
  const canAdd = $derived(!!activeFile && activeFile.kind !== "image" && knowledge.formats.includes(activeFile.ext.toLowerCase()));
  async function addToLibrary() {
    const track = store.track;
    const path = store.activeFile;
    if (!track || !path) return;
    kbAdding = true;
    const why = await knowledge.add(path, track);
    kbAdding = false;
    if (why) store.lastError = why;
    await knowledge.checkViewer(track, path);
  }
  $effect(() => {
    if (!knowledge.formats.length) void knowledge.loadFormats();
  });

  /** The tree as rows with their depth; `shown` decides folding at render time. */
  type Row = WsEntry & { depth: number };
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
        out.push({ ...e, depth: 0 });
        continue;
      }
      out.push({ ...e, depth });
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
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1048576).toFixed(1)} MB`;
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

  /** A plain mouse wheel over the tab strip scrolls it sideways. */
  function scrollTabs(e: WheelEvent) {
    const el = e.currentTarget as HTMLElement;
    if (el.scrollWidth <= el.clientWidth || e.deltaX !== 0) return;
    el.scrollLeft += e.deltaY;
    e.preventDefault();
  }

  const diffHtml = $derived(store.diffPath ? highlight(store.diffs[store.diffPath] ?? "", "diff") : "");
  const codeHtml = $derived(activeFile?.text !== undefined && activeFile.text !== null && !activeFile.editable ? highlight(activeFile.text, languageFor(activeFile.ext)) : "");
  const crumbs = $derived(store.activeFile ? store.activeFile.split("/") : []);
  const raw = $derived(!!store.rawMarkdown[store.activeFile]);
  /** Where the preview protocol serves a file of the current track. */
  function previewUrl(path: string): string {
    const base = previewBase();
    // The theme picks the scrollbar colours the page is served with.
    return base + [store.track ?? "", ...path.split("/")].map(encodeURIComponent).join("/") + `?theme=${store.theme}`;
  }

  /** Markdown and HTML open rendered, with their source a toggle away. */
  const previewable = $derived(activeFile?.kind === "markdown" || activeFile?.kind === "html");
  /** Code and text open straight in the editor; markdown and HTML when shown as source. */
  const editing = $derived(!!activeFile?.editable && (activeFile.kind === "text" || (previewable && raw)));
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && menu) menu = null;
    else if (e.key === "Escape" && store.panelTree && store.panelTab === "file") store.panelTree = false;
  }}
  onpointerdown={(e) => {
    if (menu && !(e.target as HTMLElement | null)?.closest?.(".fmenu")) menu = null;
  }}
  onblur={() => (menu = null)}
/>

{#if menu}
  {@const path = menu.path}
  <div class="fmenu" role="menu" style="left: {menu.x}px; top: {menu.y}px">
    <button class="fitem" role="menuitem" onclick={() => attachToMessage(path)}>{t("ws.attachToMessage")}</button>
    <button class="fitem" role="menuitem" onclick={() => download(path)}>{t("ws.download")}</button>
  </div>
{/if}

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
    <div class="filetabs" onwheel={scrollTabs}>
      {#each store.openFiles as path (path)}
        <div class="tab file" class:on={store.panelTab === "file" && store.activeFile === path}>
          <button class="fname mono" onclick={() => { store.activeFile = path; store.panelTab = "file"; }} title={path}>{path.split("/").at(-1)}</button>
          {#if store.closeAsked === path}
            <button class="mono discard" onclick={() => store.closeFile(path)} title={t("ws.discardTitle")}>{t("ws.discard")}</button>
          {:else if store.isDirty(path)}
            <button class="x dirty" onclick={() => store.closeFile(path)} aria-label={t("ws.unsavedClose")} title={t("ws.unsavedClose")}><span class="dot"></span></button>
          {:else}
            <button class="x" onclick={() => store.closeFile(path)} aria-label={t("ws.closeFile")}><Icon name="close" size={10} /></button>
          {/if}
        </div>
      {/each}
    </div>
    <span class="grow"></span>
    <button class="tab icon" title={t("ws.refresh")} disabled={store.treeLoading || store.gitLoading} onclick={() => store.refreshWorkspace()}>
      <Icon name="refresh" size={14} />
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
      {#if editing && store.isDirty(store.activeFile)}
        <button class="btn sm btn-acc" disabled={store.isSaving(store.activeFile)} onclick={() => store.saveFile(store.activeFile)} title="Ctrl+S">{t("ws.save")}</button>
      {/if}
      {#if previewable}
        <button class="btn sm" onclick={() => (store.rawMarkdown = { ...store.rawMarkdown, [store.activeFile]: !raw })}>{raw ? t("ws.preview") : t("ws.raw")}</button>
      {/if}
      {#if inLibrary}
        <button class="tab icon kbin" title={t("ws.kbIn", { status: knowledge.viewerSource?.status ?? "" })} onclick={() => knowledge.show("sources")}>
          <Icon name="book" size={14} />
        </button>
      {:else if canAdd}
        <button class="tab icon" title={t("ws.kbAdd")} disabled={kbAdding} onclick={addToLibrary}>
          <Icon name="book" size={14} />
        </button>
      {/if}
      {#if local}<button class="btn sm" onclick={() => store.revealFile(store.activeFile)}>{t("ws.reveal")}</button>{/if}
      <button class="tab icon" class:on={store.panelTree} title={t("ws.toggleTree")} onclick={() => (store.panelTree = !store.panelTree)}>
        <Icon name="files" size={14} />
      </button>
    </div>
    <div class="split">
      <div class="content">
        {#if !activeFile}
          <div class="mono empty">{t("ws.loading")}</div>
        {:else if activeFile.kind === "markdown" && !raw}
          <div class="mdwrap"><Markdown source={store.textOf(store.activeFile)} base={store.activeFile.split("/").slice(0, -1).join("/")} /></div>
        {:else if activeFile.kind === "html" && !raw}
          {#if store.isDirty(store.activeFile)}<div class="previewnote">{t("ws.previewSaved")}</div>{/if}
          <!-- Served from its own origin, sandboxed: its scripts run, apart from the app. Reloaded when the file is. -->
          {#key activeFile}
            <iframe class="htmlframe" sandbox="allow-scripts" src={previewUrl(store.activeFile)} title={activeFile.name}></iframe>
          {/key}
        {:else if editing}
          {#if store.hasConflict(store.activeFile)}
            <div class="conflict">
              <span>{t("ws.conflict")}</span>
              <span class="grow"></span>
              <button class="btn sm" onclick={() => store.saveFile(store.activeFile, true)}>{t("ws.overwrite")}</button>
              <button class="btn sm" onclick={() => store.discardDraft(store.activeFile)}>{t("ws.reload")}</button>
            </div>
          {/if}
          {#key store.activeFile}
            <CodeEditor
              goto={store.gotoLine?.path === store.activeFile ? store.gotoLine : null}
              value={store.textOf(store.activeFile)}
              language={languageFor(activeFile.ext)}
              onchange={(text) => store.setDraft(store.activeFile, text)}
              onsave={() => store.saveFile(store.activeFile)}
            />
          {/key}
        {:else if previewable || activeFile.kind === "text"}
          <pre class="src hljs">{@html codeHtml}</pre>
        {:else if activeFile.kind === "image"}
          <div class="imgwrap"><img src={activeFile.data_url} alt={activeFile.name} /></div>
        {:else}
          <div class="mono empty">{activeFile.kind === "binary" ? t("ws.binary") : t("ws.large")} · {kb(activeFile.size)}</div>
        {/if}
      </div>
      {#if store.panelTree}
        <!-- A click on the file beside the drawer closes it (and goes no further). -->
        <button class="scrim" type="button" aria-label={t("ws.closeTree")} onclick={() => (store.panelTree = false)}></button>
        <div class="drawer" role="dialog" aria-label={t("ws.files")}>{@render tree(true)}</div>
      {/if}
    </div>
  {/if}
</aside>

<!-- The file tree, with its filter. `compact` (the drawer over a file) drops the
     file sizes and adds a files / changes switch, as in Kiro. -->
{#snippet tree(compact: boolean)}
  <div class="treewrap">
    <div class="treebar">
      {#if compact}
        <div class="modes" role="tablist" aria-label={t("ws.files")}>
          <button class="mode" class:on={drawerMode === "files"} role="tab" aria-selected={drawerMode === "files"} title={t("ws.files")} onclick={() => (drawerMode = "files")}>
            <Icon name="files" size={13} />
          </button>
          <button
            class="mode"
            class:on={drawerMode === "changes"}
            role="tab"
            aria-selected={drawerMode === "changes"}
            title={t("ws.changes")}
            onclick={() => {
              drawerMode = "changes";
              if (!store.git) store.loadGit();
            }}
          >
            <span class="mono pm">±</span>
          </button>
        </div>
      {/if}
      {#if !compact || drawerMode === "files"}
        <input
          class="filter"
          type="text"
          bind:value={filter}
          placeholder={searchBy === "content" ? t("ws.searchContent") : t("ws.filter")}
          aria-label={t("ws.filter")}
          spellcheck="false"
        />
      {:else}
        <span class="grow"></span>
      {/if}
      {#if compact}
        <button class="tab icon refresh" title={t("ws.refresh")} disabled={store.treeLoading || store.gitLoading} onclick={() => store.refreshWorkspace()}>
          <Icon name="refresh" size={13} />
        </button>
      {/if}
    </div>

    {#if compact && drawerMode === "changes"}
      <div class="tree">
        {#if !store.git}
          <div class="mono empty">{store.gitLoading ? t("ws.loading") : t("ws.none")}</div>
        {:else if !store.git.repo}
          <div class="mono empty">{t("ws.noRepo")}</div>
        {:else if store.git.changes.length === 0}
          <div class="mono empty">{t("ws.clean")}</div>
        {/if}
        {#each store.git?.changes ?? [] as c (c.path)}
          <!-- A change opens its diff in the changes view. -->
          <button
            class="node file"
            title={c.from ? `${c.from} → ${c.path}` : c.path}
            onclick={() => {
              store.panelTree = false;
              store.panelTab = "changes";
              void store.loadDiff(c.path);
            }}
          >
            <span class="mono ext" style="color: {codeTone(c.code)}">{codeLabel(c.code)}</span>
            <span class="name">{c.path}</span>
          </button>
        {/each}
      </div>
    {:else}
      <div class="searchby" role="tablist" aria-label={t("ws.searchBy")}>
        <button class="sb" class:on={searchBy === "name"} role="tab" aria-selected={searchBy === "name"} onclick={() => (searchBy = "name")}>{t("ws.byName")}</button>
        <button class="sb" class:on={searchBy === "content"} role="tab" aria-selected={searchBy === "content"} onclick={() => (searchBy = "content")}>{t("ws.byContent")}</button>
      </div>
      <div class="tree">
        {#if searchBy === "content" && filter.trim()}
          {#if searching && !found.length}
            <div class="mono empty">{t("ws.searching")}</div>
          {:else if !found.length}
            <div class="mono empty">{t("ws.noMatch")}</div>
          {/if}
          {#each foundByFile as g (g.path)}
            <button class="node file" class:on={store.activeFile === g.path && store.panelTab === "file"} onclick={() => store.openFile(g.path)} oncontextmenu={(e) => fileMenu(e, g.path)} title={g.path}>
              <span class="mono ext">{ext(g.path) || "·"}</span>
              <span class="name">{g.path}</span>
              <span class="mono size">{g.lines.length}</span>
            </button>
            {#each g.lines as f (f.line)}
              <button class="hit" onclick={() => store.openFile(f.path, f.line)} oncontextmenu={(e) => fileMenu(e, f.path)} title={`${f.path}:${f.line}`}>
                <span class="mono ln">{f.line}</span>
                <span class="mono htext">{@html marked(f.text)}</span>
              </button>
            {/each}
          {/each}
          {#if foundCut}<div class="mono empty">{t("ws.searchCut")}</div>{/if}
        {:else}
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
                <button class="node file" class:on={store.activeFile === r.path && store.panelTab === "file"} style="padding-left: {filter.trim() ? 10 : 24 + r.depth * 14}px" onclick={() => store.openFile(r.path)} oncontextmenu={(e) => fileMenu(e, r.path)} title={r.path}>
                  <span class="mono ext">{ext(r.name) || "·"}</span>
                  <span class="name">{filter.trim() ? r.path : r.name}</span>
                  {#if !compact}<span class="mono size">{kb(r.size)}</span>{/if}
                </button>
              {/if}
            {/if}
          {/each}
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

<style>
  .fmenu {
    position: fixed;
    z-index: 60;
    min-width: 180px;
    padding: 4px 0;
    background: var(--card);
    border: 1px solid var(--lines);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.22);
  }

  .fitem {
    display: block;
    width: 100%;
    padding: 7px 14px;
    background: transparent;
    border: none;
    text-align: left;
    font-size: 12.5px;
    color: var(--txt);
    cursor: pointer;
  }

  .fitem:hover {
    background: var(--sel);
    color: var(--hi);
  }

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

  /* Open-file tabs scroll sideways (wheel or trackpad) but draw no
     scrollbar, which would push the strip off its line. */
  .filetabs {
    display: flex;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    border-left: 1px solid var(--line);
    margin-left: 4px;
  }

  .filetabs::-webkit-scrollbar {
    display: none;
  }

  .tab.file {
    flex: 0 0 auto;
    max-width: 180px;
    padding: 0 4px 0 10px;
    gap: 4px;
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
    position: relative;
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

  /* The tree over the file: a drawer from the right, the file under it. */
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 4;
    padding: 0;
    border: none;
    background: transparent;
    cursor: default;
  }

  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 5;
    width: min(420px, 68%);
    min-width: 220px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--rail);
    border-left: 1px solid var(--line);
    box-shadow: -10px 0 24px rgba(0, 0, 0, 0.18);
    animation: slidein 0.14s ease-out;
  }

  @keyframes slidein {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }

  .mdwrap {
    padding: 18px 22px 28px;
    font-size: var(--chat-fs);
    line-height: 1.7;
    color: var(--txt);
  }

  .previewnote {
    padding: 6px 12px;
    font-size: 11.5px;
    color: var(--dim);
    border-bottom: 1px solid var(--line);
  }

  .htmlframe {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: 0;
    /* Pages are written for a white page unless they say otherwise. */
    background: #fff;
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
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px 4px;
  }

  /* Files / changes, side by side in one frame. */
  .modes {
    display: flex;
    flex-shrink: 0;
    border: 1px solid var(--line);
  }

  .mode {
    width: 28px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: none;
    color: var(--dim);
    cursor: pointer;
  }

  .mode.on {
    color: var(--acc);
    background: var(--accbg);
  }

  .pm {
    font-size: 13px;
  }

  .treebar .refresh {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
  }

  /* Name / content, the filter's two ways. */
  .searchby {
    flex-shrink: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    margin: 4px 10px 4px;
    border: 1px solid var(--line);
  }

  .sb {
    height: 24px;
    padding: 0;
    background: transparent;
    border: none;
    font-size: 11.5px;
    color: var(--dim);
    cursor: pointer;
  }

  .sb.on {
    color: var(--hi);
    background: var(--sel);
  }

  .hit {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 2px 10px 2px 38px;
    background: transparent;
    border: none;
    text-align: left;
    cursor: pointer;
  }

  .hit:hover {
    background: var(--sel);
  }

  .ln {
    width: 30px;
    flex-shrink: 0;
    text-align: right;
    font-size: 10.5px;
    color: var(--lab);
  }

  .htext {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--dim);
  }

  .htext :global(mark) {
    background: var(--warnbg);
    color: var(--hi);
    outline: 1px solid var(--warnln);
  }

  .filter {
    flex: 1;
    min-width: 0;
    width: 100%;
    height: 28px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 11px;
    padding: 0 9px;
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

  .kbin {
    color: var(--acct);
  }

  .x.dirty .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--hi);
    display: block;
  }

  .x.dirty:hover .dot {
    background: var(--deltx);
  }

  .discard {
    background: var(--delbg);
    border: 0;
    color: var(--deltx);
    font-size: 9.5px;
    padding: 0 6px;
    height: 18px;
    align-self: center;
    margin-right: 4px;
  }

  .conflict {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    font-size: 12px;
    color: var(--warn);
    background: var(--warnbg);
    border-bottom: 1px solid var(--warnln);
  }
</style>
