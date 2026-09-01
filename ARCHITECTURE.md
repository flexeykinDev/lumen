# Lumen — Architecture

A Dynamic Island–style glass music surface for Windows 11.
Tauri 2 + Rust (edition 2024) host, Svelte 5 + TypeScript renderer.

---

## 1. Crate selection (and why)

| Concern | Crate | Why this one |
|---|---|---|
| Shell | `tauri` 2.x | Required. WebView2 host, tray, IPC. |
| Backdrop | `window-vibrancy` 0.6 | Only maintained crate that wraps `DWMWA_SYSTEMBACKDROP_TYPE` (Mica) *and* `SetWindowCompositionAttribute` (Acrylic) with correct build-number gating. |
| SMTC | `windows` (`Media_Control`, `Storage_Streams`) | Direct WinRT. `smtc-suite` is a thin wrapper that adds a dependency, its own runtime, and lags the `windows` release train. We need `SessionsChanged` / `MediaPropertiesChanged` / `PlaybackInfoChanged` / `TimelinePropertiesChanged` event registration, which is ~200 lines direct and costs nothing in binary size. |
| Hotkeys | `global-hotkey` 0.7 | tauri-apps' own crate. Used **directly**, not via `tauri-plugin-global-shortcut` — the plugin adds an IPC hop and a permission surface we don't need. |
| Mouse hooks | `windows` (`WH_MOUSE_LL`) | No crate abstracts this well. Needs its own thread with a real message pump. |
| Volume | `windows` (`IAudioEndpointVolume`) | Core Audio. No wrapper crate justifies its weight. |
| Discord | `discord-rich-presence` | Pure-Rust IPC over the named pipe. No Discord game SDK blob, no C deps. `discord-presence` is the runner-up; `discord-sdk` drags in a native library and blows the size budget. |
| Album art → accent | `color_thief` + `image` (png/jpeg only) | MMCQ quantization, ~200 LOC of dependency. `kmeans_colors` looks better but pulls `palette` + `rayon`. We post-process color_thief output in HSL for vibrancy instead. |
| Config | `serde` + `serde_json` | Portable JSON next to the exe. |
| Async | `tokio` (rt-multi-thread, sync, time) | Already in the Tauri tree. Used for the SMTC actor and the RPC actor. |
| Logging | `tracing` + `tracing-subscriber` (env-filter off by default) | Compiled out below `info` in release. |

