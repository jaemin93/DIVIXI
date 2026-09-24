<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { store, agentLabel } from "./store.svelte";
  import {
    kb,
    K_AGENT,
    K_CONFIG,
    K_EMBED_DIMS,
    K_EMBED_ENABLED,
    K_EMBED_KEY,
    K_EMBED_MODEL,
    K_EMBED_URL,
    K_EXTRACT,
    K_POOL,
    type KItem,
    type KSource,
    type KTab,
  } from "./knowledge.svelte";
  import KnowledgeGraph from "./KnowledgeGraph.svelte";
  import ArtifactMenu from "./ArtifactMenu.svelte";
  import Icon from "./Icon.svelte";
  import { whenFull, whenLabel } from "./time";
  import { t } from "./i18n.svelte";

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
    { id: "settings", label: t("kb.tab.settings") },
  ]);

  let query = $state(kb.query);
  let folded = $state<Record<string, boolean>>({});
  let open = $state<Record<number, boolean>>({});
  let copied = $state(0);
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  let confirmRemove = $state("");

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

  // Settings: read once when the tab opens, written as they change.
  let sAgent = $state("");
  let sConfig = $state<Record<string, string>>({});
  let sPool = $state(2);
  let sExtract = $state(true);
  let sLoaded = $state(false);
  /** The cheapest options on the chosen agent, to mark them. */
  let cheap = $state<Record<string, string>>({});
  let eOn = $state(false);
  let eUrl = $state("");
  let eModel = $state("");
  let eKey = $state("");
  let eDims = $state("");
  let eTest = $state<{ ok: boolean; text: string } | null>(null);
  let eTesting = $state(false);

  const get = (key: string) => invoke<string | null>("get_setting", { key }).catch(() => null);

  $effect(() => {
    if (kb.tab !== "settings" || sLoaded) return;
    void (async () => {
      const [agent, config, pool, extract, on, url, model, key, dims] = await Promise.all(
        [K_AGENT, K_CONFIG, K_POOL, K_EXTRACT, K_EMBED_ENABLED, K_EMBED_URL, K_EMBED_MODEL, K_EMBED_KEY, K_EMBED_DIMS].map(get),
      );
      sAgent = agent || store.readyAgents[0]?.kind || "";
      cheap = await defaults(sAgent);
      try {
        sConfig = config ? JSON.parse(config) : { ...cheap };
      } catch {
        sConfig = { ...cheap };
      }
      sPool = Math.min(5, Math.max(1, Number(pool) || 2));
      sExtract = extract !== "off";
      eOn = on === "on";
      eUrl = url ?? "";
      eModel = model ?? "";
      eKey = key ?? "";
      eDims = dims ?? "";
      sLoaded = true;
    })();
  });

  async function defaults(agent: string): Promise<Record<string, string>> {
    if (!agent) return {};
    return invoke<Record<string, string>>("knowledge_default_config", { agent }).catch(() => ({}));
  }

  const agentOptions = $derived(store.optionsOf(sAgent));
  const modelOption = $derived(agentOptions.find((o) => o.category === "model"));
  const effortOption = $derived(agentOptions.find((o) => o.category === "thought_level"));

  async function save(key: string, value: string) {
    try {
      await invoke("set_setting", { key, value });
    } catch (err) {
      store.lastError = String(err);
    }
  }

  /** A new agent starts on its cheapest model and least effort. */
  async function setAgent(agent: string) {
    if (agent === sAgent) return;
    sAgent = agent;
    cheap = await defaults(agent);
    sConfig = { ...cheap };
    await save(K_AGENT, agent);
    await save(K_CONFIG, JSON.stringify(sConfig));
  }

  async function setOption(id: string, value: string) {
    const next = { ...sConfig };
    if (value) next[id] = value;
    else delete next[id];
    sConfig = next;
    await save(K_CONFIG, JSON.stringify(next));
  }

  /** Remote embedding endpoints that speak OpenAI's /embeddings. */
  const PRESETS = [
    { id: "openai", label: "OpenAI", url: "https://api.openai.com/v1", model: "text-embedding-3-small", dims: "" },
    { id: "voyage", label: "Voyage", url: "https://api.voyageai.com/v1", model: "voyage-3.5-lite", dims: "" },
    { id: "gemini", label: "Gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-embedding-001", dims: "768" },
    { id: "custom", label: t("kb.emb.custom"), url: "", model: "", dims: "" },
  ];

  function preset(p: (typeof PRESETS)[number]) {
    eUrl = p.url;
    eModel = p.model;
    eDims = p.dims;
    eTest = null;
  }

  async function testEndpoint() {
    eTesting = true;
    eTest = null;
    try {
      const n = await invoke<number>("knowledge_embed_test", { url: eUrl, model: eModel, key: eKey, dims: Number(eDims) || null });
      eTest = { ok: true, text: t("kb.emb.testOk", { n }) };
    } catch (err) {
      eTest = { ok: false, text: String(err) };
    } finally {
      eTesting = false;
    }
  }

  async function saveEmbedding() {
    await save(K_EMBED_URL, eUrl.trim());
    await save(K_EMBED_MODEL, eModel.trim());
    await save(K_EMBED_KEY, eKey.trim());
    await save(K_EMBED_DIMS, eDims.trim());
    await save(K_EMBED_ENABLED, eOn ? "on" : "off");
    await invoke("knowledge_embed_now").catch(() => {});
    await kb.loadEmbedding();
  }

  const menuSource = $derived(menu ? kb.sources.find((s) => s.id === menu!.id) : undefined);
  const menuArtifact = $derived(menu ? kb.artifactOf(menu.id) : undefined);
