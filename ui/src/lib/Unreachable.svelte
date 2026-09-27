<script lang="ts">
  import { instance, switchInstance } from "./ipc.svelte";
  import Mark from "./Mark.svelte";
  import { t } from "./i18n.svelte";

  /** The chosen remote instance could not be reached: why, and the ways on. */
</script>

<main class="un">
  <Mark size={40} />
  <h1 class="serif">{t("instances.unreachable")}</h1>
  <pre class="mono why">{instance.error}</pre>
  <div class="acts">
    <button class="btn btn-acc" onclick={() => location.reload()}>{t("instances.retry")}</button>
    <button class="btn" onclick={() => switchInstance(null)}>{t("instances.backLocal")}</button>
  </div>
</main>

<style>
  .un {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    color: var(--txt);
  }

  h1 {
    margin: 0;
    font-weight: 400;
    font-size: 22px;
    color: var(--hi);
  }

  .why {
    max-width: 640px;
    margin: 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--dim);
    white-space: pre-wrap;
    text-align: center;
  }

  .acts {
    display: flex;
    gap: 8px;
  }
</style>
