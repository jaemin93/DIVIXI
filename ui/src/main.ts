import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";
import { arrive, ready, inTauri } from "./lib/ipc.svelte";

// In a phone's browser, a pairing token in the address bar is redeemed and
// taken back out before anything is asked of the server -- otherwise the
// first request goes out unsigned and the app looks signed out for a moment
// after a scan that worked.
void arrive()
  .then(ready)
  .then((ok) => {
    if (!ok) return;
    connectEvents().catch((err) => {
      store.lastError = String(err);
    });
    connectKnowledge().catch((err) => {
      store.lastError = String(err);
    });
    store.restore();
    // The one request divixi makes without being asked: which release is
    // newest, once a launch, at most once a day, and only while the switch in
    // Settings -> About is on (ui/src/lib/updateSchedule.ts). Not awaited and
    // nothing waits on it. Only in the app's own window: a phone's browser has
    // no installer to update.
    if (inTauri) void store.checkAtStartup();
  });

export default mount(App, { target: document.getElementById("app")! });
