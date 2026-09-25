<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { store, type ArtifactInfo, type WsEntry } from "./store.svelte";
  import AgentPicker from "./AgentPicker.svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  let draft = $state("");
  let contextOpen = $state(false);
  let box = $state<HTMLTextAreaElement>();
  /** Highlighted row in the slash list. */
  let slashIndex = $state(0);

  function submit(e?: Event) {
    e?.preventDefault();
    const text = draft;
    const something = text.trim() || store.attachments.length || (store.chatArtifact && store.designSelected.length);
    // A design picked a moment ago is still being written out: wait for it.
    if (!something || store.busy || store.attaching > 0) return;
    draft = "";
    store.send(text);
    queueMicrotask(grow);
  }

  /** The box grows with its text, up to eight lines; only past that does it scroll. */
  const MAX_BOX = 8 * 22 + 24;
  function grow() {
    if (!box) return;
    box.style.height = "auto";
    const wanted = box.scrollHeight;
    box.style.height = `${Math.min(wanted, MAX_BOX)}px`;
    box.style.overflowY = wanted > MAX_BOX ? "auto" : "hidden";
  }

  // ----- slash commands -----
  /** The word being typed after a leading "/", or null when not completing. */
  const slashQuery = $derived.by(() => {
    if (!draft.startsWith("/") || draft.includes("\n")) return null;
    const m = draft.match(/^\/([^\s]*)$/);
    return m ? m[1].toLowerCase() : null;
  });
  const slashMatches = $derived(
    slashQuery === null ? [] : store.slashCommands.filter((c) => c.name.toLowerCase().startsWith(slashQuery)),
  );
  const slashOpen = $derived(slashQuery !== null && slashMatches.length > 0);

  // Typing "/" is intent to talk to the conductor: with no list known yet,
  // open its session now so the commands (and the first reply) are ready.
  $effect(() => {
    if (store.chatArtifact) return;
    if (slashQuery !== null && store.slashCommands.length === 0 && !store.conductorState.open) void store.openConductor();
  });
  const slashWaiting = $derived(slashQuery !== null && slashMatches.length === 0 && store.conductorOpening === store.track);

  $effect(() => {
    void slashMatches.length;
    slashIndex = 0;
  });

  /** Put a command in the box; one without input goes straight out. */
  function complete(name: string) {
    const cmd = store.slashCommands.find((c) => c.name === name);
    draft = `/${name} `;
    if (cmd && !cmd.hint) {
      submit();
      return;
    }
    box?.focus();
    queueMicrotask(grow);
  }

  function onKey(e: KeyboardEvent) {
    if (atOpen && atItems.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        atIndex = (atIndex + 1) % atItems.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        atIndex = (atIndex - 1 + atItems.length) % atItems.length;
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        pickAt(atItems[atIndex]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        atHidden = true;
        return;
      }
    }
    if (slashOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        slashIndex = (slashIndex + 1) % slashMatches.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        slashIndex = (slashIndex - 1 + slashMatches.length) % slashMatches.length;
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        complete(slashMatches[slashIndex].name);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        draft = "";
        return;
      }
    }
    // Enter sends; Shift+Enter (and Ctrl/Alt+Enter) breaks the line.
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  }

  // ----- @ designs and files -----
  /** Where the caret is, so "@" completion reads the word being typed. */
  let caret = $state(0);
  let atIndex = $state(0);
  /** Escape closes the list until the next keystroke. */
  let atHidden = $state(false);
  let treeAsked = "";
  function trackCaret() {
    caret = box?.selectionStart ?? draft.length;
  }

  /** The "@word" right before the caret, or null. */
  const atToken = $derived.by(() => {
    // An artifact has no folder to search.
    if (store.chatArtifact) return null;
    const before = draft.slice(0, caret);
    const m = before.match(/(?:^|\s)@([^\s@]*)$/);
    return m ? { query: m[1].toLowerCase(), start: before.length - m[1].length - 1 } : null;
  });

  // The file list comes from the track's folder; fetch it the first time "@" is typed.
  $effect(() => {
    if (atToken && store.track && treeAsked !== store.track && !store.treeLoading) {
      treeAsked = store.track;
      void store.loadTree();
    }
  });

  /** Files whose name or path has the query; names that start with it first. */
  const atMatches = $derived.by((): WsEntry[] => {
    if (!atToken) return [];
    const q = atToken.query;
    const files = store.tree.filter((e) => !e.dir);
    if (!q) return files.slice(0, 30);
    const scored: [number, WsEntry][] = [];
    for (const e of files) {
      const name = e.name.toLowerCase();
      const path = e.path.toLowerCase();
      let score = -1;
      if (name.startsWith(q)) score = 0;
      else if (name.includes(q)) score = 1;
      else if (path.includes(q)) score = 2;
      if (score >= 0) scored.push([score, e]);
    }
    scored.sort((a, b) => a[0] - b[0] || a[1].path.length - b[1].path.length);
    return scored.slice(0, 30).map(([, e]) => e);
  });
  /** Designs to attach: the five most recent with nothing typed, else those
   *  whose name (first) or a tag has the query, most recent first. */
  const designMatches = $derived.by((): ArtifactInfo[] => {
    if (!atToken) return [];
    const q = atToken.query;
    if (!q) return store.designs.slice(0, 5);
    const scored: [number, ArtifactInfo][] = [];
    for (const d of store.designs) {
      const title = d.title.toLowerCase();
      let score = -1;
      if (title.startsWith(q)) score = 0;
      else if (title.includes(q)) score = 1;
      else if (d.tags.some((tag) => tag.toLowerCase().includes(q))) score = 2;
      if (score >= 0) scored.push([score, d]);
    }
    scored.sort((a, b) => a[0] - b[0] || b[1].updated_at - a[1].updated_at);
    return scored.slice(0, 8).map(([, d]) => d);
  });

  /** One list for the keyboard: designs first, then files. */
  type AtItem = { kind: "design"; design: ArtifactInfo } | { kind: "file"; file: WsEntry };
  const atItems = $derived<AtItem[]>([
    ...designMatches.map((design) => ({ kind: "design" as const, design })),
    ...atMatches.map((file) => ({ kind: "file" as const, file })),
  ]);
  const atOpen = $derived(atToken !== null && !atHidden && (atItems.length > 0 || store.treeLoading));

  $effect(() => {
    void atItems.length;
    atIndex = 0;
  });

  function pickAt(item: AtItem) {
    if (item.kind === "design") pickDesign(item.design);
    else pickFile(item.file);
  }

  /** Put "@name" in place of what was typed and attach the design. */
  function pickDesign(d: ArtifactInfo) {
    if (!atToken) return;
    const before = draft.slice(0, atToken.start);
    const after = draft.slice(caret);
    const inserted = `@${d.title} `;
    draft = before + inserted + after.replace(/^\s+/, "");
    const at = before.length + inserted.length;
    void attachDesign(d.id);
    queueMicrotask(() => {
      box?.focus();
      box?.setSelectionRange(at, at);
      caret = at;
      grow();
    });
  }

  /** Put "@path" in place of what was typed and attach the file. */
  function pickFile(entry: WsEntry) {
    if (!atToken) return;
    const before = draft.slice(0, atToken.start);
    const after = draft.slice(caret);
    const inserted = `@${entry.path} `;
    draft = before + inserted + after.replace(/^\s+/, "");
    const at = before.length + inserted.length;
    void store.attach([store.absoluteInTrack(entry.path)]);
    queueMicrotask(() => {
      box?.focus();
      box?.setSelectionRange(at, at);
      caret = at;
      grow();
    });
  }

  function preview(entry: WsEntry, e: MouseEvent) {
    e.stopPropagation();
    void store.openFile(entry.path);
  }

  function size(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1048576).toFixed(1)} MB`;
  }

  // ----- the + menu -----
  let menuOpen = $state(false);
  function insertAtCaret(ch: string) {
    menuOpen = false;
    const at = box?.selectionStart ?? draft.length;
    const lead = at > 0 && !/\s$/.test(draft.slice(0, at)) ? " " : "";
    draft = draft.slice(0, at) + lead + ch + draft.slice(at);
    const next = at + lead.length + ch.length;
    queueMicrotask(() => {
      box?.focus();
      box?.setSelectionRange(next, next);
      caret = next;
      grow();
    });
  }
  async function attachDesign(id: string) {
    menuOpen = false;
    await store.attachDesign(id, {
      goal: t("design.tag.goal"),
      constraint: t("design.tag.constraint"),
      question: t("design.tag.question"),
      idea: t("design.tag.idea"),
      note: t("design.notes"),
      questions: t("design.brief.questions"),
      answer: t("design.brief.answer"),
      unanswered: t("design.brief.unanswered"),
      links: t("design.brief.links"),
      files: t("design.brief.files"),
      frames: t("design.brief.frames"),
    });
    box?.focus();
  }

  async function upload() {
    menuOpen = false;
    await store.pickAttachments();
    box?.focus();
  }

  // ----- paste and drop -----
  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])];
    if (!files.length) return;
    e.preventDefault();
    for (const f of files) {
      const ext = f.type.split("/")[1]?.replace("jpeg", "jpg") ?? "bin";
      void store.attachBlob(f, f.name || `pasted.${ext}`);
    }
  }

  /** The window tells us where files dropped from the system land. */
  let dropping = $state(false);
  onMount(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        if (store.view !== "track" && store.view !== "design") return;
        const p = e.payload;
        // Over a design's board, the board takes it.
        if ("position" in p && store.overDesignBoard(p.position)) {
          dropping = false;
          return;
        }
        if (p.type === "enter" || p.type === "over") dropping = true;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          void store.attach(p.paths);
          box?.focus();
        }
      })
      .then((u) => (unlisten = u))
      .catch(() => {});
    return () => unlisten?.();
  });

  const ctx = $derived(store.context);
  const pct = $derived(ctx && ctx.size > 0 ? Math.min(100, (ctx.used / ctx.size) * 100) : 0);

  function k(n: number): string {
    return n >= 1000 ? `${(n / 1000).toFixed(n >= 100000 ? 0 : 1)}k` : String(n);
  }

  function shortPath(p: string | undefined): string {
    if (!p) return "";
    const parts = p.split(/[\\/]/).filter(Boolean);
    if (parts.length <= 3) return p;
    const sep = p.includes("\\") ? "\\" : "/";
    return `…${sep}${parts.slice(-2).join(sep)}`;
  }

</script>

<div class="composer" class:dropping>
  {#if store.attachments.length}
    <!-- What goes with the next message. -->
    <div class="attached" aria-label={t("composer.attached")}>
      {#each store.attachments as a (a.path)}
        <span class="file" title={a.path}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" /><path d="M9 1.5v3h3" /></svg>
          <span class="fname">{a.name}</span>
          <span class="mono fsize">{size(a.size)}</span>
          <button type="button" class="fx" onclick={() => store.detach(a.path)} aria-label={t("composer.detach")} title={t("composer.detach")}><Icon name="close" size={10} /></button>
        </span>
      {/each}
    </div>
  {/if}
  <form onsubmit={submit}>
    <!-- Attach: files, and the two ways to point at things in the box. -->
    <Popover bind:open={menuOpen} width={300}>
      {#snippet trigger()}
        <button type="button" class="btn plus" onclick={() => (menuOpen = !menuOpen)} aria-label={t("composer.add")} title={t("composer.add")} aria-haspopup="menu" aria-expanded={menuOpen}>
          <svg width="14" height="14" viewBox="0 0 14 14" stroke="currentColor" stroke-width="1.3" aria-hidden="true"><path d="M7 1v12M1 7h12" /></svg>
        </button>
      {/snippet}
      <div class="addmenu" role="menu">
        <button type="button" class="tile" role="menuitem" onclick={upload}>
          <svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.1" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" /><path d="M9 1.5v3h3M6 9h4M6 11.5h4" /></svg>
          <span>{t("composer.upload")}</span>
        </button>
        {#if !store.chatArtifact}
          <!-- A design goes to the conductor as its brief and, with ink, a picture of its board. -->
          <div class="rule"></div>
          <div class="mlab-sm mhead">{t("composer.attachDesign")}</div>
          {#each store.designs.slice(0, 5) as d (d.id)}
            <button type="button" class="mitem design" role="menuitem" onclick={() => attachDesign(d.id)} style="--bar: {d.color || 'transparent'}">
              <span class="mono mkey">▦</span>
              <span class="mbody"><span class="mtitle">{d.title}</span>{#if d.tags.length}<span class="mdesc">{d.tags.join(" · ")}</span>{/if}</span>
            </button>
          {:else}
            <div class="mono mnone">{t("composer.noDesigns")}</div>
          {/each}
          {#if store.designs.length > 5}
            <button type="button" class="mitem" role="menuitem" onclick={() => insertAtCaret("@")}>
              <span class="mono mkey">@</span>
              <span class="mbody"><span class="mtitle">{t("composer.allDesigns", { n: store.designs.length })}</span><span class="mdesc">{t("composer.allDesignsDesc")}</span></span>
            </button>
          {/if}
        {/if}
        <div class="rule"></div>
        <button type="button" class="mitem" role="menuitem" onclick={() => insertAtCaret("/")}>
          <span class="mono mkey">/</span>
          <span class="mbody"><span class="mtitle">{t("composer.menuCommand")}</span><span class="mdesc">{t("composer.menuCommandDesc")}</span></span>
        </button>
        <button type="button" class="mitem" role="menuitem" onclick={() => insertAtCaret("@")}>
          <span class="mono mkey">@</span>
          <span class="mbody"><span class="mtitle">{t("composer.menuFile")}</span><span class="mdesc">{t("composer.menuFileDesc")}</span></span>
        </button>
      </div>
    </Popover>
    <div class="boxwrap">
      {#if atOpen}
        <!-- Designs (most recent first), then files in the track's folder, narrowed by what follows the "@". -->
        <div class="slash" role="listbox" aria-label={t("composer.files")}>
          {#if designMatches.length}
            <div class="mlab-sm ph">{t("composer.designs")}</div>
            {#each designMatches as d, i (d.id)}
              <div
                class="frow"
                class:on={i === atIndex}
                role="option"
                aria-selected={i === atIndex}
                tabindex="-1"
                style="box-shadow: inset 2px 0 0 {d.color || 'transparent'}"
                onmouseenter={() => (atIndex = i)}
                onmousedown={(e) => e.preventDefault()}
                onclick={() => pickDesign(d)}
                onkeydown={() => {}}
              >
                <span class="ficon mono">▦</span>
                <span class="fcol">
                  <span class="fn">{d.title}</span>
                  {#if d.tags.length}<span class="mono fp">{d.tags.join(" · ")}</span>{/if}
                </span>
                <span class="mono fs">{whenLabel(d.updated_at, store.now, store.lang)}</span>
              </div>
            {/each}
          {/if}
          <div class="mlab-sm ph">{t("composer.files")}</div>
          {#if atMatches.length === 0}
            <div class="mono waiting"><span class="dot pulse"></span>{store.treeLoading ? t("composer.filesLoading") : t("composer.noFiles")}</div>
          {/if}
          {#each atMatches as f, j (f.path)}
            {@const i = designMatches.length + j}
            <div
              class="frow"
              class:on={i === atIndex}
              role="option"
              aria-selected={i === atIndex}
              tabindex="-1"
              onmouseenter={() => (atIndex = i)}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => pickFile(f)}
              onkeydown={() => {}}
            >
              <svg class="ficon" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.1" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" /><path d="M9 1.5v3h3" /></svg>
              <span class="fcol">
                <span class="mono fn">{f.name}</span>
                <span class="mono fp">{f.path}</span>
              </span>
              <span class="mono fs">{size(f.size)}</span>
              <button type="button" class="eye" onclick={(e) => preview(f, e)} aria-label={t("composer.preview")} title={t("composer.preview")}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.1" aria-hidden="true"><path d="M1 8s2.5-5 7-5 7 5 7 5-2.5 5-7 5-7-5-7-5z" /><circle cx="8" cy="8" r="2" /></svg>
              </button>
            </div>
          {/each}
        </div>
      {:else if slashWaiting}
        <div class="slash" role="status">
          <div class="mono waiting"><span class="dot pulse"></span>{t("composer.opening")}</div>
        </div>
      {:else if slashOpen}
        <!-- The agent's slash commands, narrowed by what follows the "/". -->
        <div class="slash" role="listbox" aria-label={t("composer.commands")}>
          <div class="mlab-sm ph">{t("composer.commands")}</div>
          {#each slashMatches as c, i (c.name)}
            <button
              type="button"
              class="cmd"
              class:on={i === slashIndex}
              role="option"
              aria-selected={i === slashIndex}
              onmouseenter={() => (slashIndex = i)}
              onclick={() => complete(c.name)}
            >
              <span class="mono cname">/{c.name}</span>
              {#if c.hint}<span class="mono chint">{c.hint}</span>{/if}
              <span class="cdesc">{c.description}</span>
            </button>
          {/each}
        </div>
      {/if}
      <textarea
        bind:this={box}
        bind:value={draft}
        rows="1"
        disabled={store.busy}
        placeholder={store.busy ? t("composer.busy") : store.chatArtifact ? t("design.placeholder") : t("composer.placeholder")}
        aria-label={t("composer.placeholder")}
        onkeydown={onKey}
        oninput={() => {
          grow();
          trackCaret();
          atHidden = false;
        }}
        onkeyup={trackCaret}
        onclick={trackCaret}
        onpaste={onPaste}
      ></textarea>
    </div>
    {#if store.busy}
      <!-- While the conductor answers, the send button is a stop button: Ctrl+C. -->
      <button class="btn send stop" type="button" disabled={store.cancelling} onclick={() => store.cancelConductor()} title={t("composer.stopTitle")} aria-label={t("composer.stop")}>■</button>
    {:else}
      <button class="btn send" type="submit" disabled={(!draft.trim() && !store.attachments.length) || store.attaching > 0} aria-label={t("composer.send")}>→</button>
    {/if}
  </form>

  <div class="status">
    <AgentPicker />

    {#if store.chatArtifact}
      {#if store.designSelected.length}
        <span class="chip static mono">{t("design.withSelected", { n: store.designSelected.length })}</span>
      {/if}
    {:else}
      <span class="chip static" title={store.currentTrack?.cwd}>
        <Icon name="folder" size={14} />
        <span class="mono path">{shortPath(store.currentTrack?.cwd)}</span>
      </span>
    {/if}

    <span class="grow"></span>

    {#if store.lastError}<span class="mono err" title={store.lastError}>{store.lastError}</span>{/if}

    <!-- context -->
    <Popover bind:open={contextOpen} align="right" width={280}>
      {#snippet trigger()}
        <button class="ctx" onclick={() => (contextOpen = !contextOpen)} title={t("composer.contextTitle")} aria-expanded={contextOpen}>
          <span class="bar"><span class="fill" style="width: {pct}%"></span></span>
        </button>
      {/snippet}
      <div class="mlab ph">{t("composer.context")}</div>
      <div class="kv mono">
        {#if ctx}
          <div class="row"><span class="dim">{t("composer.used")}</span><span>{t("composer.tokens", { used: k(ctx.used), size: k(ctx.size) })}</span></div>
          <div class="row"><span class="dim">{t("composer.ratio")}</span><span>{pct.toFixed(1)}%</span></div>
          {#if ctx.cost !== undefined && ctx.cost !== null}
            <div class="row"><span class="dim">{t("composer.cost")}</span><span>{ctx.cost.toFixed(4)} {ctx.currency ?? ""}</span></div>
          {/if}
          <div class="row"><span class="dim">{t("composer.source")}</span><span>{store.activeRun ? t("composer.liveRun") : t("composer.lastRun")}</span></div>
        {:else}
          <div class="row dim">{t("composer.noRuns")}</div>
        {/if}
      </div>
    </Popover>
  </div>
</div>

<style>
  .composer {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--rail);
    padding: 16px 34px 10px;
    outline: 1px dashed transparent;
    outline-offset: -6px;
  }

  /* Files dragged over the window land here. */
  .composer.dropping {
    outline-color: var(--acc);
    background: var(--accbg);
  }

  .attached {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0 0 8px 53px;
  }

  .file {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    padding: 0 4px 0 9px;
    border: 1px solid var(--lines);
    background: var(--inp);
    color: var(--dim);
    font-size: 12px;
    max-width: 280px;
  }

  .fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--txt);
  }

  .fsize {
    font-size: 10px;
    color: var(--lab);
    flex-shrink: 0;
  }

  .fx {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .fx:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .plus {
    width: 44px;
    height: 44px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .addmenu {
    display: flex;
    flex-direction: column;
    padding: 8px;
  }

  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 14px 10px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--txt);
    font-size: 12px;
  }

  .tile:hover,
  .mitem:hover {
    background: var(--sel);
    color: var(--hi);
  }

  .rule {
    height: 1px;
    background: var(--line);
    margin: 8px 0;
  }

  .mitem {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 8px 8px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--txt);
  }

  .mhead {
    padding: 2px 8px 6px;
  }

  .mitem.design {
    box-shadow: inset 2px 0 0 var(--bar);
  }

  .mnone {
    padding: 4px 8px 8px;
    font-size: 11px;
    color: var(--lab);
  }

  .mkey {
    width: 16px;
    flex-shrink: 0;
    font-size: 14px;
    color: var(--lab);
    text-align: center;
  }

  .mbody {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .mtitle {
    font-size: 13px;
  }

  .mdesc {
    font-size: 11px;
    color: var(--lab);
  }

  /* One file in the "@" list: name, path, size, and a look at it. */
  .frow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px 7px 14px;
    border-left: 2px solid transparent;
    color: var(--dim);
    cursor: pointer;
  }

  .frow.on {
    color: var(--hi);
    background: var(--sel);
    border-left-color: var(--acc);
  }

  .ficon {
    flex-shrink: 0;
    color: var(--lab);
  }

  .fcol {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .fn {
    font-size: 12px;
    font-weight: 600;
    color: inherit;
  }

  .fp {
    font-size: 10px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fs {
    font-size: 10px;
    color: var(--lab);
    flex-shrink: 0;
  }

  .eye {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .eye:hover {
    color: var(--hi);
    background: var(--card);
  }

  form {
    display: flex;
    align-items: flex-end;
    gap: 9px;
  }

  .boxwrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  textarea {
    flex: 1;
    field-sizing: content;
    min-height: 44px;
    max-height: 200px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 14px;
    line-height: 22px;
    padding: 11px 14px;
    outline: none;
    resize: none;
    overflow-y: hidden;
  }

  textarea:focus {
    border-color: var(--acc);
  }

  /* The slash list, above the box. */
  .slash {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 30;
    max-height: 280px;
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--lines);
    padding-bottom: 4px;
  }

  .slash .ph {
    padding: 10px 14px 6px;
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    font-size: 11px;
    color: var(--lab);
  }

  .waiting .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  .cmd {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 7px 14px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .cmd.on {
    color: var(--hi);
    background: var(--sel);
    border-left-color: var(--acc);
  }

  .cname {
    font-size: 12px;
    color: inherit;
    flex-shrink: 0;
  }

  .chint {
    font-size: 10px;
    color: var(--lab);
    flex-shrink: 0;
  }

  .cdesc {
    font-size: 11px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }


  .send {
    width: 44px;
    height: 44px;
    padding: 0;
    font-size: 15px;
    letter-spacing: 0;
    border-color: var(--accln);
    color: var(--acct);
  }

  .send.stop {
    font-size: 11px;
    border-color: var(--acct);
  }

  /* One row, always: chips shrink and cut before anything wraps. No
     overflow clipping here: the popovers open upward out of this row. */
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 32px;
    margin-top: 6px;
    min-width: 0;
    white-space: nowrap;
  }

  .chip {
    height: 26px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0 8px;
    background: transparent;
    border: 0;
    color: var(--dim);
    font-size: 10px;
    letter-spacing: 0.12em;
  }

  .chip:not(.static):hover {
    color: var(--hi);
    background: var(--sel);
  }

  .chip.static {
    color: var(--lab);
    min-width: 0;
    flex-shrink: 1;
  }

  .path {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .err {
    font-size: 10px;
    color: var(--acct);
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Context: a hairline track, accent fill; no numbers until asked. */
  .ctx {
    height: 26px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    background: transparent;
    border: 0;
  }

  .ctx:hover {
    background: var(--sel);
  }

  .bar {
    width: 72px;
    height: 3px;
    background: var(--lines);
    display: block;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--acc);
    transition: width 200ms linear;
  }

  .ph {
    padding: 12px 14px 8px;
  }

  .kv {
    display: flex;
    flex-direction: column;
    padding: 0 14px 12px;
    gap: 6px;
    font-size: 11px;
    color: var(--txt);
  }

  .row {
    display: flex;
    gap: 12px;
  }

  .row .dim {
    color: var(--lab);
    width: 40px;
    flex-shrink: 0;
    font-size: 11px;
  }
</style>
