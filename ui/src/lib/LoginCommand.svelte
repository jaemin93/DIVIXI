<script lang="ts">
  import { instanceId, invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { commandFromHere } from "./agentLogin";

  /** The command that logs this agent in again, ready to paste. */
  let { agent }: { agent: string } = $props();

  const status = $derived(store.agents?.find((a) => a.kind === agent));

  /** How this PC reaches the instance shown, when it is over ssh. */
  let ssh = $state<string | null>(null);
  $effect(() => {
    if (!instanceId) return;
    invoke<{ id: string; ssh: string }[]>("remote_hosts")
      .then((hosts) => (ssh = hosts.find((h) => h.id === instanceId)?.ssh || null))
      .catch(() => {});
  });

  const command = $derived(commandFromHere(status?.login_command || status?.login_hint || "", ssh));

  let copied = $state(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (err) {
      store.lastError = String(err);
    }
  }
</script>

{#if command}
  <div class="login">
    <code class="mono">{command}</code>
    <button class="btn" onclick={copy}>{copied ? t("agents.copied") : t("agents.copy")}</button>
  </div>
{/if}

<style>
  .login {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  code {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
    font-size: 12px;
  }
  button {
    flex: none;
  }
</style>
