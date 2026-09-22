<script lang="ts">
  import { store } from "./store.svelte";
  import Mark from "./Mark.svelte";
</script>

<div class="bar">
  <span class="brand"><Mark size={14} live={store.busy} />ORCHESTRA</span>
  <span class="grow"></span>
  {#if store.lastError}
    <span class="mono err">{store.lastError}</span>
  {/if}
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
    display: flex;
    align-items: center;
    gap: 9px;
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.22em;
    color: var(--txt);
  }

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
</style>
