<script lang="ts">
  import { onDestroy } from "svelte";
  import { store } from "./store.svelte";
  import { kb, type KItem, type KSource, type KTab } from "./knowledge.svelte";
  import KnowledgeGraph from "./KnowledgeGraph.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import Icon from "./Icon.svelte";
  import { whenFull, whenLabel } from "./time";
  import { t } from "./i18n.svelte";
  import { docTrouble, indexProgress } from "./indexProgress";
  import { tick } from "svelte";

  /**
   * The knowledge library, after Kiro Crew's: the items agents can search
   * (listed per document, or found by a search), the entity graph, the
   * documents themselves with their sync state, and how documents are
   * described.
   */
  const tabs = $derived<{ id: KTab; label: string }[]>([
    { id: "list", label: t("kb.tab.list") },
    { id: "graph", label: t("kb.tab.graph") },
    { id: "sources", label: t("kb.tab.sources") },
  ]);

  let folded = $state<Record<string, boolean>>({});
  let open = $state<Record<number, boolean>>({});
  let copied = $state(0);
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  let confirmRemove = $state("");

  // The graph's conversation belongs to this page: leaving it closes the panel.
  onDestroy(() => store.closeGraphChat());

  const categories = $derived([...new Set(kb.items.map((i) => i.category))].sort());
  const shown = $derived(kb.category ? kb.items.filter((i) => i.category === kb.category) : kb.items);
  /** Items grouped by document, in the order they come (a search keeps its ranking). */
  const groups = $derived.by(() => {
    const out: { source: KSource | undefined; id: string; items: KItem[] }[] = [];
    for (const item of shown) {
      let g = out.find((x) => x.id === item.source_id);
      if (!g) {
        g = { id: item.source_id, source: kb.sources.find((s) => s.id === item.source_id), items: [] };
        out.push(g);
      }
      g.items.push(item);
    }
    return out;
  });

  const current = $derived(kb.indexing.find((s) => s.status === "indexing"));
  /** Indexing under way, for the small mark on the List and Graph tabs; null when done. */
  const progress = $derived(indexProgress(kb.sources, kb.embedding));
  /** Documents that are gone or failed to index, for the same small line. */
  const trouble = $derived(docTrouble(kb.sources));
  /** A source just brought into view from that line, outlined for a moment. */
  let flashed = $state("");

  /** Go to the Sources tab and bring the first document that needs attention into view. */
  async function showTrouble() {
    const id = trouble?.first;
    await kb.setTab("sources");
    if (!id) return;
    await tick();
    document.getElementById(`kb-source-${id}`)?.scrollIntoView({ block: "center", behavior: "smooth" });
    flashed = id;
    setTimeout(() => {
      if (flashed === id) flashed = "";
    }, 1800);
  }

  function name(s: KSource | undefined, id: string): string {
    return s ? kb.nameOf(s) : id;
  }

  function preview(item: KItem): string {
    const text = item.summary || item.content.replace(/\s+/g, " ");
    return text.length > 260 ? `${text.slice(0, 260)}…` : text;
  }

  async function copy(item: KItem) {
    await navigator.clipboard.writeText(item.content);
    copied = item.id;
    setTimeout(() => {
      if (copied === item.id) copied = 0;
    }, 1200);
  }

  function statusLabel(s: KSource): string {
    switch (s.status) {
      case "indexing":
        return t("kb.status.indexing", { done: s.done, total: s.total });
      case "pending":
        return t("kb.status.pending");
      case "synced":
        return t("kb.status.synced");
      case "missing":
        return t("kb.status.missing");
      case "duplicate":
        return t("kb.status.duplicate");
      default:
        return t("kb.status.error");
    }
  }

  const menuSource = $derived(menu ? kb.sources.find((s) => s.id === menu!.id) : undefined);
  const menuArtifact = $derived(menu ? kb.artifactOf(menu.id) : undefined);
</script>

