<script lang="ts">
  import { store } from "./store.svelte";
  import AgentList from "./AgentList.svelte";
  import ThemePicker from "./ThemePicker.svelte";
</script>

<div class="settings">
  <div class="col">
    <div class="top">
      <div>
        <div class="mlab">설정</div>
        <h1 class="serif">모습</h1>
      </div>
      <span class="grow"></span>
      <button class="btn" onclick={() => (store.view = "track")}>닫기</button>
    </div>
    <p>System은 OS의 다크 모드 설정을 따릅니다.</p>
    <ThemePicker />

    <div class="top section">
      <div>
        <h1 class="serif">에이전트</h1>
      </div>
      <span class="grow"></span>
      <button class="btn" disabled={store.detecting} onclick={() => store.detect()}>
        {store.detecting ? "감지 중…" : "다시 감지"}
      </button>
      <button class="btn" onclick={() => { store.setupStep = 1; store.setupOpen = true; }}>SETUP 열기</button>
    </div>
    <p>
      앱을 켜 둔 채 설치하거나 로그인해도 여기서 다시 감지하면 반영됩니다.
      레인은 기본 에이전트로 열립니다. 컴포저에서 런마다 바꿀 수 있습니다.
    </p>
    <AgentList />
  </div>
</div>

<style>
  .settings {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    background: var(--bg);
    display: flex;
    justify-content: center;
  }

  .col {
    width: 720px;
    max-width: calc(100% - 68px);
    padding: 40px 0 48px;
  }

  .top {
    display: flex;
    align-items: flex-end;
    gap: 10px;
    padding-bottom: 6px;
  }

  .section {
    margin-top: 44px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
  }

  h1 {
    margin: 8px 0 0;
    font-weight: 400;
    font-size: 30px;
    line-height: 1.1;
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  p {
    margin: 10px 0 22px;
    font-size: 13px;
    line-height: 1.65;
    color: var(--dim);
    max-width: 560px;
  }
</style>
