<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";

  // First launch: detect as soon as the screen shows, once.
  $effect(() => {
    if (store.agents === null && !store.detecting) store.detect();
  });

  const readyCount = $derived(store.readyAgents.length);
</script>

<div class="setup">
  <div class="col">
    <div class="mlab">SETUP / 01</div>
    <h1 class="serif">어떤 에이전트가 있는지 봅니다.</h1>
    <p>
      설치된 CLI를 찾고, 각각을 ACP로 한 번씩 띄워 로그인 상태까지 확인합니다.
      나중에 설치하거나 로그인하면 설정에서 다시 감지할 수 있습니다.
    </p>

    <div class="status mono">
      {#if store.detecting}
        <span class="dot pulse" style="background: var(--ok)"></span>감지 중 · 에이전트마다 몇 초 걸립니다
      {:else if store.agents}
        <span class="dot" style="background: {readyCount ? 'var(--ok)' : 'var(--warn)'}"></span>
        {readyCount} / {store.agents.length} 준비됨
      {/if}
    </div>

    <AgentList />

    <div class="foot">
      <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>다시 감지</button>
      <span class="grow"></span>
      {#if store.lastError}<span class="mono err">{store.lastError}</span>{/if}
      <button class="btn btn-acc" disabled={readyCount === 0 || store.detecting} onclick={() => (store.view = "track")}>
        계속 →
      </button>
    </div>
  </div>
</div>

<style>
  .setup {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg);
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
    display: flex;
    justify-content: center;
  }

  .col {
    width: 720px;
    max-width: calc(100% - 68px);
    padding: 56px 0 48px;
  }

  h1 {
    margin: 10px 0 0;
    font-weight: 400;
    font-size: 36px;
    line-height: 1.1;
    color: var(--hi);
  }

  p {
    margin: 12px 0 26px;
    font-size: 14px;
    line-height: 1.65;
    color: var(--dim);
    max-width: 560px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
    padding-bottom: 12px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .foot {
    margin-top: 22px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
    display: flex;
    align-items: center;
    gap: 12px;
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

  .btn-acc {
    height: 36px;
    padding: 0 18px;
  }
</style>
