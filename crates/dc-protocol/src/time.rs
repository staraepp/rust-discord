//! Clock abstraction so heartbeat, backoff and rate-limit logic can be
//! tested deterministically without real time.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// A monotonic clock plus a way to sleep on it.
///
/// `now()` is the time elapsed since an arbitrary, fixed origin (usually the
/// clock's creation). The real implementations live in the IO crates; tests
/// use [`FakeClock`].
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Duration;

    /// Completes once `duration` has elapsed on this clock. Must be
    /// cancel-safe (dropping the future has no side effects).
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

/// A manually advanced clock for tests. Time only moves when
/// [`FakeClock::advance`] is called; clones share the same time line.
#[derive(Clone, Default)]
pub struct FakeClock {
    inner: Arc<Mutex<FakeClockState>>,
}

#[derive(Default)]
struct FakeClockState {
    now: Duration,
    next_id: u64,
    waiters: Vec<Waiter>,
}

struct Waiter {
    id: u64,
    deadline: Duration,
    waker: Waker,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves time forward and wakes every sleeper whose deadline has passed.
    pub fn advance(&self, by: Duration) {
        let due: Vec<Waker> = {
            let mut state = self.lock();
            state.now += by;
            let now = state.now;
            let (due, pending): (Vec<_>, Vec<_>) =
                state.waiters.drain(..).partition(|w| w.deadline <= now);
            state.waiters = pending;
            due.into_iter().map(|w| w.waker).collect()
        };
        for waker in due {
            waker.wake();
        }
    }

    /// Number of sleeps currently pending. Handy for asserting that the code
    /// under test is parked on the clock.
    pub fn pending_sleeps(&self) -> usize {
        self.lock().waiters.len()
    }

    /// The earliest pending deadline, as a duration from "now".
    pub fn next_wakeup_in(&self) -> Option<Duration> {
        let state = self.lock();
        state
            .waiters
            .iter()
            .map(|w| w.deadline.saturating_sub(state.now))
            .min()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeClockState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.lock().now
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        let deadline = self.now() + duration;
        FakeSleep {
            clock: self.clone(),
            deadline,
            id: None,
        }
    }
}

struct FakeSleep {
    clock: FakeClock,
    deadline: Duration,
    id: Option<u64>,
}

impl Future for FakeSleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let mut state = this.clock.lock();
        if state.now >= this.deadline {
            if let Some(id) = this.id.take() {
                state.waiters.retain(|w| w.id != id);
            }
            return Poll::Ready(());
        }
        match this.id {
            Some(id) => {
                if let Some(waiter) = state.waiters.iter_mut().find(|w| w.id == id) {
                    waiter.waker.clone_from(cx.waker());
                }
            }
            None => {
                let id = state.next_id;
                state.next_id += 1;
                state.waiters.push(Waiter {
                    id,
                    deadline: this.deadline,
                    waker: cx.waker().clone(),
                });
                this.id = Some(id);
            }
        }
        Poll::Pending
    }
}

impl Drop for FakeSleep {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.clock.lock().waiters.retain(|w| w.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Wake;

    struct CountingWaker(std::sync::atomic::AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn sleep_completes_only_after_advance() {
        let clock = FakeClock::new();
        let counter = Arc::new(CountingWaker(Default::default()));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);

        let mut sleep = Box::pin(clock.sleep(Duration::from_secs(10)));
        assert!(sleep.as_mut().poll(&mut cx).is_pending());
        assert_eq!(clock.pending_sleeps(), 1);
        assert_eq!(clock.next_wakeup_in(), Some(Duration::from_secs(10)));

        clock.advance(Duration::from_secs(9));
        assert!(sleep.as_mut().poll(&mut cx).is_pending());
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 0);

        clock.advance(Duration::from_secs(1));
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(sleep.as_mut().poll(&mut cx).is_ready());
        assert_eq!(clock.pending_sleeps(), 0);
        assert_eq!(clock.now(), Duration::from_secs(10));
    }

    #[test]
    fn dropping_a_sleep_unregisters_it() {
        let clock = FakeClock::new();
        let waker = Waker::from(Arc::new(CountingWaker(Default::default())));
        let mut cx = Context::from_waker(&waker);
        let mut sleep = Box::pin(clock.sleep(Duration::from_secs(1)));
        assert!(sleep.as_mut().poll(&mut cx).is_pending());
        drop(sleep);
        assert_eq!(clock.pending_sleeps(), 0);
    }

    #[test]
    fn zero_sleep_is_immediately_ready() {
        let clock = FakeClock::new();
        let waker = Waker::from(Arc::new(CountingWaker(Default::default())));
        let mut cx = Context::from_waker(&waker);
        let mut sleep = Box::pin(clock.sleep(Duration::ZERO));
        assert!(sleep.as_mut().poll(&mut cx).is_ready());
    }
}