<section class="page">
  <!-- One thin bar: the page's name (what it is for, on hover), its tabs, what
       the index is doing when there is something to say, and the page's buttons. -->
  <header class="kbar">
    <h1 class="serif kt" title={t("kb.sub")}>{t("kb.title")}</h1>
    <div class="tabs" role="tablist">
      {#each tabs as tab (tab.id)}
        <button role="tab" aria-selected={kb.tab === tab.id} class:on={kb.tab === tab.id} onclick={() => kb.setTab(tab.id)}>{tab.label}</button>
      {/each}
    </div>
    {#if kb.tab !== "sources" && (progress || trouble)}
      <!-- On List and Graph, only when there is something to say: indexing under way, and
           documents that need the person (gone, or failed), the latter a link to them on Sources.
           The Sources tab has the full line instead. -->
      <div class="indexing mono" role="status">
        {#if progress}
          <span class="pulse"></span>
          <span class="itext">{t("kb.indexingSmall", { done: progress.done, total: progress.total })}</span>
        {/if}
        {#if progress && trouble}<span class="sep" aria-hidden="true">·</span>{/if}
        {#if trouble}
          <button class="trouble" onclick={showTrouble} title={t("kb.troubleHint")}>
            <span class="tdot" aria-hidden="true"></span><span class="itext">{t("kb.troubleSmall", { n: trouble.count })}</span>
          </button>
        {/if}
      </div>
    {/if}
    <div class="kbtns">
      <!-- The graph's agent opens from any tab; the graph's own Agent button shows the same state. -->
      <button
        class="btn sm agent"
        class:on={store.graphChatOpen}
        aria-pressed={store.graphChatOpen}
        title={store.graphChatOpen ? t("kb.agentClose") : t("kb.agentOpen")}
        onclick={() => (store.graphChatOpen ? store.closeGraphChat() : void store.openGraphChat())}>{t("kb.agentToggle")}</button
      >
      <button class="btn sm" onclick={() => store.openSettings("knowledge")}>{t("kb.openSettings")}</button>
    </div>
  </header>
  <div class="inner">
    {#if kb.embedding.enabled && kb.embedding.error}
      <div class="banner err">{t("kb.embedError", { done: kb.embedding.embedded, total: kb.embedding.total, error: kb.embedding.error })}</div>
    {/if}
    <!-- Search readiness, embedding and sync: about the documents, so on the Sources tab only,
         under what the library is for (the sentence the old header carried). -->
    {#if kb.tab === "sources"}
    <p class="sub">{t("kb.sub")}</p>
    <div class="banner" class:busy={!!kb.indexing.length}>
      {#if current}
        <span class="pulse"></span>
        <span>{t("kb.indexingNow", { name: kb.nameOf(current), done: current.done, total: current.total })}</span>
        {#if kb.indexing.length > 1}<span class="dim">· {t("kb.queued", { n: kb.indexing.length - 1 })}</span>{/if}
      {:else if kb.indexing.length}
        <span class="pulse"></span><span>{t("kb.queued", { n: kb.indexing.length })}</span>
      {:else}
        <span class="ok">✓</span>
        {#if kb.embedding.enabled}
          <span>{t("kb.smartOn")}</span>
          <span class="dim">· {t("kb.embedded", { done: kb.embedding.embedded, total: kb.embedding.total, pct: kb.embedding.total ? Math.round((kb.embedding.embedded / kb.embedding.total) * 100) : 100 })}</span>
        {:else}
          <span>{t("kb.searchReady")}</span>
          <span class="dim">· {t("kb.keywordOnly")}</span>
        {/if}
        <span class="dim">· {t("kb.syncedCount", { done: kb.syncedCount, total: kb.sources.length })}</span>
      {/if}
    </div>
    {/if}

    {#if kb.tab === "list"}
      <div class="filters">
        <input
          class="search"
          placeholder={t("kb.searchPlaceholder")}
          bind:value={kb.query}
          onkeydown={(e) => {
            // The graph tab's search box edits the same query.
            if (e.key === "Enter") {
              void kb.search(kb.query);
            } else if (e.key === "Escape" && kb.query) {
              kb.query = "";
              void kb.search("");
            }
          }}
        />
        <select bind:value={kb.category} aria-label={t("kb.allTypes")}>
          <option value="">{t("kb.allTypes")}</option>
          {#each categories as c (c)}<option value={c}>{c.replace(/_/g, " ")}</option>{/each}
        </select>
        <select bind:value={kb.sourceFilter} onchange={() => kb.search(kb.shownQuery)} aria-label={t("kb.allSources")}>
          <option value="">{t("kb.allSources")}</option>
          {#each kb.sources as s (s.id)}<option value={s.id}>{kb.nameOf(s)}</option>{/each}
        </select>
      </div>

      {#if kb.shownQuery}
        <p class="note">{t("kb.results", { n: shown.length, q: kb.shownQuery })}</p>
      {/if}

      {#if !kb.sources.length}
        <div class="emptybox">
          <button class="btn btn-acc" onclick={() => kb.pickAndAdd()}>{t("kb.addSource")}</button>
        </div>
      {:else if !shown.length && !kb.loading}
        <p class="none">{kb.shownQuery ? t("kb.noResults") : t("kb.noItems")}</p>
      {/if}

      {#each groups as g (g.id)}
        <div class="group">
          <button class="ghead" onclick={() => (folded[g.id] = !folded[g.id])} aria-expanded={!folded[g.id]}>
            <span class="chev" class:shut={folded[g.id]}>▾</span>
            <Icon name="book" size={14} />
            <span class="gname">{name(g.source, g.id)}</span>
            <span class="mono badge">{g.items.length}</span>
            {#if g.source?.topic}<span class="gtopic">{g.source.topic}</span>{/if}
            <span class="mono gpath" title={g.source?.uri}>{g.source?.uri}</span>
          </button>
          {#if !folded[g.id]}
            {#each g.items as item (item.id)}
              <article class="card">
                <div class="ctop">
                  <button class="ctitle" onclick={() => (open[item.id] = !open[item.id])}>{item.title || t("kb.untitled")}</button>
                  <span class="mono cat">{item.category.replace(/_/g, " ")}</span>
                </div>
                {#if open[item.id]}
                  <pre class="content">{item.content}</pre>
                {:else}
                  <p class="summary">{preview(item)}</p>
                {/if}
                <div class="cfoot">
                  <span class="mono meta" title={whenFull(item.created_at, store.lang)}>{whenLabel(item.created_at, store.now, store.lang)}</span>
                  {#if item.section}<span class="mono meta">§ {item.section}</span>{/if}
                  <span class="mono meta">L{item.line_start}-{item.line_end}</span>
                  {#each item.tags as tag (tag)}<span class="mono meta">#{tag}</span>{/each}
                  {#if item.match_type}<span class="mono meta match">{item.match_type} · {item.score?.toFixed(3)}</span>{/if}
                  <span class="grow"></span>
                  <button class="btn sm" onclick={() => copy(item)}>{copied === item.id ? t("kb.copied") : t("kb.copy")}</button>
                </div>
              </article>
            {/each}
          {/if}
        </div>
      {/each}
    {:else if kb.tab === "graph"}
      <KnowledgeGraph />
    {:else if kb.tab === "sources"}
      <div class="actions">
        <span class="grow"></span>
        <button class="btn btn-acc" onclick={() => kb.pickAndAdd()}>{t("kb.addSource")}</button>
      </div>
      {#each kb.sources as s (s.id)}
        {@const art = kb.artifactOf(s.id)}
        <div
          class="source"
          class:flash={flashed === s.id}
          id="kb-source-{s.id}"
          style="border-left-color: {art?.color || 'transparent'}"
          oncontextmenu={(e) => {
            e.preventDefault();
            menu = { id: s.id, x: e.clientX, y: e.clientY };
          }}
          role="presentation"
        >
          <div class="sbody">
            <div class="sname">{kb.nameOf(s)}</div>
            <div class="mono spath">{s.source_type} · {s.uri}</div>
            {#if s.topic}<div class="stopic">{s.topic}</div>{/if}
            {#if s.error}<div class="serr" class:warn={s.status === "synced"}>{s.error}</div>{/if}
            <div class="chips">
              {#if kb.overlaps[s.id]}
                {@const o = kb.overlaps[s.id]}
                {@const other = kb.sources.find((x) => x.id === o.other)}
                <span class="overlap" title={t("kb.overlapTitle", { n: o.shared })}>{t("kb.overlap", { pct: o.percent, name: other ? kb.nameOf(other) : o.other })}</span>
              {/if}
              {#each s.themes as theme (theme)}<span class="theme">{theme}</span>{/each}
              {#each art?.tags ?? [] as tag (tag)}<span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>{/each}
            </div>
          </div>
          <span class="pill {s.status}">{statusLabel(s)}</span>
          <span class="mono meta">{t("kb.itemCount", { n: s.items })}</span>
          <span class="mono meta" title={s.last_synced ? whenFull(s.last_synced, store.lang) : ""}>{s.last_synced ? whenLabel(s.last_synced, store.now, store.lang) : "—"}</span>
          <button class="btn sm" disabled={s.status === "indexing" || s.status === "pending"} onclick={() => kb.sync(s.id)}>{t("kb.sync")}</button>
          {#if confirmRemove === s.id}
            <button class="btn sm danger" onclick={async () => { confirmRemove = ""; const why = await kb.remove(s.id); if (why) store.lastError = why; }}>{t("kb.removeConfirm")}</button>
          {:else}
            <button class="btn sm x" title={t("kb.remove")} aria-label={t("kb.remove")} onclick={() => (confirmRemove = s.id)}>×</button>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <footer class="mono">
    <span>{t("kb.stat.items", { n: kb.stats.items })}</span>
    <span>{t("kb.stat.entities", { n: kb.stats.entities })}</span>
    <span>{t("kb.stat.relations", { n: kb.stats.relations })}</span>
    <span>{t("kb.stat.sources", { n: kb.stats.sources })}</span>
  </footer>
</section>

{#if menu && menuSource}
  <ArtifactMenu
    x={menu.x}
    y={menu.y}
    name={menuArtifact?.title ?? kb.nameOf(menuSource)}
    color={menuArtifact?.color ?? ""}
    deleteNote={t("kb.deleteNote")}
    onclose={() => (menu = null)}
    onrename={(title) => store.updateArtifact(menuSource.id, { title })}
    ontags={() => (store.tagDialog = menuSource.id)}
    oncolor={(color) => store.updateArtifact(menuSource.id, { color })}
    ondelete={() => kb.remove(menuSource.id)}
  />
{/if}

<style>
  .page {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  .inner {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 20px 40px 40px;
    width: 100%;
    box-sizing: border-box;
  }

  /* The page's one bar: as thin as the other pages' heads (44 px), the tabs in it. */
  .kbar {
    flex-shrink: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: stretch;
    column-gap: 18px;
    min-height: 44px;
    padding: 0 24px 0 40px;
    border-bottom: 1px solid var(--line);
  }

  .kbar > * {
    min-height: 44px;
  }

  .kt {
    display: flex;
    align-items: center;
    margin: 0;
    font-size: 18px;
    font-weight: 400;
    color: var(--hi);
    white-space: nowrap;
    cursor: default;
  }

  .tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
  }

  .tabs button {
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    padding: 0 12px;
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
    margin-bottom: -1px;
  }

  .tabs button:hover {
    color: var(--hi);
  }

  .tabs button.on {
    color: var(--hi);
    border-bottom-color: var(--acc);
  }

  /* Right-hand, on the first row or, in a narrow window, the next. */
  .kbtns {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }

  .sub {
    margin: 0 0 12px;
    color: var(--dim);
    font-size: 13px;
  }

  .btn.agent.on {
    color: var(--hi);
    background: var(--sel);
  }

  /* The List and Graph tabs' only status, in the bar: indexing, documents to look at. */
  .indexing {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    font-size: 10px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--lab);
  }

  .indexing .itext {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .indexing .sep {
    color: var(--lines);
  }

  /* Documents that need the person: a quiet link to them on the Sources tab. */
  .trouble {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    background: transparent;
    border: 0;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    color: var(--deltx);
  }

  .trouble:hover {
    text-decoration: underline;
  }

  .tdot {
    width: 6px;
    height: 6px;
    border: 1px solid var(--deltx);
    transform: rotate(45deg);
  }

  .source.flash {
    outline: 1px solid var(--deltx);
    outline-offset: 2px;
    transition: outline-color 0.3s;
  }

  .indexing .pulse {
    width: 6px;
    height: 6px;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    padding: 10px 14px;
    font-size: 13px;
    color: var(--txt);
    margin-bottom: 16px;
  }

  .banner.err {
    color: var(--deltx);
    background: var(--delbg);
    border-color: var(--deltx);
    word-break: break-word;
  }

  .banner .ok {
    color: var(--ok);
  }

  .dim {
    color: var(--lab);
  }

  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--acc);
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }

  .filters {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }

  .search {
    flex: 1;
    min-width: 0;
  }

  .search,
  select {
    height: 34px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    padding: 0 10px;
    font-size: 13px;
    font-family: inherit;
  }

  select {
    max-width: 240px;
  }

  .note {
    color: var(--dim);
    font-size: 12.5px;
    margin: 0 0 12px;
    line-height: 1.55;
  }

  .none {
    color: var(--lab);
    font-size: 13px;
  }

  .emptybox {
    border: 1px dashed var(--lines);
    padding: 28px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    color: var(--dim);
    font-size: 13px;
    text-align: center;
  }

  .group {
    border: 1px solid var(--line);
    margin-bottom: 12px;
  }

  .ghead {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 10px 12px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--txt);
    min-width: 0;
  }

  .ghead:hover {
    background: var(--sel);
  }

  .chev {
    color: var(--lab);
    transition: transform 0.15s;
    width: 10px;
  }

  .chev.shut {
    transform: rotate(-90deg);
  }

  .gname {
    color: var(--hi);
    font-size: 13.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 30%;
  }

  .badge {
    font-size: 10.5px;
    color: var(--oktx);
    background: var(--okbg);
    border: 1px solid var(--okln);
    padding: 0 7px;
    border-radius: 9px;
  }

  .gtopic {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .gpath {
    max-width: 26%;
    font-size: 10.5px;
    color: var(--lab);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }

  .card {
    margin: 0 12px 10px 34px;
    border: 1px solid var(--line);
    background: var(--card);
    padding: 12px 14px 10px;
  }

  .ctop {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }

  .ctitle {
    flex: 1;
    background: transparent;
    border: 0;
    padding: 0;
    text-align: left;
    color: var(--hi);
    font-size: 14.5px;
  }

  .ctitle:hover {
    text-decoration: underline;
  }

  .cat {
    font-size: 11px;
    color: var(--acct);
    background: var(--accbg);
    border: 1px solid var(--accln);
    padding: 1px 8px;
    border-radius: 10px;
    white-space: nowrap;
  }

  .summary {
    margin: 7px 0 8px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--txt);
  }

  .content {
    margin: 8px 0;
    padding: 10px;
    max-height: 420px;
    overflow: auto;
    background: var(--bg);
    border: 1px solid var(--line);
    font-family: var(--mono);
    font-size: 11.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    color: var(--txt);
  }

  .cfoot {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .meta {
    font-size: 10.5px;
    color: var(--lab);
  }

  .match {
    color: var(--acct);
  }

  .grow {
    flex: 1;
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  .btn.danger {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .btn.x {
    font-size: 14px;
    color: var(--deltx);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
  }

  .source {
    display: flex;
    align-items: center;
    gap: 14px;
    border: 1px solid var(--line);
    border-left: 3px solid transparent;
    padding: 12px 14px;
    margin-bottom: 10px;
  }

  .sbody {
    flex: 1;
    min-width: 0;
  }

  .sname {
    color: var(--hi);
    font-size: 14px;
  }

  .spath {
    font-size: 10.5px;
    color: var(--lab);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stopic {
    font-size: 12.5px;
    color: var(--txt);
    margin-top: 6px;
  }

  .serr {
    font-size: 12px;
    color: var(--deltx);
    margin-top: 5px;
  }

  .serr.warn {
    color: var(--warn);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 7px;
  }

  .overlap {
    font-size: 11px;
    color: var(--warn);
    border: 1px solid var(--warnln);
    background: var(--warnbg);
    padding: 1px 7px;
  }

  .theme {
    font-size: 11px;
    color: var(--dim);
    border: 1px solid var(--lines);
    padding: 1px 7px;
  }

  .chip {
    font-size: 10.5px;
  }

  .pill {
    font-family: var(--mono);
    font-size: 11px;
    padding: 2px 10px;
    border-radius: 11px;
    border: 1px solid var(--lines);
    color: var(--dim);
    white-space: nowrap;
  }

  .pill.synced {
    color: var(--oktx);
    background: var(--okbg);
    border-color: var(--okln);
  }

  .pill.indexing,
  .pill.pending {
    color: var(--acct);
    background: var(--accbg);
    border-color: var(--accln);
  }

  .pill.error,
  .pill.missing {
    color: var(--deltx);
    background: var(--delbg);
    border-color: var(--deltx);
  }

  .pill.duplicate {
    color: var(--warn);
    background: var(--warnbg);
    border-color: var(--warnln);
  }

  footer {
    display: flex;
    gap: 22px;
    padding: 9px 40px;
    border-top: 1px solid var(--line);
    font-size: 11px;
    color: var(--dim);
    flex-shrink: 0;
  }
</style>
