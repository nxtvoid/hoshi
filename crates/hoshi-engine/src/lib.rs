//! Runtime loop: takes commands, lets the core decide events, persists them
//! through the store, then runs the resulting effects (provider turns, git).

use chrono::{SecondsFormat, Utc};
use hoshi_core::{Command, CommandError, EventEnvelope, ReadModel, Snapshot, decide};
use hoshi_store::{Store, StoreError};
use std::path::Path;

pub struct Engine {
    store: Store,
    model: ReadModel,
    sequence: u64,
}

impl Engine {
    pub fn open(database: &Path) -> Result<Self, StoreError> {
        Self::from_store(Store::open(database)?)
    }

    pub fn from_store(store: Store) -> Result<Self, StoreError> {
        let snapshot = store.snapshot()?;
        Ok(Self {
            model: ReadModel::from_snapshot(&snapshot),
            sequence: snapshot.sequence,
            store,
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            sequence: self.sequence,
            projects: self.model.projects().cloned().collect(),
            threads: self.model.threads().cloned().collect(),
        }
    }

    pub fn dispatch(&mut self, command: Command) -> Result<Vec<EventEnvelope>, CommandError> {
        let command = normalize(command)?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let events = decide(&self.model, command, &now)?;
        let sequences =
            self.store
                .append(&events, &now)
                .map_err(|error| CommandError::Internal {
                    detail: error.to_string(),
                })?;
        for event in &events {
            self.model.apply(event);
        }
        if let Some(last) = sequences.last() {
            self.sequence = *last;
        }
        Ok(sequences
            .into_iter()
            .zip(events)
            .map(|(sequence, event)| EventEnvelope { sequence, event })
            .collect())
    }
}

/// Resolves a new project's folder to its canonical path, so the same folder
/// typed two ways is still one project.
fn normalize(command: Command) -> Result<Command, CommandError> {
    match command {
        Command::ProjectCreate {
            project_id,
            title,
            workspace_root,
        } => {
            let resolved = dunce::canonicalize(&workspace_root)
                .ok()
                .filter(|path| path.is_dir())
                .ok_or_else(|| CommandError::WorkspaceRootNotFound {
                    path: workspace_root.clone(),
                })?;
            Ok(Command::ProjectCreate {
                project_id,
                title,
                workspace_root: resolved.to_string_lossy().into_owned(),
            })
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::Event;

    fn engine() -> Engine {
        Engine::from_store(Store::open_in_memory().unwrap()).unwrap()
    }

    fn create_project(root: &Path) -> Command {
        Command::ProjectCreate {
            project_id: "p".into(),
            title: "repo".into(),
            workspace_root: root.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn dispatch_returns_sequenced_events_and_updates_the_snapshot() {
        let mut engine = engine();
        let root = std::env::temp_dir();

        let envelopes = engine.dispatch(create_project(&root)).unwrap();

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].sequence, 1);
        assert!(matches!(envelopes[0].event, Event::ProjectCreated { .. }));
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.sequence, 1);
        assert_eq!(
            snapshot.projects[0].workspace_root,
            dunce::canonicalize(&root).unwrap().to_string_lossy()
        );
    }

    #[test]
    fn rejects_a_folder_that_does_not_exist() {
        let mut engine = engine();
        let missing = std::env::temp_dir().join("hoshi-definitely-missing-folder");

        let error = engine.dispatch(create_project(&missing)).unwrap_err();

        assert!(matches!(error, CommandError::WorkspaceRootNotFound { .. }));
        assert_eq!(engine.snapshot().sequence, 0);
    }
}
