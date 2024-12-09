use std::time::Instant;

pub trait Clock {
    fn current_time(&self) -> Instant;
}

#[derive(Debug, Copy, Clone)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn current_time(&self) -> Instant {
        Instant::now()
    }
}