</script>

<section class="page">
  <div class="inner">
    <header>
      <div class="mlab">KNOWLEDGE</div>
      <h1 class="serif">{t("kb.title")}</h1>
      <p class="sub">{t("kb.sub")}</p>
    </header>

    <div class="tabs" role="tablist">
      {#each tabs as tab (tab.id)}
        <button role="tab" aria-selected={kb.tab === tab.id} class:on={kb.tab === tab.id} onclick={() => kb.setTab(tab.id)}>{tab.label}</button>
      {/each}
    </div>

    {#if kb.embedding.enabled && kb.embedding.error}
      <div class="banner err">{t("kb.embedError", { error: kb.embedding.error })}</div>
    {/if}
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

    {#if kb.tab === "list"}
      <div class="filters">
        <input
          class="search"
          placeholder={t("kb.searchPlaceholder")}
          bind:value={query}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              kb.query = query;
              void kb.search(query);
            } else if (e.key === "Escape" && query) {
              query = "";
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
          <p>{t("kb.empty")}</p>
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
      <p class="note">{t("kb.sourcesNote")}</p>
      <div class="actions">
        <span class="mono dim">{t("kb.formats", { list: kb.formats.slice(0, 12).join(" ") })}…</span>
        <span class="grow"></span>
        <button class="btn btn-acc" onclick={() => kb.pickAndAdd()}>{t("kb.addSource")}</button>
      </div>
      {#if !kb.sources.length}
        <p class="none">{t("kb.empty")}</p>
      {/if}
      {#each kb.sources as s (s.id)}
        {@const art = kb.artifactOf(s.id)}
        <div
          class="source"
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
    {:else if kb.tab === "settings"}
      <div class="settings">
        <h2>{t("kb.set.title")}</h2>
        <p class="note">{t("kb.set.blurb")}</p>

        <div class="row">
          <div class="lhs">
            <div class="label">{t("kb.set.extract")}</div>
            <div class="help">{t("kb.set.extractHelp")}</div>
          </div>
          <button
            class="toggle"
            class:on={sExtract}
            role="switch"
            aria-checked={sExtract}
            aria-label={t("kb.set.extract")}
            onclick={() => {
              sExtract = !sExtract;
              void save(K_EXTRACT, sExtract ? "on" : "off");
            }}><span></span></button
          >
        </div>

        <div class="block" class:off={!sExtract}>
          <div class="label">{t("kb.set.agent")}</div>
          <div class="help">{t("kb.set.agentHelp")}</div>
          <div class="agents" role="radiogroup" aria-label={t("kb.set.agent")}>
            {#each store.readyAgents as a (a.kind)}
              <button class="agent" class:on={sAgent === a.kind} role="radio" aria-checked={sAgent === a.kind} disabled={!sExtract} onclick={() => setAgent(a.kind)}>
                {agentLabel(a.kind)}
              </button>
            {/each}
            {#if !store.readyAgents.length}<span class="dim">{t("kb.set.noAgents")}</span>{/if}
          </div>

          {#if modelOption}
            <div class="label sub">{t("kb.set.model")}</div>
            <div class="help">{t("kb.set.modelHelp")}</div>
            <div class="models" role="radiogroup" aria-label={t("kb.set.model")}>
              {#each modelOption.choices as c (c.id)}
                {@const chosen = (sConfig[modelOption.id] ?? "") === c.id}
                <button class="model" class:on={chosen} role="radio" aria-checked={chosen} disabled={!sExtract} onclick={() => setOption(modelOption.id, c.id)}>
                  <span class="mname">{c.name}</span>
                  {#if cheap[modelOption.id] === c.id}<span class="mono cheap">{t("kb.set.cheapest")}</span>{/if}
                  {#if c.id === modelOption.current}<span class="mono dflt">{t("kb.set.agentDefault")}</span>{/if}
                </button>
              {/each}
            </div>
            {#if !cheap[modelOption.id]}<p class="note">{t("kb.set.noCheapest")}</p>{/if}
          {/if}

          {#if effortOption}
            <div class="label sub">{effortOption.name}</div>
            <div class="agents">
              {#each effortOption.choices as c (c.id)}
                {@const chosen = (sConfig[effortOption.id] ?? effortOption.current) === c.id}
                <button class="agent" class:on={chosen} disabled={!sExtract} onclick={() => setOption(effortOption.id, c.id)}>{c.name}</button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="row">
          <div class="lhs">
            <div class="label">{t("kb.set.pool")}</div>
            <div class="help">{t("kb.set.poolHelp")}</div>
          </div>
          <input
            class="num"
            type="number"
            min="1"
            max="5"
            value={sPool}
            disabled={!sExtract}
            onchange={(e) => {
              sPool = Math.min(5, Math.max(1, Number((e.currentTarget as HTMLInputElement).value) || 2));
              void save(K_POOL, String(sPool));
            }}
          />
        </div>
        <p class="note">{t("kb.set.applies")}</p>

        <h2 class="gap">{t("kb.emb.title")}</h2>
        <p class="note">{t("kb.emb.blurb")}</p>
        <div class="row">
          <div class="lhs">
            <div class="label">{t("kb.emb.enable")}</div>
            <div class="help">{t("kb.emb.enableHelp")}</div>
          </div>
          <button class="toggle" class:on={eOn} role="switch" aria-checked={eOn} aria-label={t("kb.emb.enable")} onclick={() => (eOn = !eOn)}><span></span></button>
        </div>
        <div class="block" class:off={!eOn}>
          <div class="agents">
            {#each PRESETS as p (p.id)}
              <button class="agent" class:on={p.url !== "" && eUrl === p.url && eModel === p.model} disabled={!eOn} onclick={() => preset(p)}>{p.label}</button>
            {/each}
          </div>
          <label class="field"><span>{t("kb.emb.url")}</span><input bind:value={eUrl} disabled={!eOn} placeholder="https://api.openai.com/v1" spellcheck="false" /></label>
          <label class="field"><span>{t("kb.emb.model")}</span><input bind:value={eModel} disabled={!eOn} placeholder="text-embedding-3-small" spellcheck="false" /></label>
          <label class="field"><span>{t("kb.emb.key")}</span><input type="password" bind:value={eKey} disabled={!eOn} placeholder="sk-…" autocomplete="off" /></label>
          <label class="field"><span>{t("kb.emb.dims")}</span><input class="short" bind:value={eDims} disabled={!eOn} placeholder={t("kb.emb.dimsAuto")} inputmode="numeric" /></label>
          <p class="note">{t("kb.emb.keyNote")}</p>
          {#if eTest}<p class="test" class:bad={!eTest.ok}>{eTest.text}</p>{/if}
        </div>
        <div class="actions">
          <button class="btn" disabled={!eOn || eTesting || !eUrl.trim() || !eModel.trim()} onclick={testEndpoint}>{eTesting ? t("kb.emb.testing") : t("kb.emb.test")}</button>
          <span class="grow"></span>
          <button class="btn btn-acc" onclick={saveEmbedding}>{t("kb.emb.save")}</button>
        </div>
      </div>
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
    padding: 30px 40px 40px;
    width: 100%;
    box-sizing: border-box;
  }

  header h1 {
    margin: 6px 0 4px;
    font-size: 30px;
    font-weight: 400;
    color: var(--hi);
  }

  .sub {
    margin: 0 0 18px;
    color: var(--dim);
    font-size: 13.5px;
  }

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 16px;
  }

  .tabs button {
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    padding: 9px 14px;
    font-size: 13.5px;
    color: var(--dim);
    margin-bottom: -1px;
  }

  .tabs button:hover {
    color: var(--hi);
  }

  .tabs button.on {
    color: var(--hi);
    border-bottom-color: var(--acc);
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
  select,
  .num {
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

  .num {
    width: 80px;
    text-align: center;
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

  .emptybox p {
    margin: 0;
    max-width: 520px;
    line-height: 1.55;
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

  .actions .dim {
    font-size: 10.5px;
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

  .settings {
    max-width: 760px;
  }

  .settings h2 {
    font-size: 17px;
    font-weight: 500;
    color: var(--hi);
    margin: 4px 0 8px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 16px 0;
    border-bottom: 1px solid var(--line);
  }

  .lhs {
    flex: 1;
    min-width: 0;
  }

  .label {
    color: var(--hi);
    font-size: 14px;
  }

  .help {
    color: var(--dim);
    font-size: 12.5px;
    margin-top: 4px;
    line-height: 1.5;
  }

  .block {
    padding: 16px 0;
    border-bottom: 1px solid var(--line);
  }

  .block.off {
    opacity: 0.55;
  }

  .label.sub {
    margin-top: 16px;
  }

  .agents,
  .models {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }

  .agent,
  .model {
    background: transparent;
    border: 1px solid var(--lines);
    color: var(--dim);
    font-size: 12.5px;
    padding: 6px 12px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  .agent:hover:not(:disabled),
  .model:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .agent.on,
  .model.on {
    color: var(--hi);
    border-color: var(--acc);
    background: var(--accbg);
  }

  .agent:disabled,
  .model:disabled {
    cursor: default;
  }

  .cheap {
    font-size: 9.5px;
    color: var(--oktx);
    background: var(--okbg);
    border: 1px solid var(--okln);
    padding: 0 5px;
  }

  .dflt {
    font-size: 9.5px;
    color: var(--lab);
  }

  .gap {
    margin-top: 34px !important;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 10px;
    font-size: 12.5px;
    color: var(--dim);
  }

  .field span {
    width: 110px;
    flex-shrink: 0;
  }

  .field input {
    flex: 1;
    min-width: 0;
    height: 32px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    padding: 0 10px;
    font-family: var(--mono);
    font-size: 12px;
  }

  .field input.short {
    flex: 0 0 140px;
  }

  .test {
    font-size: 12.5px;
    color: var(--oktx);
    margin: 10px 0 0;
    word-break: break-word;
  }

  .test.bad {
    color: var(--deltx);
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
