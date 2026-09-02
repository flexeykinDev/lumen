//! Capturing one application's audio, rather than the whole endpoint.
//!
//! Windows 10 20H1 added a second kind of loopback: instead of tapping the
//! mixed output of a device, `ActivateAudioInterfaceAsync` can tap the streams
//! rendered by one process tree. That is what makes an app-specific effect
//! possible at all — see `boost` for what is done with the samples.
//!
//! Two things here are unusual enough to be worth stating. The interface is
//! activated *asynchronously* through a COM callback, so there is a completion
//! handler and a wait. And the activation parameters travel inside a
//! `PROPVARIANT` holding a raw blob, which windows-rs does not offer a
//! constructor for — see `BlobVariant`.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow};
use windows::{
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
            AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
            IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
        },
        System::{
            Com::{
                COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
            },
            Threading::{CreateEventW, SetEvent, WaitForSingleObject},
            Variant::VT_BLOB,
        },
    },
    core::{HRESULT, Interface},
};
use windows::Win32::System::Com::BLOB;

/// `WAVE_FORMAT_IEEE_FLOAT`. Not re-exported by the audio module in this
/// version of the bindings, and it is a stable value from mmreg.h.
const FORMAT_FLOAT: u16 = 3;

/// The format we ask the capture for.
///
/// Process loopback has no mix format of its own to query — it is a virtual
/// device — so the caller names one and the engine converts into it. 32-bit
/// float stereo at 48 kHz is what the DSP wants and what every modern endpoint
/// runs at anyway, so in practice nothing is resampled twice.
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

/// A `PROPVARIANT` pointing at an activation-parameters blob.
///
/// This API takes its parameters as a `VT_BLOB` variant and accepts nothing
/// else. The blob is *borrowed*: the pointer inside belongs to the caller's
/// stack, so the variant must never be cleared.
///
/// That is why the result is wrapped in `ManuallyDrop`. windows-rs gives
/// `PROPVARIANT` a `Drop` that calls `PropVariantClear`, which for a blob calls
/// `CoTaskMemFree` on the pointer — handing the allocator a stack address and
/// corrupting the heap. It cost a `STATUS_HEAP_CORRUPTION` crash to find, and
/// the crash landed on the *next* allocation rather than here, which is what
/// made it interesting. Nothing is leaked: the variant owns no memory.
fn blob_variant<T>(blob: &mut T) -> std::mem::ManuallyDrop<PROPVARIANT> {
    std::mem::ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<T>() as u32,
                        pBlobData: (blob as *mut T).cast(),
                    },
                },
            }),
        },
    })
}

/// Signals the waiting thread when the asynchronous activation finishes.
#[windows::core::implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    done: HANDLE,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: windows::core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        // The result is read from the operation by the waiting thread; this
        // callback exists only to say that there is one.
        unsafe {
            let _ = SetEvent(self.done);
        }
        Ok(())
    }
}

/// A running capture of one process tree's audio.
pub struct ProcessCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProcessCapture {
    /// Start capturing everything `pid` and its children render.
    ///
    /// `on_samples` is called on the capture thread with interleaved stereo
    /// floats, roughly every 10 ms. It must not block: the audio engine is
    /// waiting on the other side of it.
    pub fn start(
        pid: u32,
        mut on_samples: impl FnMut(&[f32]) + Send + 'static,
    ) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();

        let thread = std::thread::Builder::new()
            .name("lumen-process-capture".into())
            .spawn(move || {
                // COM lives and dies with this thread, and every interface below
                // belongs to it.
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if com.is_err() {
                    let _ = ready_tx.send(Err(anyhow!("CoInitializeEx failed: {com:?}")));
                    return;
                }

                match run(pid, &flag, &mut on_samples, &ready_tx) {
                    Ok(()) => {}
                    Err(e) => {
                        // If the failure happened before the handshake the
                        // caller is still waiting; if after, it is only a log.
                        let _ = ready_tx.send(Err(e));
                    }
                }

                unsafe { CoUninitialize() };
            })
            .context("could not start the capture thread")?;

        // Surface an activation failure to the caller rather than leaving a
        // thread that quietly does nothing.
        match ready_rx.recv_timeout(Duration::from_secs(4)) {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("the capture did not start within four seconds")),
        }
    }
}

impl Drop for ProcessCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The capture thread's body.
fn run(
    pid: u32,
    stop: &AtomicBool,
    on_samples: &mut impl FnMut(&[f32]),
    ready: &std::sync::mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let client = activate(pid)?;

    let format = WAVEFORMATEX {
        wFormatTag: FORMAT_FLOAT,
        nChannels: CHANNELS,
        nSamplesPerSec: SAMPLE_RATE,
        wBitsPerSample: 32,
        nBlockAlign: CHANNELS * 4,
        nAvgBytesPerSec: SAMPLE_RATE * u32::from(CHANNELS) * 4,
        cbSize: 0,
    };

    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                // Buffer duration and period must both be zero for a process
                // loopback client; the engine picks them.
                0,
                0,
                &format,
                None,
            )
            .context("could not initialise the process loopback client")?;

        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event).context("could not attach the capture event")?;
        let capture: IAudioCaptureClient =
            client.GetService().context("no IAudioCaptureClient")?;
        client.Start().context("could not start the capture")?;

        let _ = ready.send(Ok(()));

        let mut buffer: Vec<f32> = Vec::with_capacity(4096);
        while !stop.load(Ordering::SeqCst) {
            // A 200 ms wait rather than INFINITE so a stopped stream still
            // reaches the loop condition and the thread can retire.
            if WaitForSingleObject(event, 200) != WAIT_OBJECT_0 {
                continue;
            }

            loop {
                let available = capture.GetNextPacketSize().unwrap_or(0);
                if available == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                let count = frames as usize * CHANNELS as usize;
                buffer.clear();
                if data.is_null() {
                    // A silent packet carries no buffer at all; the frames are
                    // still real and must be passed on as zeroes.
                    buffer.resize(count, 0.0);
                } else {
                    buffer.extend_from_slice(std::slice::from_raw_parts(data as *const f32, count));
                }
                on_samples(&buffer);

                let _ = capture.ReleaseBuffer(frames);
            }
        }

        let _ = client.Stop();
    }

    Ok(())
}

