# Lumen

A Dynamic Island–style glass music surface for Windows 11 — a capsule that sits
above the taskbar, expands when a track changes or when you point at it, and
stays out of the way otherwise.

Real DWM Mica. No polling. No dependency on the source app.

---

## Status

**Phase 1 is implemented and running.** Phases 2–4 are scaffolded but not built —
see the roadmap below and `ARCHITECTURE.md` §5.

| | |
|---|---|
| Glass capsule docked above the taskbar | done |
| Expands on hover and on track change | done |
| True Windows 11 Mica (Acrylic → CSS fallback chain) | done |
| Album art, title, artist, progress, play state | done |
| SMTC (`GlobalSystemMediaTransportControlsSessionManager`) | done |
| Smart auto-show / auto-hide | done |
| Hotkeys — <kbd>F5</kbd> prev, <kbd>F6</kbd> play/pause, <kbd>F7</kbd> next | done |
| Accent colour extracted from album art | done |
| Tray icon, portable JSON config | done |
| Single-instance guard (named mutex) | done |
| Wheel-over-taskbar volume, middle-click close, session switcher | Phase 2 |
| Discord Rich Presence, smart pause, drag-to-position | Phase 3 |
| Lyrics, spectrum, share cards | Phase 4 |

## Requirements

- Windows 10 1809+ (Mica needs Windows 11 22H2 / build 22621; below that it
  falls back to Acrylic, and below *that* to a plain translucent panel)
- WebView2 runtime — preinstalled on Windows 11
- To build: Rust 1.85+ (edition 2024), Node 20+, MSVC toolchain

## Build

```bash
npm install
npm run release
```

The portable exe lands in `src-tauri/target/release/lumen.exe`. Copy it anywhere;
it writes `lumen.config.json` next to itself on first run.

Development:

```bash
npm run start
```

`npm run start` is `tauri dev` — it runs Vite and the host together. **Running
`target/debug/lumen.exe` directly will show an empty capsule**: debug builds
point the WebView at the Vite dev server, so the renderer has nothing to load
unless Vite is up. Use `npm run start`, or build release.

Tests:

```bash
cd src-tauri && cargo test     # host: easing, colour, hotkey parsing, AUMID
npm run check                  # renderer: types
npm run check:motion           # host/renderer easing parity
```

## Configuration

