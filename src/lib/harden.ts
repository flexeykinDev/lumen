// Stop the capsule behaving like a web page.
//
// It is one, underneath — a WebView2 with all of a browser's reflexes. Left
// alone it offers "Reload", "Save image as…", "Back", it lets a drag select the
// track title as text, and F5 reloads the whole interface. None of that means
// anything in a 428px music widget, and all of it makes the illusion collapse
// the first time someone right-clicks.
//
// Done here rather than through WebView2 flags because Tauri does not surface
// most of these, and a listener is both exact and easy to reason about.

/** Keys a browser handles that a widget has no use for. */
function isBrowserShortcut(e: KeyboardEvent): boolean {
  const k = e.key.toLowerCase();

  // Reload, in all its spellings.
  if (k === "f5" || ((e.ctrlKey || e.metaKey) && (k === "r" || k === "f5"))) return true;
  // Find, print, save, open, select-all, view-source, zoom.
  if ((e.ctrlKey || e.metaKey) && ["f", "p", "s", "o", "a", "u", "g", "j"].includes(k)) return true;
  if ((e.ctrlKey || e.metaKey) && ["+", "-", "=", "0"].includes(k)) return true;
  // Developer tools. Left available in a debug build, where it is the only way
  // to inspect a window with no chrome.
  if (!import.meta.env.DEV && (k === "f12" || (e.ctrlKey && e.shiftKey && ["i", "j", "c"].includes(k))))
    return true;
  // History navigation: nothing to go back to, and it would unload the app.
  if (e.altKey && (k === "arrowleft" || k === "arrowright")) return true;
  if (k === "browserback" || k === "browserforward" || k === "browserrefresh") return true;

  return false;
}

/**
 * Whether an element is one the user is legitimately typing in.
 *
 * There are none today, but the settings panel will have them, and silently
 * eating Ctrl+A in a text field would be a genuinely annoying bug to track down
 * later.
 */
function isTextEntry(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function hardenWebview(): void {
  // No context menu. There is nothing on it a music capsule can honour.
  window.addEventListener("contextmenu", (e) => e.preventDefault());

  window.addEventListener(
    "keydown",
    (e) => {
      if (isTextEntry(e.target)) return;
      if (isBrowserShortcut(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
    },
    // Capture, so nothing downstream sees a reload it might act on first.
    { capture: true },
  );

  // Dropping a file onto a WebView navigates to it, replacing the interface
  // with whatever was dropped. There is no way back from that short of a
  // restart, so both halves of the gesture are refused.
  for (const type of ["dragover", "drop"] as const) {
    window.addEventListener(type, (e) => e.preventDefault());
  }

  // Middle-click paste on Linux and autoscroll on Windows both start from
  // auxclick; neither is wanted, and the capsule uses middle-click itself.
  window.addEventListener("auxclick", (e) => {
    if (e.button === 1) e.preventDefault();
  });

  // A link that somehow gets clicked must not navigate the only window there is.
  window.addEventListener("click", (e) => {
    const anchor = (e.target as HTMLElement | null)?.closest?.("a");
    if (anchor) e.preventDefault();
  });
}
