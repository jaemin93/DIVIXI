import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";
import { connectKnowledge } from "./lib/knowledge.svelte";

connectEvents().catch((err) => {
  store.lastError = String(err);
});
connectKnowledge().catch((err) => {
  store.lastError = String(err);
});
store.restore();

export default mount(App, { target: document.getElementById("app")! });