`lumen.config.json`, beside the exe. Falls back to `%APPDATA%\Lumen\` if the exe
directory is read-only; the tray tooltip tells you which one is in use.

| Key | Values | Notes |
|---|---|---|
| `shape` | `pill` \| `native` | `pill` is a true capsule; `native` uses an 8 px Windows-11 flyout radius |
| `backdrop` | `auto` \| `mica` \| `acrylic` | `auto` tries Mica, then Acrylic. Applied at launch |
| `theme` | `system` \| `dark` \| `light` | |
| `monitor` | `primary` \| `cursor` | `cursor` re-docks to whichever display the pointer is on |
| `taskbarGap` | px | Gap above the work area |
| `autoExpandOnTrackChange` | bool | |
| `autoExpandMs` | ms | How long the panel stays open after a track change |
| `showWhilePaused` | bool | `false` hides the capsule until playback resumes |
| `hotkeys` | `"F5"`, `"ctrl+alt+n"`, … | Defaults: F5 previous, F6 play/pause, F7 next. Accepts both `Ctrl+KeyA` and `ctrl+a` shorthand |
| `mouse.taskbarWheelVolume` | bool | Wheel over the taskbar changes volume |
| `mouse.taskbarWheelPerApp` | bool | Over an app *button*, changes that app's own volume; over empty bar, the master |
| `mouse.taskbarWheelOverFullscreen` | bool | Keeps the wheel working where the bar would be when a game covers it, targeting the covering app |
| `mouse.taskbarCloseButton` | `middle` \| `right` \| `none` | Which button closes the app under a taskbar button. `right` replaces the jump list |

### Taskbar volume

Scroll over a taskbar button and you move **that application's** volume, not the
system master — the same thing the Volume Mixer slider does. Scroll over an empty
stretch of the bar and you move the master, with the native Windows flyout.

This distinction matters when streaming. Windows applies the master volume at the
very last stage before your speakers, *after* Discord (or OBS) has already
captured the application's audio — so turning the master down makes a game no
quieter for your viewers. The per-app volume is applied upstream of that, which is
why it is the one that works. Ctrl scrolls in coarser steps, Shift in finer ones.

If a borderless full-screen game is covering the taskbar, scrolling where the bar
*would* be still works and targets the game.

> Changing the *defaults* does not change an existing `lumen.config.json` — it
> already has your current bindings written into it. Edit the `hotkeys` block, or
> delete the file to regenerate it.

Hotkeys, position, shape and theme apply immediately. `backdrop` needs a restart.

## How it hits the constraints

**0% CPU at idle — met.** SMTC is event-driven, never polled. The progress bar is
interpolated in a `requestAnimationFrame` loop that exists *only* while the panel
is expanded **and** playing; collapsed or paused, there is no timer anywhere in
the process. The window-shape animator thread is spawned per transition and exits
on its final frame. The equaliser bars animate only during playback.

**Portable single exe — met, 3.78 MB.** `opt-level="z"`, fat LTO, `panic=abort`,
`strip`, and no Tauri plugins. (An NSIS installer is also produced, 1.33 MB, if
you prefer one — the bare exe needs no installation.)

**Idle RAM < 20 MB — not met, and it cannot be with WebView2.** The Rust host
process sits at ~10–14 MB, inside budget. But WebView2 spawns
`msedgewebview2.exe` (browser + GPU + renderer) and that tree is ~55–90 MB no
matter what the app does. Lumen trims what it can — one renderer process, no
out-of-process UI helpers, no background networking, a 48 KB Svelte bundle with
no runtime framework — but the floor is set by Chromium, not by this code.
Getting under 20 MB total means dropping WebView2 for Direct2D/WinUI composition,
which is a different program and costs the CSS-driven animation quality this
design is built on. `ARCHITECTURE.md` §3 has the numbers.

## Two design decisions worth knowing about

**The window is never hidden.** `hide()`/`show()` tears down the DWM backdrop and
Mica visibly re-blooms on every reveal, so the window is parked past the right
edge of the virtual desktop instead. It stays composited and warm.

**The capsule shape is a window region, and window regions are not
anti-aliased.** The OS window is fixed at the expanded size for its whole life
and `SetWindowRgn` clips it to an animated capsule — that keeps real Mica *and* a
true pill *and* a WebView that never resizes (resizing WebView2 at 60 fps
flickers and lags its own contents). The cost is ~1 px of stair-stepping at the
capsule ends. Set `shape: "native"` for an 8 px radius where it is far less
visible. `ARCHITECTURE.md` §4 covers the two alternatives that were rejected.

## Known limitations

- **Capsule ends are not anti-aliased** (~1 px stair-step). Inherent to
  `SetWindowRgn`; `shape: "native"` makes it far less visible. See above.
- **Mixed-DPI multi-monitor.** The window is sized once, at launch, for the DPI
  of the monitor it was created on. With `monitor: "cursor"` and displays at
  different scale factors, the island will be mis-sized on the other display
  until restart. Handling this properly means resizing the host window on
  `WM_DPICHANGED`; it is Phase 2 work.
- **`backdrop` needs a restart.** Every other setting applies live.
- **No outer drop shadow.** A region-clipped window gets no DWM shadow, so the
  capsule uses an inner rim light instead.

## Layout

```
src/                    Svelte 5 renderer — draws, and reports hover. No decisions.
  lib/motion.ts         the shared easing curve
  components/Island.svelte
src-tauri/src/
  lib.rs                composition root — the only file that knows two subsystems
  motion.rs             the same curve, in Rust
  media/                SMTC bridge behind a MediaBackend trait
  window/               HWND, Mica, the animated region, docking
  input/                global hotkeys
  policy.rs             when the island is visible
  color/                album art → accent
```

The host owns visibility, position, shape and timing. The renderer owns pixels.
The two animate off the same curve without sharing a clock — `motion.rs` and
`motion.ts` are asserted equal at 11 sample points by tests on both sides.
