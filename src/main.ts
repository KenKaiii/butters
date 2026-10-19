import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";

const root = document.documentElement;
// macOS gets a transparent window with our own soft corners and shadow.
if (/Mac/.test(navigator.platform) || /Macintosh/.test(navigator.userAgent)) root.classList.add("floating");

// Called by Butters right before it closes the panel, so it can melt away.
Object.defineProperty(window, "__buttersLeave", {
  value: () => root.classList.add("leaving"),
  writable: false,
});

// The panel is a utility surface: no browser context menu.
window.addEventListener("contextmenu", (e) => e.preventDefault());

export default mount(App, { target: document.getElementById("app")! });
