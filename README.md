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

### Spectrum

Sixteen log-spaced bands behind the expanded panel, from a WASAPI loopback
capture of whatever is playing. `"spectrum": { "enabled": true }`.

This is the **only** feature here that costs CPU while it runs, so it is gated
to expanded **and** playing — when the bars are not on screen the capture thread
does not exist and the audio endpoint is closed. Measured on this machine:

| state | cost |
|---|---|
| not visible | **0.000%** — no thread, no device |
| visible and playing | **0.043% of the machine** (0.52% of one core), 19.7 frames/s |

Re-measure after any change to it:

```bash
cargo test -- --ignored --nocapture spectrum_cost
```

Loopback on the render endpoint, so it hears the mix you hear and needs no
microphone permission. The FFT is hand-written — 1024 points twenty times a
second is microseconds of work, and `rustfft` would cost more in binary size
than it saves in time.

### Lyrics

Time-synced lyrics from [LRCLIB](https://lrclib.net). The current line replaces
the artist name in the expanded panel and follows the song.

**Off by default, and this is the one setting worth reading before you flip it.**
It is the only feature in Lumen that touches the network. Enabling it sends the
**artist, title, album and duration** of everything you play to `lrclib.net`. No
account, no identifier, nothing else — but it is a third party learning your
listening, and that is your call to make, not a default to inherit.

```json
"lyrics": { "enabled": true }
```

The active line sweeps left to right as it is sung.

```json
"lyrics": { "enabled": true, "geniusFallback": true, "estimatedOffsetMs": 0 }
```

**Only songs are looked up.** A lyrics database matches on artist and title, so
asking it about a gameplay video or a ninety-minute interview does not politely
fail — it returns *someone's* song and scrolls those words over unrelated audio.
Dedicated music players (Spotify, Yandex Music, YouTube Music, foobar2000…) are
trusted outright; a browser has to look like music first, judged on duration and
on title keywords in both English and Russian. A missed lyric is a shrug; a
confident lyric over the wrong audio looks broken.

**Three sources, each worse than the last:**

| source | timings | shown as |
|---|---|---|
| LRCLIB synced `.lrc` | real, per line | normal |
| LRCLIB plain text | **guessed** | italic, dimmer |
| Genius page | **guessed** | italic, dimmer |

Guessed timings spread the lines across the track in proportion to their length,
skipping a little intro and outro. They still drift: a lyrics page writes a
chorus once where it is sung three times, so the line count rarely matches what
is actually performed. That is why estimated lyrics are styled differently
rather than presented with the confidence of a measurement — and why
`estimatedOffsetMs` exists. If they consistently run late, try `-1500`; it
shifts guessed timings only and never touches a real `.lrc`.

**`geniusFallback` is a scraper, not an API.** Genius has no lyrics endpoint and
their terms prohibit serving lyrics through the API, so this reads their web
page. It depends on markup that can change without notice and it will break
sometimes; set it to `false` if you would rather it did not. There is a live
test for exactly this reason:

```bash
cargo test -- --ignored genius
```

One request per track, never per line. The whole timed lyric arrives in a single
event, and the sweep is a CSS animation with a negative delay — so there is no
polling, no per-frame JavaScript, and exactly one timer, scheduled for the
moment the next line begins. Pausing freezes the sweep; `prefers-reduced-motion`
turns it off entirely.

Tracks with no lyrics anywhere simply leave the artist name showing.

### Share cards

Tray → **Copy share card** renders a 1200×600 image of what is playing — cover,
title, artist, progress, source — puts it on the clipboard, and saves a copy to
`%USERPROFILE%\Pictures\Lumen\`.

It does not open a folder or pop a notification. The menu says copy, so it
copies; paste it straight into Discord.

The card is drawn on a `<canvas>` in the WebView rather than in Rust, because
that side already has the fonts, the decoded artwork and the accent colour. Rust
would need a font rasteriser and a second copy of the layout to maintain.

The progress bar only appears when the source reports a duration. Live streams
and some browser sources do not, and a full-width bar would be a lie.

### Smart pause

Pauses playback when you lock the machine, and starts it again when you come
back. `"smartPause": { "enabled": true, "resumeOnUnlock": true }`.

It only ever resumes **its own** pause. If you stopped the music yourself before
locking, or something else is playing when you return, it leaves things alone —
coming back to music you deliberately stopped is worse than coming back to
silence.

It does not react to full-screen games. Windows reports that through
`SHQueryUserNotificationState`, which has no event and would mean polling
forever for something that happens a few times a day; lock and unlock arrive as
real messages, so this costs nothing while nothing is happening.

### Discord Rich Presence

Shows the current track on your Discord profile as *Listening to …*, with the
title, artist, album and a live progress clock. It clears when playback stops.

It needs a Discord application id, and there is deliberately no default:
presence is published *as* an application, so the name and icon everyone sees
belong to whoever created it. Baking in a shared one would put a stranger's
branding on your profile.

1. Create an app at <https://discord.com/developers/applications>. **Its name is
   what renders after "Listening to"** — call it Lumen.
2. Copy the Application ID from General Information.
3. Under Rich Presence → Art Assets, upload an image named `lumen`.
4. Put the id in `lumen.config.json`:

```json
"discord": { "enabled": true, "applicationId": "your-id-here", "showWhilePaused": false }
```

**Album art is the app icon, not the cover.** Windows hands the cover to Lumen
as raw bytes with no URL anywhere, and Discord will only display an image it can
fetch — an uploaded asset or a link. Showing the real cover would mean uploading
every track's artwork to some third-party host, which is not a thing a local
music widget should do quietly. The field is wired for the day a source provides
a URL directly.

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

**Idle RAM < 20 MB — not met, and it cannot be with WebView2.** Measured on the
release build, idle with a track loaded: `lumen.exe` is 36 MB, and the six
`msedgewebview2.exe` children it spawns bring the tree to **405–476 MB working
set / ~203 MB private**. An earlier version of this note guessed "~55–90 MB";
that was optimistic by four to five times, and the measured figures are the ones
to plan against. Lumen trims what it can — one renderer process, no
out-of-process UI helpers, no background networking, a 59 KB Svelte bundle with
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
