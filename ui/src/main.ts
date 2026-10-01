import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";
import { arrive, ready } from "./lib/ipc.svelte";

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
  });

export default mount(App, { target: document.getElementById("app")! });
