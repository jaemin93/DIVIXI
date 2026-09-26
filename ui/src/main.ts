import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";
import { signedIn } from "./lib/ipc.svelte";

// In a browser on another device, nothing is asked of the app until the
// device is paired; the app shows how to pair it meanwhile.
void signedIn().then((ok) => {
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
