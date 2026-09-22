<script lang="ts">
  import { store } from "./store.svelte";
</script>

<div class="bar">
  <span class="brand">ORCHESTRA</span>
  <span class="sep"></span>
  <span class="mono ctx">divixi / exp-001</span>
  <span class="grow"></span>
  {#if store.lastError}
    <span class="mono err">{store.lastError}</span>
  {/if}
  <button class="btn" onclick={() => store.toggleTheme()}>
    {store.theme === "dk" ? "LIGHT" : "DARK"}
  </button>
  <span class="mono agents">
    <span class="dot" class:live={!store.busy}></span>claude-code
  </span>
</div>

<style>
  .bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 14px;
    border-bottom: 1px solid var(--line);
    background: var(--rail);
  }

  .brand {
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.22em;
    color: var(--txt);
  }

  .sep {
    width: 1px;
    height: 12px;
    background: var(--lines);
  }

  .ctx,
  .agents,
  .err {
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .err {
    color: var(--acct);
    max-width: 480px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .agents {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--dim);
  }

  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--idle);
  }

  .dot.live {
    background: var(--ok);
  }

  .btn {
    height: 24px;
    padding: 0 9px;
  }
</style>
