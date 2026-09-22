import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents, store } from "./lib/store.svelte";

connectEvents();
store.restore();

export default mount(App, { target: document.getElementById("app")! });
