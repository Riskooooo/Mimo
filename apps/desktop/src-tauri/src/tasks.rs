//! The to-do list, persisted as `tasks.json` in the app data folder.
//! Parsing and the list logic live in `mimo_core::tasks`; this only owns
//! the instance and its file.

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime};
use mimo_core::tasks::{Due, Task, TaskStore};
use tauri::{AppHandle, Manager};

pub struct Tasks {
    store: Mutex<TaskStore>,
    path: Option<PathBuf>,
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

impl Tasks {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_data_dir().ok().map(|dir| dir.join("tasks.json"));
        let store = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { store: Mutex::new(store), path }
    }

    pub fn list(&self) -> Vec<Task> {
        self.lock().tasks.clone()
    }

    pub fn snapshot(&self) -> TaskStore {
        self.lock().clone()
    }

    pub fn add(&self, title: String, due: Due) -> Task {
        self.mutate(|store| store.add(title, due, now()))
    }

    pub fn update(&self, id: u64, title: String, date: Option<NaiveDate>, time: Option<NaiveTime>) -> Option<Task> {
        self.mutate(|store| store.update(id, title, date, time))
    }

    pub fn remove(&self, id: u64) -> Option<Task> {
        self.mutate(|store| store.remove(id))
    }

    /// Removes the task best matching `query`.
    pub fn remove_matching(&self, query: &str) -> Option<Task> {
        self.mutate(|store| {
            let id = store.find(query)?.id;
            store.remove(id)
        })
    }

    /// Back to an empty list (erase memory).
    pub fn clear(&self) {
        self.mutate(|store| *store = TaskStore::default());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TaskStore> {
        self.store.lock().expect("tasks mutex poisoned")
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut TaskStore) -> T) -> T {
        let mut store = self.lock();
        let result = change(&mut store);
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*store) {
                let _ = std::fs::write(path, json);
            }
        }
        result
    }
}
