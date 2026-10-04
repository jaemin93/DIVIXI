<script lang="ts">
  import { onMount } from "svelte";
  import { instanceId, instanceName, switchInstance } from "./ipc.svelte";
  import { startup } from "./startup.svelte";
  import { slow } from "./startup";
  import { store } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import { t } from "./i18n.svelte";

  /**
   * On the way to the Divixi this webview shows: the mark turning, and
   * which instance it is. Stands where the first-track screen would
   * otherwise flash while the tracks are still on their way (startup.ts).
   * The mark's own motion, and its own reduced-motion breathing; no other.
   *
   * After a while it says so, and a remote instance offers the way back to
   * Local. Past the deadline this gives way to the Unreachable page.
   */
  let now = $state(Date.now());
  onMount(() => {
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });

  const remote = instanceId !== null;
  const name = remote ? (instanceName ?? t("instances.remote")) : t("instances.local");
  const late = $derived(slow(startup.phase, startup.since, now));

  function backLocal() {
    switchInstance(null).catch((err) => (store.lastError = String(err)));
  }
</script>

<main class="cn" aria-busy="true">
  <Mark size={40} live={true} />
  <div class="said" role="status" aria-live="polite">
    <h1 class="serif">{remote ? t("instances.connecting") : t("instances.loading")}</h1>
    <p class="mono name">{name}</p>
  </div>
  {#if late}
    <p class="slow">{t("instances.slow")}</p>
    {#if remote}<button class="btn" onclick={backLocal}>{t("instances.backLocal")}</button>{/if}
  {/if}
</main>

<style>
  .cn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    color: var(--txt);
  }

  .said {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  h1 {
    margin: 0;
    font-weight: 400;
    font-size: 22px;
    color: var(--hi);
  }

  .name {
    margin: 0;
    max-width: 480px;
    font-size: 12px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .slow {
    margin: 0;
    font-size: 12px;
    color: var(--dim);
  }
</style>
