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
});

export default mount(App, { target: document.getElementById("app")! });
