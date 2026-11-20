use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Keeps a bounded set of open previews. Closing stops new lookups. A request
/// that already acquired a session may finish before releasing its reference.
pub(super) struct PreviewSessions<T> {
    active: Mutex<VecDeque<(String, Arc<T>)>>,
    limit: usize,
    expired: &'static str,
}

impl<T> PreviewSessions<T> {
    pub(super) const fn new(limit: usize, expired: &'static str) -> Self {
        Self {
            active: Mutex::new(VecDeque::new()),
            limit,
            expired,
        }
    }

    pub(super) fn insert(&self, id: String, session: T) -> Result<(), String> {
        let mut active = self.active.lock().map_err(|_| "会话存储不可用")?;
        active.push_back((id, Arc::new(session)));
        let evicted = if active.len() > self.limit {
            active.pop_front()
        } else {
            None
        };
        drop(active);
        drop(evicted);
        Ok(())
    }

    pub(super) fn get(&self, id: &str) -> Result<Arc<T>, String> {
        self.active
            .lock()
            .map_err(|_| "会话存储不可用")?
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, session)| Arc::clone(session))
            .ok_or_else(|| self.expired.into())
    }

    pub(super) fn close(&self, id: &str) -> Result<(), String> {
        let mut active = self.active.lock().map_err(|_| "会话存储不可用")?;
        let removed = active
            .iter()
            .position(|(key, _)| key == id)
            .and_then(|index| active.remove(index));
        drop(active);
        drop(removed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewSessions;

    #[test]
    fn releases_old_sessions_and_keeps_in_flight_reads_valid() {
        let sessions = PreviewSessions::new(2, "expired");
        sessions.insert("first".into(), 1).unwrap();
        let first = sessions.get("first").unwrap();
        sessions.insert("second".into(), 2).unwrap();
        sessions.insert("third".into(), 3).unwrap();

        assert_eq!(sessions.get("first").unwrap_err(), "expired");
        assert_eq!(*first, 1);
        assert_eq!(*sessions.get("second").unwrap(), 2);
        sessions.close("second").unwrap();
        sessions.close("second").unwrap();
        assert!(sessions.get("second").is_err());
        assert_eq!(*sessions.get("third").unwrap(), 3);
    }
}
