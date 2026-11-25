//! The session list's logic, kept apart from rendering and input so it can be tested:
//! which systemd operations a session offers, the keys for them, and the list's order.

use crate::ids::SessionId;
use crate::ipc::protocol::{SessionInfo, SysdAction, SysdData};

/// A systemd operation the selected session offers, and the key that triggers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceKey {
    pub key: char,
    /// Shown in the keybind bar, e.g. "stop".
    pub label: &'static str,
    pub action: SysdAction,
}

/// The operations that make sense for a service in this state, in keybind-bar order.
///
/// Start and stop share a key, as do enable and disable: the key does what the bar
/// shows for the state on screen, so it never depends on a state the user cannot see.
pub fn service_keys(sysd: SysdData) -> Vec<ServiceKey> {
    let mut keys = Vec::with_capacity(3);
    if sysd.running {
        keys.push(ServiceKey { key: 's', label: "stop", action: SysdAction::Stop });
        keys.push(ServiceKey { key: 'R', label: "restart", action: SysdAction::Restart });
    } else {
        keys.push(ServiceKey { key: 's', label: "start", action: SysdAction::Start });
    }
    if sysd.enabled {
        keys.push(ServiceKey { key: 'a', label: "disable autostart", action: SysdAction::Disable });
    } else {
        keys.push(ServiceKey { key: 'a', label: "enable autostart", action: SysdAction::Enable });
    }
    keys
}

/// The systemd operation `key` triggers on `session`. Always `None` for sessions that are
/// not systemd services, so these keys do nothing there.
pub fn service_action_for_key(session: &SessionInfo, key: char) -> Option<SysdAction> {
    let sysd = session.sysd_data?;
    service_keys(sysd)
        .into_iter()
        .find(|service_key| service_key.key == key)
        .map(|service_key| service_key.action)
}

/// True if `key` is one of the service keys, whatever the session's state. Used to explain
/// why nothing happened when one is pressed on a session that is not a service.
pub fn is_service_key(key: char) -> bool {
    matches!(key, 's' | 'R' | 'a')
}

/// "Starting", "Stopping", ... for the status line while the request is in flight.
pub fn action_in_progress(action: SysdAction) -> &'static str {
    match action {
        SysdAction::Start => "Starting",
        SysdAction::Stop => "Stopping",
        SysdAction::Restart => "Restarting",
        SysdAction::Enable => "Enabling autostart of",
        SysdAction::Disable => "Disabling autostart of",
    }
}

/// "Started", "Stopped", ... for the status line once the daemon accepted the request.
pub fn action_done(action: SysdAction) -> &'static str {
    match action {
        SysdAction::Start => "Started",
        SysdAction::Stop => "Stopped",
        SysdAction::Restart => "Restarted",
        SysdAction::Enable => "Enabled autostart of",
        SysdAction::Disable => "Disabled autostart of",
    }
}

/// The order sessions are shown in: by name (ignoring case), then by id, so the list stays
/// put between refreshes. The daemon returns sessions in no particular order.
pub fn sort_sessions(sessions: &mut [SessionInfo]) {
    sessions.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Where the cursor goes after the list changed: on the session it was on, if that still
/// exists, otherwise at the same position (clamped to the new length).
pub fn reselect(sessions: &[SessionInfo], selected: Option<SessionId>, old_index: usize) -> usize {
    selected
        .and_then(|id| sessions.iter().position(|session| session.id == id))
        .unwrap_or_else(|| old_index.min(sessions.len().saturating_sub(1)))
}

/// The session in `after` that was not in `before`: the one a create just added.
pub fn new_session(before: &[SessionId], after: &[SessionInfo]) -> Option<SessionId> {
    after
        .iter()
        .map(|session| session.id)
        .find(|id| !before.contains(id))
}
