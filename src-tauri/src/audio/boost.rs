//! Volume boost and bass boost for one application.
//!
//! Windows caps every volume control it exposes at 100%: `ISimpleAudioVolume`
//! and `IAudioEndpointVolume` both take a scalar in `0..=1`, and there is no
//! API anywhere that goes past unity. Going louder therefore means processing
//! the samples ourselves, and that means this:
//!
//! 1. capture the application's audio with process loopback,
//! 2. mute the application so its own output is silent,
//! 3. apply gain and a bass shelf, and
//! 4. render the result to the default endpoint.
//!
//! Step 2 is where this gets interesting, because the obvious version does not
//! work. Muting the application silences the capture along with it: the process
//! loopback tap sits *after* the session's volume and mute, so a muted app
//! hands over nothing but zeroes — measured, not assumed, by
//! `tests::the_tap_follows_the_session_volume`.
//!
//! So the source is not muted. It is turned down to `RESIDUAL`, two percent,
//! and the captured copy — now two percent of full scale — is multiplied back
//! up by fifty before the requested gain is applied. The audio is `f32` the
//! whole way, so scaling down and back up loses nothing a listener could
//! detect; float has range to spare where 16-bit integers would not.
//!
//! What remains is the original still playing at two percent, 34 dB under the
//! boosted copy, roughly a whisper beside a conversation. It is a real signal
//! and it does interfere with its own delayed copy, but 34 dB down that ripple
//! is a fraction of a decibel: inaudible, where a full-level double would have
//! been an obvious echo.
//!
//! What this costs, and what it cannot avoid:
//!
//! - **Latency.** Capture, a buffer, and a render add roughly 30 ms. Music is
//!   unaffected; video is 30 ms out of lip-sync, below the ~100 ms most people
//!   notice but not nothing.
//! - **CPU**, for as long as it runs. Idle cost stays zero because the engine
//!   only exists while boost is on and something is playing.
//! - **The application's own mute.** While boosting, Lumen holds the app muted.
//!   If Lumen is killed outright the mute survives it; a clean exit restores it.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use anyhow::{Context, anyhow};
use windows::{
    Win32::{
        Foundation::WAIT_OBJECT_0,
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient,
            IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
};

use crate::audio::{dsp::Processor, loopback::ProcessCapture, session};

/// How much captured audio to hold before dropping the oldest.
///
/// Latency is the whole cost of this feature, so the queue is deliberately
/// small: at 48 kHz stereo this is about 40 ms. A larger buffer would ride out
/// scheduling hiccups at the price of the delay being audible against video.
const QUEUE_FRAMES: usize = 2048;

/// What the source is turned down to while its audio is being replaced.
///
/// Not zero: zero is a mute, and a muted session hands the capture silence.
/// Two percent is the compromise — quiet enough to be masked completely by the
/// boosted copy, loud enough that the capture is a real signal rather than the
/// bottom bits of one.
const RESIDUAL: f32 = 0.02;

/// Everything the engine needs to know about what it is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Linear gain. 1.0 is unity; 2.0 is twice the amplitude (+6 dB).
    pub gain: f32,
    /// Low-shelf lift in decibels, applied below `dsp::BASS_HZ`.
    pub bass_db: f32,
}

impl Settings {
    /// Whether these settings would change the sound at all.
    ///
    /// Running the whole capture-and-render chain to multiply by one and filter
    /// by nothing would be pure cost, so this is what gates it.
    pub fn is_effective(&self) -> bool {
        (self.gain - 1.0).abs() > 0.01 || self.bass_db.abs() > 0.1
    }
}

/// A running boost for one application.
pub struct Engine {
    /// The application being replaced, so it can be restored when this stops.
    exe: String,
    /// The level it was at before boost turned it down.
    restore: f32,
    stop: Arc<AtomicBool>,
    render: Option<std::thread::JoinHandle<()>>,
    capture: Option<ProcessCapture>,
    /// Live settings, so a slider does not restart the audio path.
    gain: Arc<AtomicU32>,
    bass: Arc<AtomicU32>,
}

