use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const FORGET_AFTER: Duration = Duration::from_secs(30 * 60);

/// Values read per key. A fresh value is served as is; a stale one is served
/// while a background read replaces it, so only the first read of a key waits.
pub(crate) struct Refreshing<T> {
    slots: Arc<Mutex<HashMap<String, Slot<T>>>>,
    fresh_for: Duration,
}

struct Slot<T> {
    value: Arc<T>,
    read_at: Instant,
    used_at: Instant,
    refreshing: bool,
}

impl<T: Send + Sync + 'static> Refreshing<T> {
    pub(crate) fn new(fresh_for: Duration) -> Self {
        Self {
            slots: Arc::default(),
            fresh_for,
        }
    }

    /// `read` returns `None` when it fails, which keeps the previous value.
    pub(crate) fn get(
        &self,
        key: &str,
        read: impl FnOnce() -> Option<T> + Send + 'static,
    ) -> Option<Arc<T>> {
        {
            let mut slots = lock(&self.slots);
            slots.retain(|_, slot| slot.used_at.elapsed() < FORGET_AFTER);
            if let Some(slot) = slots.get_mut(key) {
                slot.used_at = Instant::now();
                if slot.read_at.elapsed() >= self.fresh_for && !slot.refreshing {
                    slot.refreshing = true;
                    let slots = Arc::clone(&self.slots);
                    let key = key.to_owned();
                    thread::spawn(move || {
                        let value = read();
                        let mut slots = lock(&slots);
                        if let Some(slot) = slots.get_mut(&key) {
                            slot.refreshing = false;
                            if let Some(value) = value {
                                slot.value = Arc::new(value);
                                slot.read_at = Instant::now();
                            }
                        }
                    });
                }
                return Some(Arc::clone(&slot.value));
            }
        }
        let value = Arc::new(read()?);
        let now = Instant::now();
        lock(&self.slots).insert(
            key.to_owned(),
            Slot {
                value: Arc::clone(&value),
                read_at: now,
                used_at: now,
                refreshing: false,
            },
        );
        Some(value)
    }

    pub(crate) fn clear(&self) {
        lock(&self.slots).clear();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_stale_value_is_served_while_it_is_read_again() {
        let cache = Refreshing::new(Duration::ZERO);
        assert_eq!(*cache.get("k", || Some(1)).unwrap(), 1);

        let (sender, receiver) = mpsc::channel();
        let served = cache.get("k", move || {
            sender.send(()).unwrap();
            Some(2)
        });

        assert_eq!(*served.unwrap(), 1);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while *cache.get("k", || None).unwrap() != 2 {
            assert!(Instant::now() < deadline, "the refresh never landed");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_failed_first_read_is_not_kept() {
        let cache = Refreshing::<u32>::new(Duration::from_secs(60));

        assert_eq!(cache.get("k", || None), None);
        assert_eq!(*cache.get("k", || Some(3)).unwrap(), 3);
    }
}
