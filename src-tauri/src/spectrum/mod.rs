//! A live spectrum, from a WASAPI loopback capture.
//!
//! # The gate is the design
//!
//! Everything else in Lumen is event-driven and sleeps at idle. This cannot be:
//! a spectrum means capturing audio continuously and transforming it many times
//! a second. So it is the one component with a hard on/off switch driven from
//! outside, and it is off unless the capsule is **expanded and playing** —
//! meaning someone is looking at it. Collapsed, hidden, or paused, the capture
//! thread does not exist; there is no thread parked on a timer and no device
//! held open. See `ARCHITECTURE.md` §3 for the constraint this is answering.
//!
//! # Loopback, not a microphone
//!
//! `AUDCLNT_STREAMFLAGS_LOOPBACK` on the *render* endpoint captures what is
//! being played rather than what a microphone hears, so it needs no recording
//! permission and picks up exactly the mix the user is listening to.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Context;
use windows::Win32::{
    Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        EDataFlow, ERole, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, WAVEFORMATEX, eConsole, eRender,
    },
    System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    },
};

pub mod fft;

pub use fft::BANDS;

/// Transform size. At 48 kHz this is ~21 ms of audio — long enough to resolve a
/// bass note, short enough that the bars track the beat.
const WINDOW: usize = 1024;

/// How often bands are published. Twenty a second is smooth once the renderer
/// eases between them, and is a twentieth of the IPC a per-frame push would be.
const PUBLISH_HZ: u64 = 20;

/// How quickly a bar may fall. Audio is far spikier than the eye wants; without
/// this the bars strobe. Rise is immediate so transients still snap.
const FALL: f32 = 0.16;

pub struct Spectrum {
    running: Arc<AtomicBool>,
}

impl Spectrum {
    /// Begin capturing. `on_bands` is called about `PUBLISH_HZ` times a second
    /// on the capture thread until [`Spectrum::stop`] or drop.
    pub fn start(on_bands: impl Fn([f32; BANDS]) + Send + 'static) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);

        let _ = std::thread::Builder::new().name("lumen-spectrum".into()).spawn(move || {
            // SAFETY: paired with CoUninitialize below; this thread owns its
            // apartment for its whole life.
            if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
                tracing::warn!("spectrum: COM unavailable");
                return;
            }
            if let Err(e) = capture(&flag, on_bands) {
                tracing::warn!("spectrum capture stopped: {e:#}");
            }
            unsafe { CoUninitialize() };
        });

        Self { running }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for Spectrum {
    fn drop(&mut self) {
        self.stop();
    }
}

/// CPU time this process has consumed, in seconds.
///
/// Used by the cost measurement below. Process time rather than a performance
/// counter: the numbers here are small enough that a counter's per-core
/// normalisation and sampling interval would dominate them.
#[cfg(test)]
fn process_cpu_seconds() -> f64 {
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .is_err()
        {
            return 0.0;
        }
    }
    let to_secs = |f: FILETIME| {
        (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 10_000_000.0
    };
    to_secs(kernel) + to_secs(user)
}

fn capture(
    running: &AtomicBool,
    on_bands: impl Fn([f32; BANDS]) + Send,
) -> anyhow::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("could not create the device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(EDataFlow(eRender.0), ERole(eConsole.0))
            .context("no default render endpoint")?;
        let client: IAudioClient =
            device.Activate(CLSCTX_ALL, None).context("could not activate IAudioClient")?;

        let format = client.GetMixFormat().context("no mix format")?;
        let (channels, rate, bits) = {
            let f: &WAVEFORMATEX = &*format;
            (f.nChannels as usize, f.nSamplesPerSec as f32, f.wBitsPerSample as usize)
        };

        // 200 ms buffer, in 100 ns units. Generous: the thread wakes on a timer
        // rather than an event, and an underrun would drop audio frames.
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                2_000_000,
                0,
                format,
                None,
            )
            .context("could not initialise loopback capture")?;
        CoTaskMemFree(Some(format.cast()));

        let capture_client: IAudioCaptureClient =
            client.GetService().context("no capture client")?;
        client.Start().context("could not start capture")?;

        let window = fft::hann(WINDOW);
        let mut mono: Vec<f32> = Vec::with_capacity(WINDOW * 2);
        let mut smoothed = [0.0f32; BANDS];
        let interval = std::time::Duration::from_millis(1000 / PUBLISH_HZ);
        let mut next_publish = std::time::Instant::now();

