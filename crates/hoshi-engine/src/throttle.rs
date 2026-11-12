use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

struct Pace<T> {
    sent_at: Option<Instant>,
    /// What was offered too soon, merged, sent once the interval passes.
    waiting: Option<T>,
    flushing: bool,
}

type Sender<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Passes values on at most once per interval. Values offered meanwhile are merged into one,
/// sent once the interval passes. Sending happens under the lock, so what goes out keeps the
/// order it was offered in.
pub(crate) struct Throttle<T> {
    every: Duration,
    pace: Arc<Mutex<Pace<T>>>,
    merge: fn(&mut T, T),
    send: Sender<T>,
}

impl<T: Send + 'static> Throttle<T> {
    pub(crate) fn new(
        every: Duration,
        merge: fn(&mut T, T),
        send: impl Fn(T) + Send + Sync + 'static,
    ) -> Self {
        Self {
            every,
            pace: Arc::new(Mutex::new(Pace {
                sent_at: None,
                waiting: None,
                flushing: false,
            })),
            merge,
            send: Arc::new(send),
        }
    }

    /// Sends `value` now, or merged with what else comes until the interval since the last
    /// send passes.
    pub(crate) fn offer(&self, value: T) {
        let mut pace = lock(&self.pace);
        let now = Instant::now();
        let due = pace.sent_at.map_or(now, |at| at + self.every);
        if !pace.flushing && due <= now {
            pace.sent_at = Some(now);
            (self.send)(value);
            return;
        }
        match &mut pace.waiting {
            Some(waiting) => (self.merge)(waiting, value),
            waiting => *waiting = Some(value),
        }
        if pace.flushing {
            return;
        }
        pace.flushing = true;
        let (shared, send) = (self.pace.clone(), self.send.clone());
        thread::spawn(move || {
            thread::sleep(due.saturating_duration_since(Instant::now()));
            let mut pace = lock(&shared);
            pace.flushing = false;
            if let Some(value) = pace.waiting.take() {
                pace.sent_at = Some(Instant::now());
                send(value);
            }
        });
    }

    /// Sends what waits now, so whatever is sent next goes after it.
    pub(crate) fn flush(&self) {
        let mut pace = lock(&self.pace);
        if let Some(value) = pace.waiting.take() {
            pace.sent_at = Some(Instant::now());
            (self.send)(value);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Sent = Arc<Mutex<Vec<Vec<u32>>>>;

    fn recording(
        every: Duration,
        merge: fn(&mut Vec<u32>, Vec<u32>),
    ) -> (Throttle<Vec<u32>>, Sent) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let record = sent.clone();
        let throttle = Throttle::new(every, merge, move |value| lock(&record).push(value));
        (throttle, sent)
    }

    fn latest(waiting: &mut Vec<u32>, value: Vec<u32>) {
        *waiting = value;
    }

    fn all(waiting: &mut Vec<u32>, value: Vec<u32>) {
        waiting.extend(value);
    }

    #[test]
    fn the_first_value_goes_at_once_and_a_burst_ends_with_its_latest() {
        let (throttle, sent) = recording(Duration::from_millis(60), latest);
        for value in 1..=5 {
            throttle.offer(vec![value]);
        }
        assert_eq!(*lock(&sent), [vec![1]]);

        thread::sleep(Duration::from_millis(250));
        assert_eq!(*lock(&sent), [vec![1], vec![5]]);
    }

    #[test]
    fn values_spaced_wider_than_the_interval_all_go() {
        let (throttle, sent) = recording(Duration::from_millis(20), latest);
        for value in 1..=3 {
            throttle.offer(vec![value]);
            thread::sleep(Duration::from_millis(60));
        }
        assert_eq!(*lock(&sent), [vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn a_burst_goes_merged_and_a_flush_sends_it_at_once() {
        let (throttle, sent) = recording(Duration::from_secs(60), all);
        for value in 1..=4 {
            throttle.offer(vec![value]);
        }
        throttle.flush();
        throttle.flush();
        assert_eq!(*lock(&sent), [vec![1], vec![2, 3, 4]]);
    }
}
