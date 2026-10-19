//! Idle GOP maintenance on Linux; media owners and the UI never wait for this worker.

use super::{EVICTED, Pool, Shared, VideoDecoder, VideoFrame, try_state};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, Weak};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_millis(250);

/// Intra and nested decoding can run without the state lock. Keep activity observable until
/// the whole request finishes, including cancellation and unwinding, without relocking State.
pub(super) struct Activity<'a>(&'a AtomicUsize);

impl<'a> Activity<'a> {
    pub(super) fn new(shared: &'a Shared) -> Self {
        shared.active_requests.fetch_add(1, Ordering::AcqRel);
        Self(&shared.active_requests)
    }
}

impl Drop for Activity<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

/// Detached resources must be destroyed after every registry/cache lock has been released.
/// Contain failures individually so one decoder destructor cannot kill maintenance or make
/// another destructor panic during unwinding.
#[derive(Default)]
pub(super) struct Retired {
    decoders: Vec<Box<dyn VideoDecoder>>,
    frames: Vec<Arc<VideoFrame>>,
}

impl Drop for Retired {
    fn drop(&mut self) {
        for decoder in self.decoders.drain(..) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(decoder))).is_err() {
                log::error!("idle GOP decoder cleanup panicked; maintenance continues");
            }
        }
        for frame in self.frames.drain(..) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| filmcraft_frame::pool::recycle(frame))).is_err() {
                log::error!("idle GOP frame cleanup panicked; maintenance continues");
            }
        }
    }
}

impl Pool {
    pub(super) fn take_idle(&self) -> Retired {
        let mut retired = Retired::default();
        let owners: Vec<Arc<Shared>> = {
            let mut registered = self.caches.lock().unwrap_or_else(PoisonError::into_inner);
            registered.retain(|cache| cache.strong_count() > 0);
            if self.decoders.load(Ordering::Relaxed) <= self.max_decoders && self.bytes.load(Ordering::Relaxed) <= self.budget {
                return retired;
            }
            registered.iter().filter_map(Weak::upgrade).collect()
        };
        // Keep every upgraded owner until after the last state lock: dropping even an unused
        // snapshot may run Shared::drop if the host concurrently stopped owning its source.
        let mut order: Vec<_> = owners
            .iter()
            .enumerate()
            .filter_map(|(i, owner)| {
                let state = try_state(owner)?;
                (owner.active_requests.load(Ordering::Acquire) == 0 && state.last_used.elapsed() >= self.idle).then_some((state.last_used, i))
            })
            .collect();
        order.sort_by_key(|(used, _)| *used);
        for (_, i) in order {
            if let Some(owner) = owners.get(i) {
                self.detach_if_idle(owner, &mut retired);
            }
        }
        // All cache guards are gone before owners (potentially their decoders) can be dropped.
        retired
    }

    pub(super) fn detach_if_idle(&self, owner: &Shared, retired: &mut Retired) {
        let Some(mut state) = try_state(owner) else { return };
        // The source could have been requested since the candidate snapshot was collected.
        if owner.active_requests.load(Ordering::Acquire) != 0 || state.last_used.elapsed() < self.idle {
            return;
        }
        if self.decoders.load(Ordering::Relaxed) > self.max_decoders {
            retired.decoders.extend(state.release_decoder(self));
        }
        while self.bytes.load(Ordering::Relaxed) > self.budget {
            let Some((pts, frame)) = state.frames.pop_first() else { break };
            state.drafts.remove(&pts);
            let bytes = frame.byte_size();
            state.bytes = state.bytes.saturating_sub(bytes);
            self.bytes.fetch_sub(bytes, Ordering::Relaxed);
            EVICTED.fetch_add(1, Ordering::Relaxed);
            retired.frames.push(frame);
        }
    }
}

pub(super) fn spawn(pool: &Arc<Pool>) -> std::io::Result<std::thread::JoinHandle<()>> {
    let weak = Arc::downgrade(pool);
    std::thread::Builder::new().name("filmcraft-gop-maintenance".into()).spawn(move || {
        loop {
            std::thread::sleep(INTERVAL);
            let Some(pool) = weak.upgrade() else { return };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(pool.take_idle()))).is_err() {
                log::error!("idle GOP maintenance panicked; the next pass will retry");
            }
            // No strong pool/cache/media owner is held while waiting for the next pass.
        }
    })
}