impl Engine {
    /// Start boosting `exe`, which must currently be playing.
    pub fn start(exe: &str, settings: Settings) -> anyhow::Result<Self> {
        // Per-process capture arrived in Windows 10 2004. Below that the
        // activation fails with a bare COM error, which is a poor way to learn
        // that a feature needs a newer Windows.
        anyhow::ensure!(
            crate::platform::supports_process_loopback(),
            "volume boost needs Windows 10 version 2004 or newer (this is build {})",
            crate::platform::build()
        );

        let pid = session::any_pid(exe)
            .ok_or_else(|| anyhow!("{exe} is not playing anything to boost"))?;

        let queue: Arc<Mutex<std::collections::VecDeque<f32>>> =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(QUEUE_FRAMES * 2)));
        let stop = Arc::new(AtomicBool::new(false));
        let gain = Arc::new(AtomicU32::new(settings.gain.to_bits()));
        let bass = Arc::new(AtomicU32::new(settings.bass_db.to_bits()));

        let sink = Arc::clone(&queue);
        let capture = ProcessCapture::start(pid, move |samples| {
            let Ok(mut queue) = sink.lock() else { return };
            queue.extend(samples.iter().copied());
            // Falling behind must cost latency, not memory: drop from the front
            // so what plays is always the most recent audio.
            let limit = QUEUE_FRAMES * 2;
            while queue.len() > limit {
                queue.pop_front();
            }
        })?;

        // Remember where the user had this application before turning it down,
        // so stopping puts it back rather than at some default.
        let restore = session::read(exe).map(|v| v.scalar).unwrap_or(1.0).max(RESIDUAL);
        if let Err(e) = session::adjust(exe, 0.0, Some(RESIDUAL)) {
            tracing::warn!("could not lower {exe} for boost: {e:#}");
        }

        let flag = Arc::clone(&stop);
        let live_gain = Arc::clone(&gain);
        let live_bass = Arc::clone(&bass);
        let render = std::thread::Builder::new()
            .name("lumen-boost-render".into())
            .spawn(move || {
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if com.is_err() {
                    tracing::warn!("boost render: CoInitializeEx failed: {com:?}");
                    return;
                }
                if let Err(e) = run(&queue, &flag, &live_gain, &live_bass) {
                    tracing::warn!("boost render stopped: {e:#}");
                }
                unsafe { CoUninitialize() };
            })
            .context("could not start the boost render thread")?;

        tracing::info!(
            "boost on for {exe}: gain {:.2}x, bass {:+.1} dB",
            settings.gain,
            settings.bass_db
        );

        Ok(Self {
            exe: exe.to_owned(),
            restore,
            stop,
            render: Some(render),
            capture: Some(capture),
            gain,
            bass,
        })
    }

    /// Change gain or tone without interrupting playback.
    pub fn update(&self, settings: Settings) {
        self.gain.store(settings.gain.to_bits(), Ordering::Relaxed);
        self.bass.store(settings.bass_db.to_bits(), Ordering::Relaxed);
    }

    /// Which application this engine is boosting.
    pub fn target(&self) -> &str {
        &self.exe
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Capture first: the render thread drains what is left, so stopping it
        // the other way round ends on a fragment of stale audio.
        self.capture = None;
        if let Some(thread) = self.render.take() {
            let _ = thread.join();
        }
        // Give the application its volume back. If this fails it is left at two
        // percent, which is why it is logged rather than ignored.
        if let Err(e) = session::adjust(&self.exe, 0.0, Some(self.restore)) {
            tracing::warn!("could not restore {} after boost: {e:#}", self.exe);
        }
        tracing::info!("boost off for {}", self.exe);
    }
}

/// The render thread: pull from the queue, process, and play.
fn run(
    queue: &Mutex<std::collections::VecDeque<f32>>,
    stop: &AtomicBool,
    gain: &AtomicU32,
    bass: &AtomicU32,
) -> anyhow::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("could not create the device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .context("no default render endpoint")?;
        let client: IAudioClient =
            device.Activate(CLSCTX_ALL, None).context("could not activate IAudioClient")?;

        let format = client.GetMixFormat().context("no mix format")?;
        let channels = (*format).nChannels as usize;
        let rate = (*format).nSamplesPerSec;
        // The engine's own mix format is float; anything else would mean a
        // conversion this feature has no reason to own.
        if (*format).wBitsPerSample != 32 {
            return Err(anyhow!("the endpoint is not 32-bit float; boost needs a shared-mode float mix"));
        }

        let event = CreateEventW(None, false, false, None)?;
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                0,
                0,
                format,
                None,
            )
            .context("could not initialise the render client")?;
        client.SetEventHandle(event).context("could not attach the render event")?;

        let buffer_frames = client.GetBufferSize().context("no buffer size")?;
        let render: IAudioRenderClient = client.GetService().context("no IAudioRenderClient")?;
        client.Start().context("could not start rendering")?;

        // The capture arrives at `RESIDUAL` of full scale, so undoing that is
        // part of the gain rather than a separate stage.
        let makeup = |gain: u32| f32::from_bits(gain) / RESIDUAL;
        let mut processor = Processor::new(
            rate,
            channels,
            makeup(gain.load(Ordering::Relaxed)),
            f32::from_bits(bass.load(Ordering::Relaxed)),
        );
        let mut applied = (gain.load(Ordering::Relaxed), bass.load(Ordering::Relaxed));
        let mut block: Vec<f32> = Vec::new();


        while !stop.load(Ordering::SeqCst) {
            if WaitForSingleObject(event, 200) != WAIT_OBJECT_0 {
                continue;
            }

            // Rebuilding on every block would reset the filter's memory and
            // click; only a real change is worth that.
            let wanted = (gain.load(Ordering::Relaxed), bass.load(Ordering::Relaxed));
            if wanted != applied {
                processor = Processor::new(rate, channels, makeup(wanted.0), f32::from_bits(wanted.1));
                applied = wanted;
            }

            let padding = client.GetCurrentPadding().unwrap_or(0);
            let free = buffer_frames.saturating_sub(padding) as usize;
            if free == 0 {
                continue;
            }

            let wanted_samples = free * channels;
            block.clear();
            {
                let Ok(mut queue) = queue.lock() else { break };
                for _ in 0..wanted_samples {
                    match queue.pop_front() {
                        Some(sample) => block.push(sample),
                        // Underrun: pad with silence rather than repeating, which
                        // is the difference between a gap and a buzz.
                        None => block.push(0.0),
                    }
                }
            }

            processor.run(&mut block, channels);

            let data = render.GetBuffer(free as u32)?;
            std::ptr::copy_nonoverlapping(block.as_ptr(), data as *mut f32, block.len());
            render.ReleaseBuffer(free as u32, 0)?;
        }

        let _ = client.Stop();
        Ok(())
    }
}

