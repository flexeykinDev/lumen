import { mount } from "svelte";
import App from "./App.svelte";
import { hardenWebview } from "./lib/harden";
import "./app.css";

// Before the app mounts: a right-click or a stray F5 during startup would be
// handled by the WebView's own defaults otherwise.
hardenWebview();

const target = document.getElementById("app");
if (!target) throw new Error("#app is missing from index.html");

export default mount(App, { target });
