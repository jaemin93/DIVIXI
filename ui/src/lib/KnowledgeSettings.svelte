<script lang="ts">
  import { invoke, local } from "./ipc.svelte";
  import { store, agentLabel } from "./store.svelte";
  import { kb } from "./knowledge.svelte";
  import { K_AGENT, K_CONFIG, K_EMBED_DIMS, K_EMBED_ENABLED, K_EMBED_KEY, K_EMBED_MODEL, K_EMBED_RATE, K_EMBED_URL, K_EXTRACT, K_POOL, embedSettable, paneKeys } from "./knowledgeKeys";
  import { t } from "./i18n.svelte";

  /** The embedding is set on this PC only: from another device it is not shown, read or saved. */
  const embedHere = embedSettable(local);

  /**
   * How the knowledge library describes documents (the agent, its model
   * and effort, how many sessions at once) and where its vectors come from
   * (a remote embedding API). A pane of the settings page.
   */
  // Read once when the pane opens, written as they change.
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
  let eRate = $state("120");
  let eTest = $state<{ ok: boolean; text: string } | null>(null);
  let eTesting = $state(false);

  const get = (key: string) => invoke<string | null>("get_setting", { key }).catch(() => null);

  $effect(() => {
    if (sLoaded) return;
    void (async () => {
      const keys = paneKeys(embedHere);
      const read = await Promise.all(keys.map(get));
      const value = (key: string) => read[keys.indexOf(key)] ?? null;
      const [agent, config, pool, extract] = [K_AGENT, K_CONFIG, K_POOL, K_EXTRACT].map(value);
      const [on, url, model, key, dims, rate] = [K_EMBED_ENABLED, K_EMBED_URL, K_EMBED_MODEL, K_EMBED_KEY, K_EMBED_DIMS, K_EMBED_RATE].map(value);
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
      eRate = rate ?? "120";
      eSaved = { on: eOn, url: eUrl.trim(), model: eModel.trim(), key: eKey.trim(), dims: eDims.trim(), rate: rateValue(eRate) };
      await kb.loadEmbedding();
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

  /** Write one setting; false (and the error shown) when it failed. */
  async function save(key: string, value: string): Promise<boolean> {
    try {
      await invoke("set_setting", { key, value });
      return true;
    } catch (err) {
      store.lastError = String(err);
      return false;
    }
  }

  /** A new agent starts on its cheapest model and least effort. */
  async function setAgent(agent: string) {
    if (agent === sAgent) return;
    sAgent = agent;
    const found = await defaults(agent);
    // Another agent may have been chosen meanwhile.
    if (sAgent !== agent) return;
    cheap = found;
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

  /** What is saved, to tell unsaved edits apart. */
  let eSaved = $state({ on: false, url: "", model: "", key: "", dims: "", rate: "120" });

  /** The limit as saved: a whole number of requests a minute, 0 for none. */
  function rateValue(v: string): string {
    const n = Math.floor(Number(v));
    return Number.isFinite(n) && n >= 0 ? String(n) : "120";
  }
  let saving = $state(false);
  const dirty = $derived(
    eOn !== eSaved.on || eUrl.trim() !== eSaved.url || eModel.trim() !== eSaved.model || eKey.trim() !== eSaved.key || eDims.trim() !== eSaved.dims || rateValue(eRate) !== eSaved.rate,
  );

  async function saveEmbedding() {
    if (!embedHere) return;
    saving = true;
    // What is written is what was on screen when Save was pressed; edits
    // made meanwhile stay unsaved.
    const snap = { on: eOn, url: eUrl.trim(), model: eModel.trim(), key: eKey.trim(), dims: eDims.trim(), rate: rateValue(eRate) };
    try {
      const writes = [
        [K_EMBED_URL, snap.url],
        [K_EMBED_MODEL, snap.model],
        [K_EMBED_KEY, snap.key],
        [K_EMBED_DIMS, snap.dims],
        [K_EMBED_RATE, snap.rate],
        [K_EMBED_ENABLED, snap.on ? "on" : "off"],
      ];
      for (const [key, value] of writes) {
        if (!(await save(key, value))) return;
      }
      eSaved = snap;
      await invoke("knowledge_embed_now").catch((err) => {
        store.lastError = String(err);
      });
      await kb.loadEmbedding();
    } finally {
      saving = false;
    }
  }

</script>

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

    {#if embedHere}
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
<label class="field"><span>{t("kb.emb.rate")}</span><input class="short" bind:value={eRate} disabled={!eOn} inputmode="numeric" /><em class="unit">{t("kb.emb.rateUnit")}</em></label>
<p class="note keynote">{t("kb.emb.rateHelp")}</p>
      <div class="field">
        <span></span>
        <button class="btn sm" disabled={!eOn || eTesting || !eUrl.trim() || !eModel.trim()} onclick={testEndpoint}>{eTesting ? t("kb.emb.testing") : t("kb.emb.test")}</button>
        {#if eTest}<span class="test" class:bad={!eTest.ok}>{eTest.text}</span>{/if}
      </div>
      <p class="note keynote">{t("kb.emb.keyNote")}</p>
    </div>
    <div class="actions savebar">
      <span class="state" class:bad={!dirty && eSaved.on && !!kb.embedding.error}>
        {#if saving}
          {t("kb.emb.saving")}
        {:else if dirty}
          {t("kb.emb.unsaved")}
        {:else if !eSaved.on}
          {t("kb.emb.savedOff")}
        {:else if kb.embedding.error}
          {t("kb.embedError", { done: kb.embedding.embedded, total: kb.embedding.total, error: kb.embedding.error })}
        {:else if kb.embedding.embedded + kb.embedding.failed < kb.embedding.total}
          <span class="pulse"></span>{t("kb.emb.working", { done: kb.embedding.embedded, total: kb.embedding.total })}
        {:else}
          ✓ {t("kb.emb.savedDone", { done: kb.embedding.embedded, total: kb.embedding.total })}
          {#if kb.embedding.failed}<span class="dim">· {t("kb.emb.refused", { n: kb.embedding.failed })}</span>{/if}
        {/if}
      </span>
      <span class="grow"></span>
      <button class="btn btn-acc" disabled={!dirty || saving} onclick={saveEmbedding}>{t("kb.emb.save")}</button>
    </div>
    {/if}
  </div>

<style>
  .note {
    color: var(--dim);
    font-size: 12.5px;
    margin: 0 0 12px;
    line-height: 1.55;
  }

  .dim {
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .num {
    width: 80px;
    height: 34px;
    text-align: center;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 13px;
    font-family: inherit;
  }

  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--acc);
    flex-shrink: 0;
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
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

  .field > span:first-child {
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
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    color: var(--oktx);
    word-break: break-word;
  }

  .test.bad {
    color: var(--deltx);
  }

  .unit {
    font-style: normal;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--lab);
  }

  .keynote {
    margin: 10px 0 0 122px;
  }

  .savebar {
    margin-top: 16px;
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--dim);
    min-width: 0;
    word-break: break-word;
  }

  .state.bad {
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

</style>
