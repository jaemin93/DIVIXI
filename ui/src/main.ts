import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";
import { ready } from "./lib/ipc.svelte";

// With a remote instance chosen, nothing is asked of it until it is
// reached; the app says why not if it cannot be.
void ready().then((ok) => {
  if (!ok) return;
  connectEvents().catch((err) => {
    store.lastError = String(err);
  });
  connectKnowledge().catch((err) => {
    store.lastError = String(err);
  });
  store.restore();
  // The one request divixi makes without being asked: which release is newest,
  // once a launch, at most once a day, and only while the switch in
  // Settings -> About is on (ui/src/lib/updateSchedule.ts). Not awaited and
  // nothing waits on it -- it runs behind whatever is already on screen, and a
  // machine with no network fails quietly where only the settings card shows
  // it.
  void store.checkAtStartup();
});

export default mount(App, { target: document.getElementById("app")! });
