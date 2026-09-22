<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";
  import Mark, { type LaneState } from "./Mark.svelte";

  // First launch: detect as soon as the window shows, once.
  $effect(() => {
    if (store.agents === null && !store.detecting) store.detect();
  });

  const readyCount = $derived(store.readyAgents.length);

  // The mark is the progress display: one lane per agent.
  const lanes = $derived<LaneState[]>(
    store.detecting || store.agents === null
      ? ["probing", "probing", "probing", "probing"]
      : store.agents.map((a) =>
          a.readiness === "ready"
            ? "ready"
            : a.readiness === "needs_login" || a.readiness === "needs_download"
              ? "attention"
              : a.readiness === "error"
                ? "error"
                : "idle",
        ),
  );

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && !store.detecting) store.setupOpen = false;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop">
  <div class="panel" role="dialog" aria-modal="true" aria-labelledby="setup-title">
    <aside class="side">
      <div class="mlab">SETUP / 01</div>
      <div class="hero">
        <Mark size={168} {lanes} ink="var(--txt)" title="에이전트 감지 상태" />
      </div>
      <h1 id="setup-title" class="serif">어떤 에이전트가 있는지 봅니다.</h1>
      <p>
        설치된 CLI를 찾고, 각각을 ACP로 한 번씩 띄워 로그인 상태까지 확인합니다.
        막 아래 획 하나가 에이전트 하나입니다.
      </p>
      <div class="status mono">
        {#if store.detecting}
          감지 중 · 에이전트마다 몇 초
        {:else if store.agents}
          {readyCount} / {store.agents.length} 준비됨
        {/if}
      </div>
    </aside>

    <section class="main">
      <div class="head">
        <span class="mlab">에이전트</span>
        <span class="grow"></span>
        <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>다시 감지</button>
      </div>
      <div class="list">
        <AgentList />
      </div>
      <div class="foot">
        {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
        <span class="grow"></span>
        <button class="btn" disabled={store.detecting} onclick={() => (store.setupOpen = false)}>나중에</button>
        <button class="btn btn-acc" disabled={readyCount === 0 || store.detecting} onclick={() => (store.setupOpen = false)}>
          계속 →
        </button>
      </div>
    </section>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 36px 0 0 0; /* below the title bar */
    z-index: 20;
    background: color-mix(in srgb, var(--bg) 78%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px 24px;
  }

  .panel {
    width: 960px;
    max-width: 100%;
    max-height: 100%;
    display: flex;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .side {
    width: 300px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding: 24px 26px 22px;
    border-right: 1px solid var(--line);
    background: var(--rail);
  }

  .hero {
    padding: 34px 0 26px;
    color: var(--txt);
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