        while running.load(Ordering::SeqCst) {
            let mut frames = capture_client.GetNextPacketSize().unwrap_or(0);
            while frames > 0 {
                let mut data = std::ptr::null_mut();
                let mut count = 0u32;
                let mut flags = 0u32;
                if capture_client
                    .GetBuffer(&mut data, &mut count, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    // Windows hands back a silent packet rather than nothing at
                    // all when there is no audio; treating it as samples would
                    // read whatever the buffer happened to contain.
                    mono.extend(std::iter::repeat_n(0.0, count as usize));
                } else {
                    // The mix format is float in every case worth supporting,
                    // but check rather than assume: reinterpreting 16-bit PCM as
                    // f32 produces garbage, not quiet.
                    if bits == 32 {
                        let samples =
                            std::slice::from_raw_parts(data.cast::<f32>(), count as usize * channels);
                        // Downmix: the spectrum is one row of bars, not a stereo
                        // field.
                        for frame in samples.chunks_exact(channels) {
                            mono.push(frame.iter().sum::<f32>() / channels as f32);
                        }
                    }
                }

                let _ = capture_client.ReleaseBuffer(count);
                frames = capture_client.GetNextPacketSize().unwrap_or(0);
            }

            // Keep only what the next transform needs; a stalled consumer must
            // not grow this without bound.
            if mono.len() > WINDOW * 4 {
                mono.drain(..mono.len() - WINDOW);
            }

            let now = std::time::Instant::now();
            if now >= next_publish && mono.len() >= WINDOW {
                next_publish = now + interval;

                let start = mono.len() - WINDOW;
                let mut re: Vec<f32> =
                    mono[start..].iter().zip(&window).map(|(s, w)| s * w).collect();
                let mut im = vec![0.0f32; WINDOW];
                fft::fft(&mut re, &mut im);
                let raw = fft::to_bands(&re, &im, rate);

                for (out, value) in smoothed.iter_mut().zip(raw) {
                    // Normalised into roughly 0..1 by the transform size, then
                    // curved: loudness is logarithmic, and a linear bar spends
                    // most of its life near the floor.
                    let level = ((value / (WINDOW as f32 * 0.12)).min(1.0)).powf(0.45);
                    *out = if level > *out { level } else { *out * (1.0 - FALL) + level * FALL };
                }
                on_bands(smoothed);
            }

            // Sleeping rather than spinning: this is the whole cost of the
            // feature while it is on, and half a publish interval is plenty to
            // keep the buffer drained.
            std::thread::sleep(interval / 2);
        }

        let _ = client.Stop();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// What the capture actually costs, measured rather than asserted.
    ///
    /// Ignored by default: it opens the real audio endpoint, takes half a
    /// minute, and its number depends on the machine. Run it deliberately when
    /// changing anything in here:
    ///
    /// ```text
    /// cargo test -- --ignored --nocapture spectrum_cost
    /// ```
    ///
    /// Measures this process rather than the UI, because driving the capsule
    /// from outside proved unreliable — it auto-expands on a track change, so a
    /// sample taken against "collapsed" could quietly become one against
    /// "expanded" halfway through.
    #[test]
    #[ignore = "opens the audio device and takes ~30s"]
    fn spectrum_cost() {
        const WINDOW_SECS: f64 = 12.0;
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;

        let idle_start = process_cpu_seconds();
        std::thread::sleep(std::time::Duration::from_secs_f64(WINDOW_SECS));
        let idle = process_cpu_seconds() - idle_start;

        let publishes = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&publishes);
        let capture = Spectrum::start(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });

        let busy_start = process_cpu_seconds();
        std::thread::sleep(std::time::Duration::from_secs_f64(WINDOW_SECS));
        let busy = process_cpu_seconds() - busy_start;
        drop(capture);

        let pct = |secs: f64| 100.0 * secs / (WINDOW_SECS * cores);
        let published = publishes.load(Ordering::Relaxed);

        println!("\n--- spectrum cost over {WINDOW_SECS}s on {cores} cores ---");
        println!("  idle           {:.3}% of machine", pct(idle));
        println!("  capturing      {:.3}% of machine", pct(busy));
        println!(
            "  spectrum       {:.3}% of machine  ({:.2}% of one core)",
            pct(busy - idle),
            pct(busy - idle) * cores
        );
        println!("  published      {published} frames ({:.1}/s)", published as f64 / WINDOW_SECS);

        // The capture must actually have run, or the number above is measuring
        // nothing at all.
        assert!(published > 0, "no frames published; the capture never started");

        // A ceiling rather than an exact figure: this is a background visual,
        // and anything approaching a whole core would mean something is wrong.
        let cost_of_one_core = pct(busy - idle) * cores;
        assert!(
            cost_of_one_core < 25.0,
            "spectrum used {cost_of_one_core:.1}% of a core, which is far too much"
        );
    }
}