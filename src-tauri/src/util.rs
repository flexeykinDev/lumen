//! Small shared primitives.

use std::{
    future::IntoFuture,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Drive a future to completion on the calling thread, giving up after `timeout`.
///
/// `windows-future` exposes `IntoFuture` for the WinRT async types but keeps its
/// blocking `Async::join` private, and we deliberately do not want a general
/// async runtime on the SMTC thread. This is the whole executor we need: park
/// until the WinRT completion handler wakes us.
///
/// The timeout matters. A media source that has hung (a frozen browser tab, a
/// player mid-crash) will never complete `TryGetMediaPropertiesAsync`, and
/// without a deadline that would wedge the SMTC thread permanently — the island
/// would freeze on the last track forever. Returning `None` instead lets the
/// caller skip that refresh and stay responsive.
pub fn block_on_timeout<F: IntoFuture>(fut: F, timeout: Duration) -> Option<F::Output> {
    let mut fut = std::pin::pin!(fut.into_future());
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let deadline = Instant::now() + timeout;

    loop {
        if let Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
            return Some(value);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        // Spurious unparks are fine: the loop simply polls again.
        std::thread::park_timeout(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_ready_future() {
        assert_eq!(block_on_timeout(std::future::ready(7), Duration::from_secs(1)), Some(7));
    }

    #[test]
    fn gives_up_on_a_future_that_never_completes() {
        let never = std::future::pending::<()>();
        assert_eq!(block_on_timeout(never, Duration::from_millis(30)), None);
    }
}
