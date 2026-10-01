<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t, type Key } from "./i18n.svelte";
  import PhoneQr from "./PhoneQr.svelte";

  /**
   * Settings › Overview: this Divixi, opened in a phone's browser.
   *
   * NOT remote instances. Remote instances is this app driving ANOTHER PC's
   * Divixi; this is opening YOUR OWN on your phone. The two have been confused
   * once already, so they never share a pane, never share a word ("phone
   * access", never "remote"), and each card points at the other by name.
   *
   * The state machine is the backend's (`remote::Step`). This renders `step`
   * and never works it out again from the parts: two owners for one state
   * machine is how a card comes to say "ready" about a machine that is not.
   */

  type Probe = {
    installed: boolean;
    reachable: boolean;
    stopped: boolean;
    logged_in: boolean;
    name: string;
    https: boolean | null;
    peers: number;
    peers_online: number;
    tailnet: string;
    detail: string;
  };
  type Serve = { published: boolean | null; port_free: boolean | null; detail: string };
  type Step = "install" | "start_tailscale" | "sign_in" | "enable_magicdns" | "enable_https" | "occupied" | "publish" | "ready";
  type Status = {
    on: boolean;
    step: Step;
    address: string;
    probe: Probe;
    serve: Serve;
    awake: "awake" | "unsupported" | "refused" | null;
    port: number;
    running: boolean;
    listen_all: boolean;
    alone: boolean;
    days: number;
  };

  let status = $state<Status | null>(null);
  let busy = $state(false);
  let error = $state("");
  let copied = $state(false);
  let showQr = $state(false);

  /**
   * `fresh` asks Tailscale again instead of reusing the last answer. The
   * Check-again button means it; a render on the way into settings does not,
   * and each read is two subprocesses.
   */
  async function refresh(fresh = false) {
    try {
      status = await invoke<Status>("phone_status", { fresh });
    } catch (err) {
      store.lastError = String(err);
    }
  }

  onMount(() => void refresh());

  async function run(f: () => Promise<Status>) {
    error = "";
    busy = true;
    try {
      status = await f();
    } catch (err) {
      // `phone_set` rejects with the daemon's own words. They are the whole
      // point of the failure, so they are shown rather than summarised.
      error = String(err);
      await refresh(true);
    } finally {
      busy = false;
    }
  }

  const turn = (on: boolean) =>
    run(async () => {
      const next = await invoke<Status>("phone_set", { on });
      // A code for an address that is no longer published opens nothing.
      if (!on) showQr = false;
      return next;
    });

  /** The span the next code grants. Links already out keep the span they had. */
  async function setDays(days: number) {
    try {
      await invoke("phone_set_days", { days });
      await refresh();
    } catch (err) {
      store.lastError = String(err);
    }
  }

  /**
   * Steps Divixi cannot act on itself: Tailscale is missing, signed out, or a
   * tailnet-wide setting is off. Each opens Tailscale's own page for that one
   * thing. The alternative was printing a command for the human to type, and
   * Kiro Crew does not do that to people.
   */
  const HELP: Partial<Record<Step, { url: string; label: Key }>> = {
    install: { url: "https://tailscale.com/download", label: "phone.help.install" },
    start_tailscale: { url: "https://tailscale.com/kb/1080/cli#status", label: "phone.help.startTailscale" },
    sign_in: { url: "https://tailscale.com/kb/1080/cli#login", label: "phone.help.signIn" },
    enable_magicdns: { url: "https://tailscale.com/kb/1081/magicdns", label: "phone.help.enableMagicdns" },
    enable_https: { url: "https://tailscale.com/kb/1153/enabling-https", label: "phone.help.enableHttps" },
  };
  const help = $derived(status ? HELP[status.step] : undefined);
  const openHelp = (url: string) => invoke("open_url", { url }).catch((err) => (store.lastError = String(err)));

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    copied = true;
    setTimeout(() => (copied = false), 1600);
  }

  /** One line per step: what is in the way, and what to do about it. */
  const STEP_BLURB: Record<Step, Key> = {
    install: "phone.step.install",
    start_tailscale: "phone.step.startTailscale",
    sign_in: "phone.step.signIn",
    enable_magicdns: "phone.step.enableMagicdns",
    enable_https: "phone.step.enableHttps",
    occupied: "phone.step.occupied",
    publish: "phone.step.publish",
    ready: "phone.step.ready",
  };

  const s = $derived(status);
  /** Only `publish` earns the button: every other step has something in the way. */
  const canTurnOn = $derived(s?.step === "publish");
  const on = $derived(s?.step === "ready");