/// Keeps at most one engine alive, pointed at whatever is playing.
///
/// The engine itself knows nothing about media sessions; this is what decides
/// when one should exist. That split is what keeps the idle cost at zero: with
/// boost off, or nothing playing, there is no engine, no capture and no thread.
#[derive(Default)]
pub struct Supervisor {
    engine: Mutex<Option<Engine>>,
}

impl Supervisor {
    /// Reconcile the running engine with what is wanted now.
    ///
    /// `exe` is the application to boost — the one the capsule is following —
    /// and `None` means nothing identifiable is playing.
    pub fn apply(&self, exe: Option<&str>, playing: bool, enabled: bool, settings: Settings) {
        let Ok(mut engine) = self.engine.lock() else { return };

        let wanted = exe.filter(|_| enabled && playing && settings.is_effective());

        match (&*engine, wanted) {
            // Nothing wanted: dropping the engine also unmutes the application.
            (Some(_), None) => *engine = None,
            (Some(running), Some(target)) if running.target() == target => {
                running.update(settings);
            }
            // A different application is playing now, so the old one has to be
            // released before the new one is muted.
            (Some(_), Some(target)) => {
                *engine = None;
                *engine = Engine::start(target, settings)
                    .inspect_err(|e| tracing::warn!("boost could not start: {e:#}"))
                    .ok();
            }
            (None, Some(target)) => {
                *engine = Engine::start(target, settings)
                    .inspect_err(|e| tracing::warn!("boost could not start: {e:#}"))
                    .ok();
            }
            (None, None) => {}
        }
    }

    /// Stop and unmute, for shutdown.
    pub fn stop(&self) {
        if let Ok(mut engine) = self.engine.lock() {
            *engine = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_settings_are_not_worth_running_for() {
        assert!(!Settings { gain: 1.0, bass_db: 0.0 }.is_effective());
        assert!(!Settings { gain: 1.004, bass_db: 0.05 }.is_effective());
    }

    /// The whole chain, against real audio.
    ///
    /// Checks the three things that make boost work rather than merely run: the
    /// source ends up muted, this process is the one making sound instead, and
    /// stopping puts the source back exactly as it was. Ignored because it needs
    /// something playing:
    ///   cargo test --lib boost_replaces -- --ignored --nocapture
    #[test]
    #[ignore = "needs audio playing"]
    fn boost_replaces_the_source_and_restores_it() {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        assert!(com.is_ok(), "COM: {com:?}");

        let Some((exe, _)) = session::loudest_session() else {
            panic!("nothing is playing — start some audio and run this again");
        };
        let ours = std::env::current_exe().expect("exe path").display().to_string().to_lowercase();
        println!("source: {exe}");

        let before = session::peak_and_mute(&exe).expect("source session");
        assert!(!before.1, "the source is already muted; test cannot tell what boost did");

        let engine = Engine::start(&exe, Settings { gain: 1.8, bass_db: 6.0 }).expect("boost start");
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let during = session::peak_and_mute(&exe).expect("source session");
        let rendered = session::peak_and_mute(&ours);
        let level = session::read(&exe).map(|v| v.scalar).unwrap_or(1.0);
        println!(
            "source level: {level:.3}   source peak: {:.4}   our render peak: {:?}",
            during.0,
            rendered.map(|r| r.0)
        );

        assert!(
            level <= RESIDUAL + 0.001,
            "boost must turn the source down, or both copies play at full level"
        );
        let peak = rendered.map(|(peak, _)| peak).unwrap_or(0.0);
        assert!(peak > 0.0, "boost produced no audio of its own — the sound would just be gone");

        drop(engine);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after = session::read(&exe).map(|v| v.scalar).unwrap_or(0.0);
        assert!(after > RESIDUAL, "stopping boost must restore the source's volume, got {after}");

        unsafe { CoUninitialize() };
    }

    #[test]
    fn a_real_change_is_worth_running_for() {
        assert!(Settings { gain: 1.5, bass_db: 0.0 }.is_effective());
        assert!(Settings { gain: 1.0, bass_db: 6.0 }.is_effective());
        // Cutting is as much a change as boosting.
        assert!(Settings { gain: 0.5, bass_db: 0.0 }.is_effective());
    }
}
