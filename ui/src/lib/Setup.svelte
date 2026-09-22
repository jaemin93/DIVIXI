<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";
  import ThemePicker from "./ThemePicker.svelte";
  import Mark from "./Mark.svelte";

  // First launch: detect as soon as the window shows, once.
  $effect(() => {
    if (store.agents === null && !store.detecting) store.detect();
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
      <div class="mlab">SETUP / {step === 1 ? "01" : "02"} · {step} / 2</div>
      <div class="hero">
        <Mark size={168} ink="var(--txt)" live={store.detecting} />
      </div>
      {#if step === 1}
        <h1 id="setup-title" class="serif">어떤 에이전트가 있는지 봅니다.</h1>
        <p>설치된 CLI를 찾고, 각각을 ACP로 한 번씩 띄워 로그인 상태까지 확인합니다.</p>
      {:else}
        <h1 id="setup-title" class="serif">어떤 모습으로 볼지 고릅니다.</h1>
        <p>테마는 언제든 설정이나 타이틀바에서 바꿀 수 있습니다.</p>
      {/if}
      <div class="status mono">
        {#if step === 1 && store.agents && !store.detecting}
          {readyCount} / {store.agents.length} 준비됨
        {:else if step === 2}
          {store.theme === "dk" ? "DARK" : "LIGHT"} 적용 중
        {/if}
      </div>
    </aside>

    <section class="main">
      {#if step === 1}
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
          <button class="btn" disabled={store.detecting} onclick={finish}>나중에</button>
          <button class="btn btn-acc" disabled={readyCount === 0 || store.detecting} onclick={() => (store.setupStep = 2)}>
            계속 →
          </button>
        </div>
      {:else}
        <div class="head">
          <span class="mlab">모습</span>
        </div>
        <div class="body">
          <ThemePicker />
          <p class="hint">System은 OS의 다크 모드 설정을 따르고, OS가 바뀌면 같이 바뀝니다.</p>
        </div>
        <div class="foot">
          <button class="btn" onclick={() => (store.setupStep = 1)}>← 에이전트</button>
          <span class="grow"></span>
          <button class="btn btn-acc" onclick={finish}>완료</button>
        </div>
      {/if}
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
    padding: 24px;
  }

  /* Sized by the window, not by its content: the panel follows a window
     resize but never jumps when detection fills the list in. The list
     scrolls inside. */
  .panel {
    width: min(960px, calc(100vw - 48px));
    height: min(640px, calc(100vh - 36px - 48px));
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

  .hint {
    max-width: 520px;
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
