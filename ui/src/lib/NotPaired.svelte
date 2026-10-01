<script lang="ts">
  import { session } from "./ipc.svelte";
  import { t } from "./i18n.svelte";

  /**
   * What a browser sees at this address without a pairing.
   *
   * Someone on the tailnet who opens the address out of curiosity, or a
   * phone whose sign-in has run out, gets this rather than an app that fails
   * on every request. It is the whole page: there is nothing behind it to
   * show, and a shell full of empty panes would only look broken.
   */
</script>

<div class="page">
  <div class="card">
    <h1>{t("web.notPaired")}</h1>
    <p>{t("web.notPairedHow")}</p>
    {#if session.error}
      <p class="warn">{session.error}</p>
    {/if}
  </div>
</div>

<style>
  .page {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100dvh;
    padding: 1.5rem;
    background: var(--bg);
  }

  .card {
    max-width: 30rem;
  }

  h1 {
    margin: 0 0 0.75rem;
    font-size: 1.1rem;
    line-height: 1.45;
    color: var(--hi);
  }

  p {
    margin: 0 0 0.75rem;
    font-size: 0.85rem;
    line-height: 1.6;
    color: var(--dim);
  }

  .warn {
    color: var(--warn);
  }
</style>
