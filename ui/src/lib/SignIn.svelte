<script lang="ts">
  import { remote } from "./ipc.svelte";
  import Mark from "./Mark.svelte";
  import { t } from "./i18n.svelte";

  /** A remote instance's window that is not (or no longer) paired: how to connect again. */
</script>

<main class="signin">
  <Mark size={48} />
  <h1 class="serif">Divixi</h1>
  {#if remote.state === "dropped"}
    <p>{t("remote.signin.dropped")}</p>
  {:else}
    <p>{t("remote.signin.how")}</p>
  {/if}
  <p class="step">{t("remote.signin.step")}</p>
</main>

<style>
  .signin {
    min-height: 100dvh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    background: var(--bg);
    color: var(--txt);
    text-align: center;
  }

  h1 {
    margin: 0;
    font-size: 26px;
    color: var(--hi);
  }

  p {
    margin: 0;
    max-width: 420px;
    font-size: 15px;
    line-height: 1.6;
    color: var(--dim);
  }

  .step {
    font-size: 14px;
    color: var(--txt);
  }
</style>
