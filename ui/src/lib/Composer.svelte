<script lang="ts">
  import { store } from "./store.svelte";

  let draft = $state("");

  function submit(e: Event) {
    e.preventDefault();
    const text = draft;
    draft = "";
    store.send(text);
  }
</script>

<form onsubmit={submit}>
  <input
    type="text"
    bind:value={draft}
    disabled={store.busy}
    placeholder={store.busy ? "레인이 실행 중입니다…" : "레인에 태스크 보내기"}
    aria-label="레인에 태스크 보내기"
  />
  <button class="btn send" type="submit" disabled={store.busy || !draft.trim()} aria-label="보내기">→</button>
</form>

<style>
  form {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--rail);
    padding: 16px 34px 18px;
    display: flex;
    gap: 9px;
  }

  input {
    flex: 1;
    height: 44px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 14px;
    padding: 0 14px;
    outline: none;
  }

  input:focus {
    border-color: var(--acc);
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
</style>
