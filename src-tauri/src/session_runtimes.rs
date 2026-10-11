//! Lazy process slots keyed by conversation, never by Agent or a global limit.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct Sessions<T>(Mutex<HashMap<String, Arc<T>>>);
impl<T> Default for Sessions<T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}
impl<T: Default> Sessions<T> {
    pub fn get(&self, id: &str) -> Arc<T> {
        self.0
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }
    pub fn existing(&self, id: &str) -> Option<Arc<T>> {
        self.0.lock().unwrap().get(id).cloned()
    }
    pub fn entries(&self) -> Vec<(String, Arc<T>)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, runtime)| (id.clone(), runtime.clone()))
            .collect()
    }
    pub fn remove_idle(&self, id: &str, runtime: &Arc<T>) {
        let mut slots = self.0.lock().unwrap();
        // A caller already using the slot must keep its identity until it finishes.
        if slots
            .get(id)
            .is_some_and(|slot| Arc::ptr_eq(slot, runtime) && Arc::strong_count(slot) == 2)
        {
            slots.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_share_only_their_own_slot_and_have_no_count_limit() {
        let sessions = Sessions::<Mutex<usize>>::default();
        let first = sessions.get("first");
        *first.lock().unwrap() = 1;
        assert!(Arc::ptr_eq(&first, &sessions.get("first")));
        for n in 0..100 {
            assert_eq!(*sessions.get(&format!("other-{n}")).lock().unwrap(), 0);
        }
        let held = first.clone();
        sessions.remove_idle("first", &first);
        assert!(sessions.existing("first").is_some());
        drop(held);
        sessions.remove_idle("first", &first);
        assert!(sessions.existing("first").is_none());
        assert_eq!(sessions.entries().len(), 100);
    }
}
