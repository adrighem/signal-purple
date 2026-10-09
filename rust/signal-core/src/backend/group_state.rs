use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

use super::coordinator::ProjectionDisposition;

#[derive(Clone, Default)]
pub(crate) struct DepartedGroups {
    state: Arc<Mutex<GroupLeaveState>>,
    operation: Arc<AsyncMutex<()>>,
}

#[derive(Default)]
struct GroupLeaveState {
    leaving: HashSet<String>,
    departed: HashSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GroupDepartureState {
    Active,
    Leaving,
    Departed,
}

pub(crate) fn departure_projection_disposition(
    state: GroupDepartureState,
) -> Option<ProjectionDisposition> {
    match state {
        GroupDepartureState::Active => None,
        GroupDepartureState::Leaving => Some(ProjectionDisposition::Retry),
        GroupDepartureState::Departed => Some(ProjectionDisposition::Complete),
    }
}

impl DepartedGroups {
    pub(crate) fn departure_state(&self, identifier: &str) -> GroupDepartureState {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.departed.contains(identifier) {
            GroupDepartureState::Departed
        } else if state.leaving.contains(identifier) {
            GroupDepartureState::Leaving
        } else {
            GroupDepartureState::Active
        }
    }

    pub(crate) fn contains(&self, identifier: &str) -> bool {
        self.departure_state(identifier) != GroupDepartureState::Active
    }

    pub(crate) fn is_departed(&self, identifier: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .departed
            .contains(identifier)
    }

    pub(crate) fn begin_leave(&self, identifier: String) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .leaving
            .insert(identifier);
    }

    pub(crate) fn cancel_leave(&self, identifier: &str) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .leaving
            .remove(identifier);
    }

    pub(crate) fn mark_departed(&self, identifier: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.leaving.remove(&identifier);
        state.departed.insert(identifier);
    }

    pub(crate) async fn lock_operation(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operation.lock().await
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum GroupLeaveCompletion {
    Accepted {
        peer_notification_sent: bool,
        local_group_removed: bool,
    },
    Failed(String),
}