**Explicitly not used:** `souvlaki` (publishes SMTC, doesn't consume it), `tauri-plugin-*` for anything the host can do in-process.

---

## 2. Module graph

```
src-tauri/src/
├── main.rs              # thin: sets subsystem, calls lumen::run()
├── lib.rs               # composition root — builds every actor, wires channels
├── motion.rs            # SHARED easing (must stay identical to src/lib/motion.ts)
├── config.rs            # portable JSON config: load / save / defaults
├── single_instance.rs   # named-mutex guard (a loose exe gets double-clicked)
├── util.rs              # block_on_timeout — the whole executor the SMTC thread needs
├── policy.rs            # when the island is visible
├── ipc.rs               # #[tauri::command] surface + typed event emitters
│
├── media/
│   ├── mod.rs           # MediaBackend trait, MediaEvent, TransportCmd
│   ├── model.rs         # TrackInfo, PlaybackState, Timeline, SessionId
│   └── smtc.rs          # WinRT GlobalSystemMediaTransportControlsSessionManager
│
├── window/
│   ├── mod.rs           # IslandHandle facade
│   ├── island.rs        # state machine: Hidden ⇄ Collapsed ⇄ Expanded, park offscreen
│   ├── backdrop.rs      # Backdrop trait: Mica → Acrylic → CSS fallback chain
│   ├── shape.rs         # animated SetWindowRgn pill region (60 fps, transitions only)
│   └── taskbar.rs       # ABM_GETTASKBARPOS → dock rect on the correct monitor
│
├── input/
│   ├── mod.rs
│   ├── hotkeys.rs       # global-hotkey manager + rebinding
│   └── mouse_hook.rs    # [Phase 2] WH_MOUSE_LL thread
│
├── audio/volume.rs      # [Phase 2] IAudioEndpointVolume
├── presence/mod.rs      # [Phase 3] Discord RPC actor
├── smart_pause/mod.rs   # [Phase 3] SHQueryUserNotificationState
└── color/mod.rs         # album art → Accent { base, fg, glow }
```

Every subsystem is an **actor**: owns its thread/task, receives `Cmd` over an `mpsc`,
emits `Event` over a `broadcast`. `lib.rs` is the only place that knows about more
than one of them. Nothing calls into another module's internals.

### Core interfaces

```rust
// media/mod.rs
pub trait MediaBackend: Send + Sync + 'static {
    fn subscribe(&self) -> broadcast::Receiver<MediaEvent>;
    fn snapshot(&self) -> Option<NowPlaying>;
    fn control(&self, cmd: TransportCmd) -> anyhow::Result<()>;
    fn sessions(&self) -> Vec<SessionSummary>;   // Phase 2: session switcher
    fn focus(&self, id: &SessionId) -> anyhow::Result<()>;
}

pub enum MediaEvent {
    SessionsChanged(Vec<SessionSummary>),
    TrackChanged(Box<NowPlaying>),
    PlaybackChanged(PlaybackState),
    TimelineChanged(Timeline),
    Vanished,
}

// window/backdrop.rs
pub trait Backdrop: Send + Sync {
    fn kind(&self) -> BackdropKind;                 // Mica | Acrylic | None
    fn set_theme(&self, dark: bool);
    fn set_shape(&self, w: i32, h: i32, radius: i32) -> windows::core::Result<()>;
}
```

---

## 3. The three hard constraints, honestly

### `0% CPU when idle` — **met**, measured

Sampled across the whole process tree (host + 6 WebView2 processes) with the
island collapsed and playback paused: **0.000 s of CPU over 12.0 s of wall
clock**. Not "rounds to zero" — the scheduler never ran any of it.

How the design gets there:
- SMTC is **event-driven**, never polled. WinRT `TypedEventHandler`s fire on a
  system thread; we forward into a broadcast channel.
- The progress bar does **not** run a timer. On `TimelineChanged` we store
  `(position, timestamp, rate)` and the renderer interpolates — but only inside a
  `requestAnimationFrame` loop that is **torn down whenever the island is not expanded**.
- CSS animations are transform/opacity only (compositor thread), and only exist
  during the ~340 ms transition.
- The `WH_MOUSE_LL` hook (Phase 2) is callback-driven.
- Verified idle state: zero timers, zero rAF, zero polling.

### `Portable single .exe < 10 MB` — **met**, measured at **3.78 MB**
`opt-level="z"`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true`,
no Tauri plugins, `image` with only `png`+`jpeg` decoders. The NSIS installer,
for anyone who wants one, is 1.33 MB.
Config lives in `lumen.config.json` next to the exe; if that path is read-only we
fall back to `%APPDATA%\Lumen\` and say so in the tray tooltip.

### `Idle RAM < 20 MB` — **not achievable for the process tree, and you should know why**
The Rust host process will sit at **~10–14 MB** — comfortably inside your budget.
But WebView2 spawns `msedgewebview2.exe` (browser + GPU + renderer), and that tree
is **~55–90 MB** no matter what the app does. There is no Tauri configuration that
avoids it; it is the cost of an HTML renderer.

What this project actually does about it:
- `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`,
  `--disable-background-timer-throttling=false`, `--js-flags=--lite-mode`,
  and `--renderer-process-limit=1` passed via `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`.
- Single window, single renderer — the tray menu is native, not HTML.
- No frameworks at runtime: Svelte 5 compiles to ~12 KB of JS, no virtual DOM.

Realistic total: **~70–100 MB working set**, of which Lumen's own code is ~12 MB.
If <20 MB total is a hard requirement rather than a target, the renderer has to be
Direct2D/WinUI composition instead of WebView2 — that is a different project, and
it would cost you the CSS-driven animation quality this design is built on.
Say the word and I'll cost it out.

---

## 4. Why the window never hides, and how the shape works

`hide()`/`show()` destroys and recreates the DWM backdrop — Mica visibly re-blooms
on every show. So `island.rs` **parks the window off-screen** at
`(work_area.right + 64, monitor.top - 4096)` instead. The window is always visible
to DWM, always composited, always warm.

**Shape — and a design that was shipped, was wrong, and got replaced.**

The original implementation kept the window fixed at its expanded size and
clipped it to a capsule with `SetWindowRgn`, specifically so WebView2 would never
have to resize. That is wrong, for a reason worth recording:

> **`SetWindowRgn` does not clip the DWM system backdrop.** Mica and Acrylic are
> composited by DWM across the window's whole rectangle, outside the region clip.
> The region only ever clipped the WebView's own output.

The bug hid behind Mica for the entire first phase. `DWMSBT_MAINWINDOW` is
near-opaque and tinted from the wallpaper, so over a dark desktop the unclipped
slab was nearly the same colour as what it covered. Switching to translucent
Acrylic made it obvious: a 428×148 sheet of glass over the screen with the
capsule drawn inside it.

**Lumen now animates the window rectangle itself** (`window/geometry.rs`), bottom
anchored so the capsule grows upward out of the taskbar without its lower edge
moving. `SetWindowRgn` is not used anywhere.

Consequences, stated plainly:

- WebView2 *does* now resize on every frame of a transition — the cost the region
  was introduced to avoid. Measured impact is in §3.
- **The true pill is gone.** DWM exposes three fixed corner radii and no custom
  value, so `DWMWCP_ROUND` (~8 px, matching a stock Windows 11 flyout) is the
  closest reachable shape. Unlike a region it is anti-aliased, and it rounds the
  backdrop along with the window.
- A real capsule requires a Windows.UI.Composition backdrop with a
  `RoundedRectangleGeometry` clip — arbitrary shape, real Acrylic, and animation
  on the compositor thread. That is the correct long-term answer and remains
  unimplemented.
- A region-clipped window gets no DWM drop shadow, so the capsule uses an inner
  rim light instead.

**Backdrop choice.** `Auto` prefers **Acrylic** (`DWMSBT_TRANSIENTWINDOW`), not
Mica. These are not two flavours of one effect: Mica samples *only the wallpaper*
and deliberately ignores windows behind it, which is right for a large app window
and wrong for a floating capsule. Acrylic samples what is actually behind, which
is what "glass" means here. `backdrop: "mica"` remains available.

**Clock synchronisation.** Rust owns the region animation; the renderer owns the
content animation. They are not driven by the same clock — instead, at transition
start the host emits one event carrying `{ state, duration_ms }` and *both sides
run the identical easing curve*, `cubic-bezier(0.22, 1, 0.36, 1)`. `motion.rs` and
`motion.ts` must stay in lockstep; both are ~20 lines and there is a test asserting
they agree at 11 sample points. This gives perfect visual sync with **zero IPC
traffic during the animation**.

---

## 4b. Taskbar volume: two findings worth keeping

**Per-app volume is a different lever from master volume, not a nicer one.**
Windows applies `ISimpleAudioVolume` (the Volume Mixer) before the audio engine
mixes, and `IAudioEndpointVolume` (the master slider) at the endpoint, last.
Anything capturing an application's sound for a stream or a recording taps it
upstream of the endpoint — so turning the master down quietens your own speakers
and changes nothing for a Discord viewer. Turning the application down does. This
is why `audio::session` exists alongside `audio::volume`.

Sessions are matched to an application by **full executable path**, never by pid.
Measured on this machine: Discord's window belongs to pid 5888 while its two
audio sessions belong to 16036 and 16668 — a pid match would have moved neither.
Browsers, Electron apps and game engines all render audio from child processes
that share the parent's image, so the path catches the whole family, and pid
reuse stops being a hazard at all.

**`IUIAutomation::ElementFromPoint` does not work on the Windows 11 taskbar.** It
returns the bare `Shell_TrayWnd` pane — `ControlType.Pane`, `Name=""` — for every
point along the bar, never descending into the XAML buttons. An implementation
built on it resolves nothing, forever, and is indistinguishable from a feature
that was never wired up; that cost a long detour through a fullscreen game that
happened to be covering the bar at the time and looked like the cause.

What works is entering the tree by handle (`ElementFromHandle` on the taskbar's
own HWND), asking for `Button` descendants server-side, and hit-testing their
bounding rectangles. Button labels are the window title plus a **localised**
suffix (`"KYSLINGO - The Law of Recognition —1 запущенное окно"`), so the suffix
is never parsed: the button is matched to the window whose title the label starts
with, longest first. See `input::taskbar_target`, which has unit tests for the
matcher.

---

## 5. Phase plan

- **Phase 1 (this pass)** — window + Mica + shape animation, SMTC, album art,
  progress, accent extraction, hotkeys, auto show/hide, tray, config.
- **Phase 2** — `WH_MOUSE_LL` (taskbar wheel → volume, middle-click → close,
  Alt+middle → kill), session switcher.
- **Phase 3** — Discord RPC, smart pause, cover crossfade polish, drag-to-position.
- **Phase 4** — lyrics overlay, spectrum, share card.
