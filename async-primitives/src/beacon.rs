use futures::task::AtomicWaker;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Clone)]
pub struct Beacon(Arc<RawBeacon>);

struct RawBeacon {
    waker: AtomicWaker,
    /// Current beacon value
    flag: AtomicBool,
    /// Consistency mode
    strong: bool,
}

impl RawBeacon {
    fn memory_ordering(&self) -> Ordering {
        if self.strong {
            Ordering::SeqCst
        } else {
            Ordering::Relaxed
        }
    }
}

impl Beacon {
    pub fn relaxed(flag: bool) -> Self {
        Self(Arc::new(RawBeacon {
            waker: AtomicWaker::new(),
            flag: AtomicBool::new(flag),
            strong: false,
        }))
    }

    pub fn strong(flag: bool) -> Self {
        Self(Arc::new(RawBeacon {
            waker: AtomicWaker::new(),
            flag: AtomicBool::new(flag),
            strong: true,
        }))
    }

    pub fn alter(&self, value: bool) {
        self.0.flag.store(value, self.0.memory_ordering());
        self.0.waker.wake();
    }

    pub fn read(&self) -> bool {
        self.0.flag.load(self.0.memory_ordering())
    }

    pub fn once(&self, anticipated_value: bool) -> Once {
        Once {
            beacon: self.0.clone(),
            anticipated_value,
        }
    }
}

/// Future that fires once beacon switches to `anticipated_value`.
pub struct Once {
    beacon: Arc<RawBeacon>,
    anticipated_value: bool,
}

impl Future for Once {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ord = self.beacon.memory_ordering();
        // quick check to avoid registration if already done.
        if self.beacon.flag.load(ord) == self.anticipated_value {
            return Poll::Ready(());
        }

        self.beacon.waker.register(cx.waker());

        // Need to check condition **after** `register` to avoid a race
        // condition that would result in lost notifications.
        if self.beacon.flag.load(ord) == self.anticipated_value {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
