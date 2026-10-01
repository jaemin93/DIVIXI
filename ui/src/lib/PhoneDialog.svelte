<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import PhoneQr from "./PhoneQr.svelte";

  /**
   * The rail's phone button: use Divixi on your phone.
   *
   * A dialog and not a view, because the rail is only ever one level deep and
   * this is one action, not a place. It shows a code, or says what is in the
   * way and sends you to the card that can fix it — it never duplicates the
   * card's state machine.
   */

  let { onclose }: { onclose: () => void } = $props();

  type Status = { step: string; address: string; days: number };

  let status = $state<Status | null>(null);
  let showing = $state(false);

  onMount(async () => {
    try {
      status = await invoke<Status>("phone_status");
      // Ready already: the one reason to open this is to get a code, so do
      // not make them press a second button for it.
      showing = status.step === "ready";
    } catch (err) {
      store.lastError = String(err);
    }
  });

  const ready = $derived(status?.step === "ready");

  function settings() {
    store.openSettings("overview");
    onclose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-label={t("phone.dialog.title")}>
  <div class="head">
    <span class="title">{t("phone.dialog.title")}</span>
    <span class="grow"></span>
    <button class="btn sm" onclick={onclose}>{t("phone.dialog.close")}</button>
  </div>

  <div class="body">
    {#if status === null}
      <p class="hint">{t("phone.qr.making")}</p>
    {:else if ready}
      <!-- The sentence that says whose Divixi this is. Scanning opens THIS
           computer's, which is the whole difference from remote instances. -->
      <p class="blurb">{t("phone.dialog.blurb")}</p>
      {#if showing}
        <PhoneQr onhide={() => (showing = false)} />
      {:else}
        <button class="btn btn-acc" onclick={() => (showing = true)}>{t("phone.dialog.show")}</button>
      {/if}
      <hr />
      <!-- For a camera that cannot be used: the same one-time link, copied
           instead of drawn. One mint, two ways to carry it. -->
      <p class="hint">{t("phone.dialog.noCamera")}</p>
      {#if !showing}
        <button class="btn sm" onclick={() => (showing = true)}>{t("phone.dialog.makeLink")}</button>
      {/if}
    {:else}
      <p class="blurb">{t("phone.dialog.notReady")}</p>
      <button class="btn btn-acc" onclick={settings}>{t("phone.dialog.openSettings")}</button>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 40;
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(28rem, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    overflow: auto;
    background: var(--card);
    border: 1px solid var(--line);
    z-index: 41;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 18px;
    border-bottom: 1px solid var(--line);
  }

  .title {
    font-size: 14px;
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: flex-start;
    padding: 16px 18px 18px;
  }

  .blurb {
    margin: 0;
    font-size: 13px;
    line-height: 1.55;
    color: var(--txt);
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--dim);
  }

  hr {
    width: 100%;
    height: 0;
    margin: 2px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }
</style>