</script>

{#if s}
  <div class="card">
    <div class="cardhead">
      <span class="ctitle">{t("phone.title")}</span>
      <span class="state" class:on>{on ? t("phone.on") : t("phone.off")}</span>
      <span class="grow"></span>
      {#if on}
        <button class="btn btn-acc" disabled={busy} onclick={() => (showQr = !showQr)}>
          {showQr ? t("phone.qr.hide") : t("phone.showQr")}
        </button>
        <button class="btn" disabled={busy} onclick={() => turn(false)}>{t("phone.turnOff")}</button>
      {:else if canTurnOn}
        <button class="btn btn-acc" disabled={busy} onclick={() => turn(true)}>{t("phone.turnOn")}</button>
      {/if}
      <button class="btn" disabled={busy} onclick={() => refresh(true)} title={t("phone.checkHint")}>{t("phone.check")}</button>
    </div>

    <div class="body">
      <p class="blurb">{t(STEP_BLURB[s.step])}</p>

      <!-- The daemon's own words, whenever it said anything. Our step is a
           classification; this is what Tailscale actually reported. -->
      {#if s.probe.detail && s.step !== "ready"}
        <p class="said mono">{s.probe.detail}</p>
      {/if}
      {#if s.step === "occupied" && s.serve.detail}
        <p class="said mono">{s.serve.detail}</p>
      {/if}

      {#if s.address}
        <div class="line">
          <span class="addr mono">{s.address}</span>
          <button class="btn sm" onclick={() => copy(s.address)}>{copied ? t("phone.copied") : t("phone.copy")}</button>
        </div>
      {/if}

      <!-- The cost of being on, stated where the switch is -- and only while
           Divixi is actually holding the machine awake. Published by hand, it
           holds nothing, and a promise nobody is keeping is worse than none. -->
      {#if s.awake === "awake" || s.awake === "unsupported"}
        <p class="hint">
          {s.awake === "awake" ? t("phone.awake") : t("phone.awakeUnsupported")}
        </p>
      {/if}

      <!-- Publishing succeeds on a tailnet of one and the address looks right,
           and then nothing can open it. Said before that happens, not after. -->
      {#if s.alone}
        <p class="warn">{t("phone.alone")}</p>
      {/if}

      {#if on}
        <!-- Said where the code is made, and the same number the token
             carries: `phone_pair_link` signs in whatever this says. -->
        <div class="line">
          <span class="hint">{t("phone.days")}</span>
          <div class="seg" role="radiogroup" aria-label={t("phone.days")}>
            {#each [1, 7, 30] as d (d)}
              <button class="segopt" class:on={s.days === d} role="radio" aria-checked={s.days === d} onclick={() => setDays(d)}>
                {t("phone.daysN", { n: String(d) })}
              </button>
            {/each}
          </div>
        </div>
      {/if}

      {#if showQr && on}
        <PhoneQr onhide={() => (showQr = false)} />
      {/if}

      {#if error}
        <p class="warn">{error}</p>
      {/if}

      <!-- Only where Divixi cannot act for itself. Tailscale's own page for
           the one thing that is missing, not a command to type. -->
      {#if help}
        <div class="line">
          <button class="btn sm" onclick={() => openHelp(help!.url)}>{t(help.label)}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .card {
    border: 1px solid var(--line);
    background: var(--card);
    margin-bottom: 12px;
  }

  .cardhead {
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

  .state {
    font-size: 12px;
    color: var(--lab);
  }

  .state.on {
    color: var(--ok);
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

  .blurb {
    margin: 0;
    font-size: 13px;
    line-height: 1.55;
    color: var(--txt);
  }

  .addr {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--hi);
    overflow-wrap: anywhere;
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--dim);
  }

  .seg {
    display: flex;
    border: 1px solid var(--line);
  }

  .segopt {
    padding: 3px 10px;
    border: 0;
    background: none;
    font: inherit;
    font-size: 12px;
    color: var(--dim);
    cursor: pointer;
  }

  .segopt.on {
    background: var(--line);
    color: var(--hi);
  }

  /* What Tailscale itself said. Set apart from our own words so it is clear
     which is which when the two disagree. */
  .said {
    margin: 0;
    padding: 8px 10px;
    border-left: 2px solid var(--line);
    font-size: 12px;
    line-height: 1.5;
    color: var(--lab);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .warn {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--warn);
    overflow-wrap: anywhere;
  }

</style>
