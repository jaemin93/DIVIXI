<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";

  // First launch: detect as soon as the window shows, once.
  $effect(() => {
    if (store.agents === null && !store.detecting) store.detect();
  });

  const readyCount = $derived(store.readyAgents.length);

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && !store.detecting) store.setupOpen = false;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop">
  <div class="panel" role="dialog" aria-modal="true" aria-labelledby="setup-title">
    <div class="head">
      <div class="mlab">SETUP / 01 · 에이전트</div>
      <span class="grow"></span>
      <span class="status mono">
        {#if store.detecting}
          <span class="dot pulse" style="background: var(--ok)"></span>감지 중
        {:else if store.agents}
          <span class="dot" style="background: {readyCount ? 'var(--ok)' : 'var(--warn)'}"></span>
          {readyCount} / {store.agents.length} 준비됨
        {/if}
      </span>
    </div>

    <h1 id="setup-title" class="serif">어떤 에이전트가 있는지 봅니다.</h1>
    <p>
      설치된 CLI를 찾고, 각각을 ACP로 한 번씩 띄워 로그인 상태까지 확인합니다.
      나중에 설치하거나 로그인하면 설정에서 다시 감지할 수 있습니다.
    </p>

    <div class="list">
      <AgentList />
    </div>

    <div class="foot">
      <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>다시 감지</button>
      <span class="grow"></span>
      {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
      <button class="btn" disabled={store.detecting} onclick={() => (store.setupOpen = false)}>나중에</button>
      <button class="btn btn-acc" disabled={readyCount === 0 || store.detecting} onclick={() => (store.setupOpen = false)}>
        계속 →
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 36px 0 0 0; /* below the title bar */
    z-index: 20;
    background: color-mix(in srgb, var(--bg) 78%, transparent);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 48px 24px;
    overflow-y: auto;
  }

  .panel {
    width: 760px;
    max-width: 100%;
    max-height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--card);
    border: 1px solid var(--lines);
    border-left: 2px solid var(--acc);
    padding: 22px 28px 20px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  h1 {
    margin: 14px 0 0;
    font-weight: 400;
    font-size: 30px;
    line-height: 1.1;
    color: var(--hi);
  }

  p {
    margin: 10px 0 18px;
    font-size: 13px;
    line-height: 1.65;
    color: var(--dim);
    max-width: 560px;
  }

  .list {
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

  .grow {
    flex: 1;
  }

  .err {
    font-size: 10px;
    color: var(--acct);
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .btn-acc {
    height: 36px;
    padding: 0 18px;
  }
</style>
