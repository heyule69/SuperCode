//! Host-owned permission for one active turn. The controller's session id alone
//! is not authority: a client also needs its private, process-inherited token.
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub(super) struct Permissions(Mutex<HashMap<String, Access>>);
struct Access {
    token: String,
    attempt: String,
    full: bool,
    turn: Option<String>,
}
impl Default for Access {
    fn default() -> Self {
        Self {
            token: uuid::Uuid::new_v4().to_string(),
            attempt: String::new(),
            full: false,
            turn: None,
        }
    }
}
impl Permissions {
    pub(super) fn token(&self, session: &str) -> String {
        self.0
            .lock()
            .unwrap()
            .entry(session.into())
            .or_default()
            .token
            .clone()
    }
    pub(super) fn prepare(&self, session: &str, full: bool) -> String {
        let mut entries = self.0.lock().unwrap();
        let access = entries.entry(session.into()).or_default();
        access.attempt = uuid::Uuid::new_v4().to_string();
        access.full = full;
        access.turn = None;
        access.attempt.clone()
    }
    pub(super) fn bind(&self, session: &str, turn: &str) {
        if let Some(access) = self.0.lock().unwrap().get_mut(session) {
            if !access.attempt.is_empty() && access.turn.is_none() {
                access.turn = Some(turn.into());
            }
        }
    }
    pub(super) fn fail(&self, session: &str, attempt: &str) {
        if let Some(access) = self.0.lock().unwrap().get_mut(session) {
            if access.attempt == attempt {
                access.full = false;
                access.turn = None;
                access.attempt.clear();
            }
        }
    }
    pub(super) fn finish(&self, session: &str, turn: &str) {
        if let Some(access) = self.0.lock().unwrap().get_mut(session) {
            if access.turn.as_deref() == Some(turn) {
                access.full = false;
                access.turn = None;
                access.attempt.clear();
            }
        }
    }
    pub(super) fn update(&self, session: &str, turn: &str, full: bool) {
        if let Some(access) = self.0.lock().unwrap().get_mut(session) {
            if access.turn.as_deref() == Some(turn) {
                access.full = full;
            }
        }
    }
    pub(super) fn revoke(&self, session: &str) {
        if let Some(access) = self.0.lock().unwrap().get_mut(session) {
            access.full = false;
            access.turn = None;
            access.attempt.clear();
        }
    }
    pub(super) fn turn(&self, session: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .get(session)
            .and_then(|access| access.turn.clone())
    }
    pub(super) fn full(&self, session: &str, token: Option<&str>, turn: Option<&str>) -> bool {
        self.0.lock().unwrap().get(session).is_some_and(|access| {
            access.full
                && !access.attempt.is_empty()
                && token == Some(access.token.as_str())
                && turn.is_some()
                && access.turn.as_deref() == turn
        })
    }
    pub(super) fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_access_is_bound_to_the_host_turn_and_the_calling_session_token() {
        let permissions = Permissions::default();
        let a = permissions.token("a");
        let b = permissions.token("b");
        assert_ne!(a, b);
        let attempt = permissions.prepare("a", true);
        assert!(!permissions.full("a", Some(&a), None));
        permissions.bind("a", "t1");
        assert!(permissions.full("a", Some(&a), Some("t1")));
        assert!(!permissions.full("a", None, Some("t1")));
        assert!(!permissions.full("a", Some(&b), Some("t1")));
        assert!(!permissions.full("a", Some(&a), Some("other")));
        assert!(!permissions.full("b", Some(&a), Some("t1")));
        permissions.fail("a", &attempt);
        assert!(!permissions.full("a", Some(&a), Some("t1")));
    }
    #[test]
    fn completion_failure_and_permission_changes_do_not_authorize_a_later_turn() {
        let permissions = Permissions::default();
        let token = permissions.token("a");
        let previous = permissions.prepare("a", true);
        permissions.bind("a", "old");
        permissions.finish("a", "old");
        assert!(!permissions.full("a", Some(&token), Some("old")));
        permissions.prepare("a", true);
        permissions.finish("a", "old");
        permissions.fail("a", &previous);
        permissions.bind("a", "new");
        assert!(permissions.full("a", Some(&token), Some("new")));
        permissions.update("a", "old", false);
        assert!(permissions.full("a", Some(&token), Some("new")));
        permissions.update("a", "new", false);
        assert!(!permissions.full("a", Some(&token), Some("new")));
        permissions.update("a", "new", true);
        assert!(permissions.full("a", Some(&token), Some("new")));
        assert_eq!(permissions.token("a"), token); // Warm clients keep their identity.
        permissions.revoke("a");
        permissions.bind("a", "after-stop");
        permissions.update("a", "new", true);
        assert!(!permissions.full("a", Some(&token), Some("new")));
        assert!(!permissions.full("a", Some(&token), Some("after-stop")));
        permissions.prepare("a", true);
        permissions.bind("a", "after-stop");
        assert!(permissions.full("a", Some(&token), Some("after-stop")));
        permissions.clear();
        assert!(!permissions.full("a", Some(&token), Some("after-stop")));
    }
}
