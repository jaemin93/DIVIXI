<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";

  /**
   * Settings › Remote access: turn the app's server on, pair a device with
   * a QR (a link good once, for five minutes), and see and drop paired
   * devices. Reaching it from outside this PC (Tailscale) comes next.
   */
  type Device = { id: string; name: string; created: number; last_seen: number; login?: string | null };
  type Status = { enabled: boolean; running: boolean; port: number; local_url: string; devices: Device[] };
  type Pairing = { url: string; expires: number; qr_svg: string };

  let status = $state<Status | null>(null);
  let busy = $state(false);
  let pairing = $state<Pairing | null>(null);
  let left = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;
  let confirmAll = $state(false);

  async function load() {
    try {
      status = await invoke<Status>("remote_status");
    } catch (err) {
      store.lastError = String(err);
    }
  }
  onMount(load);
  onDestroy(() => clearInterval(timer));

  async function toggle() {
    if (!status) return;
    busy = true;
    try {
      status = await invoke<Status>("remote_set_enabled", { enabled: !status.enabled });
      if (!status.enabled) pairing = null;
    } catch (err) {
      store.lastError = String(err);
    } finally {
      busy = false;
    }
  }

  async function pair() {
    try {
      pairing = await invoke<Pairing>("remote_pair");
      clearInterval(timer);
      const tick = () => {
        left = Math.max(0, pairing ? pairing.expires - Math.floor(Date.now() / 1000) : 0);
        if (left === 0) {
          clearInterval(timer);
          // The new device shows up in the list once it has paired.
          void load();
        }
      };
      tick();
      timer = setInterval(tick, 1000);
    } catch (err) {
      store.lastError = String(err);
    }
  }

  async function drop(device?: string) {
    try {
      status = await invoke<Status>("remote_drop", { device: device ?? null });
      confirmAll = false;
    } catch (err) {
      store.lastError = String(err);
    }
  }

  const mmss = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
</script>

{#if status}
  <div class="rs">
    <div class="line">
      <div class="what">
        <div class="title">{t("remote.server")}</div>
        <div class="dim">{status.running ? t("remote.runningAt", { url: status.local_url }) : t("remote.off")}</div>
      </div>
      <button class="btn" class:btn-acc={!status.enabled} disabled={busy} onclick={toggle}>{status.enabled ? t("remote.turnOff") : t("remote.turnOn")}</button>
    </div>

    {#if status.running}
      <div class="line">
        <div class="what">
          <div class="title">{t("remote.pair")}</div>
          <div class="dim">{t("remote.pairNote")}</div>
        </div>
        <button class="btn" onclick={pair}>{pairing && left > 0 ? t("remote.pairAgain") : t("remote.pairButton")}</button>
      </div>
      {#if pairing && left > 0}
        <div class="qr">
          <div class="code">{@html pairing.qr_svg}</div>
          <div class="qrside">
            <div class="mono left">{t("remote.expiresIn", { time: mmss(left) })}</div>
            <input class="mono link" readonly value={pairing.url} onfocus={(e) => e.currentTarget.select()} />
            <button class="btn sm" onclick={() => navigator.clipboard.writeText(pairing!.url)}>{t("remote.copyLink")}</button>
          </div>
        </div>
      {/if}
    {/if}

    <div class="mlab-sm head">{t("remote.devices", { n: status.devices.length })}</div>
    {#if !status.devices.length}
      <div class="dim none">{t("remote.noDevices")}</div>
    {/if}
    {#each status.devices as d (d.id)}
      <div class="device">
        <div class="what">
          <div class="title">{d.name}</div>
          <div class="mono dim">
            {t("remote.lastSeen", { when: whenLabel(d.last_seen * 1000, store.now, store.lang) })}{#if d.login} · {d.login}{/if}
          </div>
        </div>
        <button class="btn sm" onclick={() => drop(d.id)}>{t("remote.drop")}</button>
      </div>
    {/each}
    {#if status.devices.length > 1}
      {#if confirmAll}
        <div class="line">
          <span class="warn">{t("remote.dropAllAsk")}</span>
          <button class="btn sm danger" onclick={() => drop()}>{t("remote.dropAll")}</button>
          <button class="btn sm" onclick={() => (confirmAll = false)}>{t("remote.cancel")}</button>
        </div>
      {:else}
        <button class="btn sm allbtn" onclick={() => (confirmAll = true)}>{t("remote.dropAll")}</button>
      {/if}
    {/if}

    <p class="note">{t("remote.scopeNote")}</p>
  </div>
{/if}

<style>
  .rs {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 640px;
  }

  .line,
  .device {
    display: flex;
    align-items: center;
    gap: 14px;
  }

  .device {
    padding: 8px 0;
    border-bottom: 1px solid var(--lineq);
  }

  .what {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-size: 13px;
    color: var(--hi);
  }

  .dim {
    font-size: 11.5px;
    color: var(--dim);
  }

  .head {
    margin-top: 10px;
  }

  .none {
    margin-top: -6px;
  }

  .qr {
    display: flex;
    gap: 18px;
    align-items: center;
    padding: 12px;
    background: var(--card);
    border: 1px solid var(--line);
  }

  .code {
    width: 220px;
    height: 220px;
    flex-shrink: 0;
    background: #fff;
  }

  .code :global(svg) {
    width: 100%;
    height: 100%;
  }

  .qrside {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .left {
    font-size: 12px;
    color: var(--warn);
  }

  .link {
    width: 100%;
    height: 28px;
    padding: 0 8px;
    font-size: 10.5px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--dim);
  }

  .btn.sm {
    height: 26px;
    padding: 0 12px;
    align-self: flex-start;
  }

  .allbtn {
    align-self: flex-start;
  }

  .danger {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .warn {
    font-size: 12px;
    color: var(--warn);
  }

  .note {
    font-size: 12px;
    line-height: 1.6;
    color: var(--dim);
  }
</style>
