// Entry point for the settings window.
//
// A separate document from the capsule, not a route inside it: the island is a
// 428px always-on-top widget that must stay cheap, and loading a full settings
// UI into it would put every one of these components in the process that has to
// stay at zero cost while idle.

import { mount } from "svelte";
import Settings from "./components/Settings.svelte";
import { hardenWebview } from "./lib/harden";
import "./app.css";
import "./settings.css";

hardenWebview();

const target = document.getElementById("settings");
if (!target) throw new Error("#settings is missing from settings.html");

export default mount(Settings, { target });
