<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";
  import ThemePicker from "./ThemePicker.svelte";
  import Mark from "./Mark.svelte";
  import LangPicker from "./LangPicker.svelte";
  import { t } from "./i18n.svelte";

  // First launch: detect as soon as the window shows, once. A failed
  // detection leaves agents null; without the guard the effect would
  // start another probe every time one ended.
  let tried = false;
  $effect(() => {
    if (store.agents === null && !store.detecting && !tried) {
      tried = true;
      store.detect();
    }
  });

  const readyCount = $derived(store.readyAgents.length);
  const step = $derived(store.setupStep);

  function finish() {
    store.setupOpen = false;
    store.setupStep = 1;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && !store.detecting) finish();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop">
  <div class="panel" role="dialog" aria-modal="true" aria-labelledby="setup-title">
    <aside class="side">
      <div class="mlab">{t("setup.step", { step: step === 1 ? "01" : "02", n: step })}</div>
      <div class="hero">
        <Mark size={168} live={store.detecting} />
      </div>
      {#if step === 1}
        <h1 id="setup-title" class="serif">{t("setup.agentsTitle")}</h1>
        <p>{t("setup.agentsBlurb")}</p>
      {:else}
        <h1 id="setup-title" class="serif">{t("setup.lookTitle")}</h1>
        <p>{t("setup.lookBlurb")}</p>
      {/if}
      <div class="status mono">
        {#if step === 1 && store.agents && !store.detecting}
          {t("setup.ready", { ready: readyCount, total: store.agents.length })}
        {:else if step === 2}
          {t("setup.applying", { theme: store.theme === "dk" ? "DARK" : "LIGHT" })}
        {/if}
      </div>
    </aside>

    <section class="main">
      {#if step === 1}
        <div class="head">
          <span class="mlab">{t("setup.agents")}</span>
          <span class="grow"></span>
          <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>{t("setup.redetect")}</button>
        </div>
        <div class="list">
          <AgentList />
        </div>
        <div class="foot">
          {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
          <span class="grow"></span>
          <button class="btn" disabled={store.detecting} onclick={finish}>{t("setup.later")}</button>
          <button class="btn btn-acc" disabled={readyCount === 0 || store.detecting} onclick={() => (store.setupStep = 2)}>
            {t("setup.continue")}
          </button>
        </div>
      {:else}
        <div class="head">
          <span class="mlab">{t("setup.look")}</span>
        </div>
        <div class="body">
          <ThemePicker />
          <div class="mlab sub">{t("lang.label")}</div>
          <LangPicker />
        </div>
        <div class="foot">
          <button class="btn" onclick={() => (store.setupStep = 1)}>{t("setup.backToAgents")}</button>
          <span class="grow"></span>
          <button class="btn btn-acc" onclick={finish}>{t("setup.finish")}</button>
        </div>
      {/if}
    </section>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 32px 0 0 0; /* below the window chrome */
    z-index: 20;
    background: color-mix(in srgb, var(--bg) 78%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  /* Sized by the window, not by its content: the panel follows a window
     resize but never jumps when detection fills the list in. The list
     scrolls inside. */
  .panel {
    width: min(960px, calc(100vw - 48px));
    height: min(640px, calc(100vh - 32px - 48px));
    display: flex;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .side {
    width: 300px;
    flex-shrink: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 24px 26px 22px;
    border-right: 1px solid var(--line);
    background: var(--rail);
    overflow: hidden;
  }

  .hero {
    padding: 28px 0 22px;
    color: var(--txt);
  }

  /* Short windows: give the list the room, not the mark. */
  @media (max-height: 700px) {
    .side {
      width: 260px;
    }
    .hero {
      padding: 16px 0 14px;
    }
    .hero :global(svg) {
      width: 112px;
      height: 112px;
    }
  }

  h1 {
    margin: 0;
    font-weight: 400;
    font-size: 26px;
    line-height: 1.15;
    color: var(--hi);
  }

  p {
    margin: 12px 0 0;
    font-size: 13px;
    line-height: 1.65;
    color: var(--dim);
  }

  .status {
    margin-top: auto;
    padding-top: 22px;
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 20px 28px 18px;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 6px;
    height: 38px;
  }

  .grow {
    flex: 1;
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-bottom: 1px solid var(--line);
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: 12px;
    border-bottom: 1px solid var(--line);
  }

  .sub {
    margin: 22px 0 10px;
  }

  .foot {
    padding-top: 16px;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .err {
    font-size: 10px;
    color: var(--acct);
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .btn-acc {
    height: 36px;
    padding: 0 18px;
  }
</style>
