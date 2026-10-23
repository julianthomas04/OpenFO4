use std::sync::Mutex;

/// A one-slot hand-off between a producer thread and the render thread. A newer value replaces an
/// unread one, so the producer never waits and the consumer always sees the latest value.
#[derive(Debug)]
pub struct Mailbox<T> {
    slot: Mutex<Option<T>>,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

impl<T> Mailbox<T> {
    /// Stores `value`; returns true when it replaced a value nobody had taken.
    pub fn put(&self, value: T) -> bool {
        self.lock().replace(value).is_some()
    }

    pub fn take(&self) -> Option<T> {
        self.lock().take()
    }

    pub fn clear(&self) {
        self.lock().take();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<T>> {
        // A value is only ever swapped in or out, so a poisoned slot is still consistent.
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_mailbox_gives_nothing() {
        assert_eq!(Mailbox::<u32>::default().take(), None);
    }

    #[test]
    fn a_value_is_taken_exactly_once() {
        let mailbox = Mailbox::default();
        assert!(!mailbox.put(1));
        assert_eq!(mailbox.take(), Some(1));
        assert_eq!(mailbox.take(), None);
    }

    #[test]
    fn a_newer_value_replaces_an_unread_one_and_reports_it() {
        let mailbox = Mailbox::default();
        assert!(!mailbox.put(1));
        assert!(mailbox.put(2));
        assert_eq!(mailbox.take(), Some(2));
        assert!(!mailbox.put(3), "a taken slot is empty again");
    }

    #[test]
    fn clearing_drops_an_unread_value() {
        let mailbox = Mailbox::default();
        mailbox.put(1);
        mailbox.clear();
        assert_eq!(mailbox.take(), None);
    }

    #[test]
    fn a_producer_thread_never_blocks_behind_a_slow_consumer() {
        let mailbox = std::sync::Arc::new(Mailbox::default());
        let producer = std::thread::spawn({
            let mailbox = mailbox.clone();
            move || (0..1000).filter(|&n| mailbox.put(n)).count()
        });
        let replaced = producer.join().unwrap();
        assert_eq!(replaced, 999);
        assert_eq!(mailbox.take(), Some(999));
    }
}
