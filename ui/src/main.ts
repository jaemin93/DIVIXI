import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";
import { arrive, ready, inTauri, instance, overWeb } from "./lib/ipc.svelte";
import { step } from "./lib/startup.svelte";
import { DEADLINE_MS } from "./lib/startup";
import { t } from "./lib/i18n.svelte";

// A phone's browser, or any browser: the styles that only it needs key off
// this (tokens.css: fields at 16px, so iOS does not zoom in on them).
if (overWeb) document.documentElement.dataset.web = "";

// Until the tracks are in, the page says it is connecting rather than
// showing an empty app (startup.ts). Not for ever: past the deadline a
// remote instance shows why it is not there.
const deadline = setTimeout(() => step({ type: "deadline", error: t("instances.timeout", { seconds: DEADLINE_MS / 1000 }) }), DEADLINE_MS);

// In a phone's browser, a pairing token in the address bar is redeemed and
// taken back out before anything is asked of the server -- otherwise the
// first request goes out unsigned and the app looks signed out for a moment
// after a scan that worked.
void arrive()
  .then(ready)
  .then(async (ok) => {
    if (!ok) {
      step({ type: "failed", error: instance.error });
      return;
    }
    step({ type: "connected" });
    connectEvents().catch((err) => {
      store.lastError = String(err);
    });
    connectKnowledge().catch((err) => {
      store.lastError = String(err);
    });
    const restoring = store.restore();
    // The one request divixi makes without being asked: which release is
    // newest, once a launch, at most once a day, and only while the switch in
    // Settings -> About is on (ui/src/lib/updateSchedule.ts). Not awaited and
    // nothing waits on it. Only in the app's own window: a phone's browser has
    // no installer to update.
    if (inTauri) void store.checkAtStartup();
    const loaded = await restoring;
    step({ type: "loaded", ok: loaded, error: store.lastError });
  })
  .catch((err) => step({ type: "failed", error: String(err) }))
  .finally(() => clearTimeout(deadline));

export default mount(App, { target: document.getElementById("app")! });