/// Activate an `IAudioClient` bound to one process tree.
fn activate(pid: u32) -> anyhow::Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                // The tree, not the process: a browser renders its audio from
                // child processes, so targeting the pid alone captures silence
                // from exactly the applications this is most wanted for.
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };

    unsafe {
        let done = CreateEventW(None, false, false, None)?;
        let handler: IActivateAudioInterfaceCompletionHandler = Completion { done }.into();
        let activation = blob_variant(&mut params);

        let operation: IActivateAudioInterfaceAsyncOperation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*activation),
            &handler,
        )
        .context("ActivateAudioInterfaceAsync failed")?;

        if WaitForSingleObject(done, 4_000) != WAIT_OBJECT_0 {
            return Err(anyhow!("the audio interface never activated"));
        }

        let mut result = HRESULT(0);
        let mut interface = None;
        operation
            .GetActivateResult(&mut result, &mut interface)
            .context("could not read the activation result")?;
        result.ok().context("the process loopback activation was refused")?;

        interface
            .and_then(|i| i.cast::<IAudioClient>().ok())
            .ok_or_else(|| anyhow!("activation returned no audio client"))
    }
}

/// Root-mean-square level of a block, as a quick "is there sound here" answer.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_is_zero_for_silence_and_positive_for_sound() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0.0; 64]), 0.0);
        assert!(rms(&[0.5, -0.5, 0.5, -0.5]) > 0.49);
    }

    /// Where the capture tap sits relative to the application's own volume.
    ///
    /// The whole boost design turns on this. If the tap were *before* the
    /// session volume, the source could simply be muted while its audio was
    /// still captured. It is not — the capture follows the slider, so a muted
    /// source captures silence. That is why `boost` turns the source down to
    /// two percent rather than muting it, and multiplies the difference back.
    ///
    /// An earlier version of this test asked only whether a *muted* app still
    /// produced something, ran against a `SoundPlayer` tone, and concluded the
    /// opposite. It was wrong, and it took a silent Spotify to prove it — hence
    /// a ratio here rather than a yes/no.
    ///
    /// Ignored because it needs a real application playing real audio:
    ///   cargo test --lib the_tap_follows -- --ignored --nocapture
    #[test]
    #[ignore = "needs audio playing"]
    fn the_tap_follows_the_session_volume() {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        assert!(com.is_ok(), "COM: {com:?}");

        let Some((exe, pid)) = crate::audio::session::loudest_session() else {
            panic!("nothing is playing — start some audio and run this again");
        };
        println!("target: {exe} (pid {pid})");

        let level = Arc::new(std::sync::Mutex::new(0.0f32));
        let sink = Arc::clone(&level);
        let capture = ProcessCapture::start(pid, move |samples| {
            let mut peak = sink.lock().expect("level lock");
            *peak = peak.max(rms(samples));
        })
        .expect("capture should start");

        let measure = |label: &str| {
            // Settle first: a volume change reaches the engine over a few
            // hundred milliseconds, and sampling across that transition
            // measures the old level as much as the new one.
            std::thread::sleep(Duration::from_millis(700));
            *level.lock().expect("level lock") = 0.0;
            std::thread::sleep(Duration::from_millis(1200));
            let value = *level.lock().expect("level lock");
            println!("{label}: rms {value:.5}");
            value
        };

        let was = crate::audio::session::read(&exe).map(|v| v.scalar).unwrap_or(1.0);
        crate::audio::session::adjust(&exe, 0.0, Some(1.0)).expect("full volume");
        let full = measure("at 100%");
        crate::audio::session::adjust(&exe, 0.0, Some(0.02)).expect("quiet");
        let quiet = measure("at 2%");
        crate::audio::session::adjust(&exe, 0.0, Some(was)).expect("restore");

        drop(capture);
        unsafe { CoUninitialize() };

        assert!(full > 0.0005, "nothing was captured at full volume: {full}");
        assert!(quiet > 0.0, "two percent must still be a signal, or there is nothing to boost");
        // Loose bounds because music is not a test tone: the two windows measure
        // different bars of the same song.
        let ratio = quiet / full;
        println!("ratio {ratio:.4} — expected around 0.02");
        assert!(
            ratio < 0.2,
            "the capture ignored the session volume; boost's design assumes it does not"
        );
    }
}
