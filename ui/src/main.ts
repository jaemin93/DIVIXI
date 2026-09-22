import { mount } from "svelte";
import "./lib/tokens.css";
import App from "./App.svelte";
import { connectEvents } from "./lib/store.svelte";

connectEvents();

export default mount(App, { target: document.getElementById("app")! });
