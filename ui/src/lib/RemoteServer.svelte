<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel } from "./time";
  import { RECENT, recentDevices } from "./deviceList";

  /**
   * Settings › Remote instances, this PC's side: its GitHub account (who
   * the user is, to instances reached at an address), and this Divixi as a
   * remote instance for other PCs, which lets in its owner: the same
   * GitHub account.
   */
  type Account = { client_id: string; login: string; owner: string };
  type Device = { id: string; name: string; last_seen: number; login?: string | null; scope: "full" | "conversation" };
  type Status = { enabled: boolean; running: boolean; port: number; all: boolean; addresses: string[]; owner: string; devices: Device[] };
  type Code = { user_code: string; verification_uri: string; expires_in: number };

  let account = $state<Account | null>(null);
  let status = $state<Status | null>(null);
  let clientId = $state("");
  let code = $state<Code | null>(null);
  let port = $state(7488);
  let error = $state("");
  let busy = $state(false);
  /** Every signed-in device listed, not only the few seen most lately. */
  let allDevices = $state(false);
  const devices = $derived(recentDevices(status?.devices ?? [], allDevices));

  onMount(async () => {
    try {
      account = await invoke<Account>("github_account");
      clientId = account.client_id;
      status = await invoke<Status>("remote_server_status");
      port = status.port;
    } catch (err) {
      store.lastError = String(err);
    }
  });

  async function run(f: () => Promise<void>) {
    error = "";
    busy = true;
    try {
      await f();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  const signIn = () =>
    run(async () => {
      if (clientId.trim() !== account?.client_id) account = await invoke<Account>("github_set_client_id", { clientId });
      code = await invoke<Code>("github_login_start");
      try {
        account = await invoke<Account>("github_login_wait");
      } finally {
        code = null;
      }
      status = await invoke<Status>("remote_server_status");
    });

  const signOut = () =>
    run(async () => {
      account = await invoke<Account>("github_logout");
      status = await invoke<Status>("remote_server_status");
    });

  const serve = (enabled: boolean, all: boolean) =>
    run(async () => {
      status = await invoke<Status>("remote_server_set", { enabled, all, port: Number(port) || 7488 });
      port = status.port;
    });

  const drop = (device: string) =>
    run(async () => {
      status = await invoke<Status>("remote_server_drop", { device });
    });

  /** The lost-phone button: one write ends every token this Divixi issued. */
  let confirming = $state(false);
  const dropAll = () =>
    run(async () => {
      status = await invoke<Status>("remote_server_drop_all");
      confirming = false;
    });
</script>

{#if account && status}
  <section class="card">
    <div class="chead"><span class="ctitle">{t("gh.title")}</span></div>
    <div class="body">
      {#if account.login}
        <div class="line">
          <span class="dot on"></span>
          <span class="who">{t("gh.signedIn", { login: account.login })}</span>
          <span class="grow"></span>
          <button class="btn sm" disabled={busy} onclick={signOut}>{t("gh.signOut")}</button>
        </div>
        <p class="hint">{t("gh.why")}</p>
      {:else if code}
        <p class="hint">{t("gh.enter")}</p>
        <div class="code mono">{code.user_code}</div>
        <p class="hint mono">{code.verification_uri}</p>
      {:else}
        <label>
          <span class="lab">{t("gh.clientId")}</span>
          <input class="mono" bind:value={clientId} placeholder="Ov23li…" spellcheck="false" autocomplete="off" />
          <span class="hint">{t("gh.clientIdHint")}</span>
        </label>
        <div class="line">
          <button class="btn btn-acc" disabled={busy || !clientId.trim()} onclick={signIn}>{t("gh.signIn")}</button>
        </div>
        <p class="hint">{t("gh.why")}</p>
      {/if}
    </div>
  </section>

  <section class="card">
    <div class="chead">
      <span class="ctitle">{t("serve.title")}</span>
      <span class="grow"></span>
      <span class="state" class:on={status.running}>{status.running ? t("serve.on", { port: status.port }) : t("serve.off")}</span>
      <button class="btn sm" class:btn-acc={!status.enabled} disabled={busy} onclick={() => serve(!status!.enabled, status!.all)}>
        {status.enabled ? t("serve.turnOff") : t("serve.turnOn")}
      </button>
    </div>
    <div class="body">
      <p class="hint">{t("serve.note")}</p>
      <div class="two">
        <div class="seg" role="radiogroup" aria-label={t("serve.where")}>
          <button class="segopt" class:on={!status.all} role="radio" aria-checked={!status.all} disabled={busy} onclick={() => serve(status!.enabled, false)}>{t("serve.local")}</button>
          <button class="segopt" class:on={status.all} role="radio" aria-checked={status.all} disabled={busy} onclick={() => serve(status!.enabled, true)}>{t("serve.all")}</button>
        </div>
        <label class="port">
          <span class="lab">{t("remote.port")}</span>
          <input class="mono" type="number" min="1024" max="65535" bind:value={port} onchange={() => serve(status!.enabled, status!.all)} />
        </label>
      </div>
      <p class="hint">{status.all ? t("serve.allHint") : t("serve.localHint")}</p>
      {#if status.all}
        <div class="addrs mono">
          {#each status.addresses as a (a)}<span>http://{a}:{status.port}</span>{/each}
        </div>
      {/if}
      {#if status.owner}
        <p class="hint">{t("serve.owner", { login: status.owner })}</p>
      {:else}
        <p class="warn">{t("serve.noOwner")}</p>
      {/if}

      {#if status.devices.length}
        <div class="lab devhead">{t("serve.devices", { n: status.devices.length })}</div>
        {#each devices.shown as d (d.id)}
          <div class="line dev">
            <div class="what">
              <div class="who">{d.name}</div>
              <!-- How it came in, because that is what decides what it may
                   do: a phone gets the conversation, the rest get the machine. -->
              <div class="hint mono">
                {t("serve.lastSeen", { when: whenLabel(d.last_seen * 1000, store.now, store.lang) })}
                {#if d.scope === "conversation"}· {t("serve.viaPhone")}{:else if d.login}· GitHub {d.login}{:else}· SSH{/if}
              </div>
            </div>
            <button class="btn sm" disabled={busy} onclick={() => drop(d.id)}>{t("serve.drop")}</button>
          </div>
        {/each}
        <!-- The few seen most lately; the rest a press away, and back. -->
        {#if devices.hidden || allDevices}
          <button class="more" aria-expanded={allDevices} onclick={() => (allDevices = !allDevices)}>
            {allDevices ? t("serve.devicesFewer", { n: RECENT }) : t("serve.devicesAll", { n: status.devices.length })}
          </button>
        {/if}
        <div class="line">
          {#if confirming}
            <span class="hint">{t("serve.dropAllSure")}</span>
            <button class="btn sm" disabled={busy} onclick={dropAll}>{t("serve.dropAllYes")}</button>
            <button class="btn sm" disabled={busy} onclick={() => (confirming = false)}>{t("serve.dropAllNo")}</button>
          {:else}
            <button class="btn sm" disabled={busy} onclick={() => (confirming = true)}>{t("serve.dropAll")}</button>
          {/if}
        </div>
        <p class="hint">{t("serve.dropAllNote")}</p>
      {/if}
    </div>
  </section>

  {#if error}<p class="err mono">{error}</p>{/if}
{/if}

<style>
  .card {
    border: 1px solid var(--line);
    background: var(--card);
    margin-bottom: 12px;
  }

  .chead {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 18px;
    border-bottom: 1px solid var(--line);
  }

  .ctitle {
    font-size: 14px;
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 18px 16px;
  }

  .line {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .who {
    font-size: 13px;
    color: var(--hi);
  }

  .what {
    flex: 1;
    min-width: 0;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--idle);
  }

  .dot.on {
    background: var(--ok);
  }

  .state {
    font-size: 12px;
    color: var(--lab);
  }

  .state.on {
    color: var(--ok);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .lab {
    font-size: 12.5px;
    color: var(--txt);
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--dim);
  }

  .warn {
    margin: 0;
    font-size: 12px;
    color: var(--warn);
  }

  input {
    height: 34px;
    padding: 0 10px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 13px;
  }

  input:focus {
    border-color: var(--acc);
  }

  .code {
    align-self: flex-start;
    padding: 10px 16px;
    border: 1px solid var(--acc);
    font-size: 22px;
    letter-spacing: 0.18em;
    color: var(--hi);
  }

  .two {
    display: flex;
    align-items: flex-end;
    gap: 16px;
  }

  .port input {
    width: 110px;
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--line);
  }

  .segopt {
    height: 34px;
    padding: 0 14px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    border-bottom: 2px solid transparent;
    color: var(--dim);
    font-size: 12.5px;
  }

  .segopt:last-child {
    border-right: 0;
  }

  .segopt.on {
    color: var(--hi);
    background: var(--sel);
    border-bottom-color: var(--acc);
  }

  .addrs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    font-size: 12px;
    color: var(--txt);
    user-select: text;
  }

  .devhead {
    margin-top: 6px;
  }

  .dev {
    padding: 6px 0;
    border-top: 1px solid var(--lineq);
  }

  .dev .who {
    overflow-wrap: anywhere;
  }

  .more {
    display: block;
    width: 100%;
    padding: 8px 0;
    background: none;
    border: 0;
    border-top: 1px solid var(--lineq);
    font-size: 12px;
    color: var(--dim);
    text-align: left;
    cursor: pointer;
  }

  .more:hover {
    color: var(--hi);
  }

  .btn.sm {
    height: 28px;
    padding: 0 12px;
  }

  .err {
    margin: 0 0 12px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--deltx);
    white-space: pre-wrap;
  }
</style>
