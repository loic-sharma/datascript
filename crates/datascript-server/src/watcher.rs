//! Recursive filesystem watcher that invalidates [`Session`]'s schema cache when files under
//! the UI root directory change (D-043, M4-01).

use std::path::Path;
use std::sync::Arc;

use datascript_engine::Session;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps the underlying OS file watcher alive until dropped.
pub struct RootWatcher {
    _watcher: RecommendedWatcher,
}

impl std::fmt::Debug for RootWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootWatcher").finish_non_exhaustive()
    }
}

impl RootWatcher {
    /// Starts watching `root` recursively, invalidating `session`'s cached schemas whenever files
    /// are created, modified, removed, or renamed.
    pub fn watch(root: &Path, session: Arc<Session>) -> notify::Result<Self> {
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else {
                // On watcher errors (e.g., rescan needed), conservatively invalidate.
                session.invalidate_schemas();
                return;
            };
            if should_invalidate(&event.kind) {
                session.invalidate_schemas();
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher })
    }
}

fn should_invalidate(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    )
}
