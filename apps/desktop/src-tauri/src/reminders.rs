//! Keeps reminders/alarms (persisted in the app data folder) and rings them
//! on time: a background thread sleeps until the next one is due, then
//! brings the pill up in "ringing" mode. Only works while Mimo runs and the
//! PC is awake.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use chrono::{Local, NaiveTime, TimeZone, Timelike};
use mimo_core::reminders::{Reminder, ReminderKind, ReminderStore, When};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Re-check at least this often, so a clock change or sleep/resume never
/// leaves a reminder waiting much longer than it should.
const MAX_SLEEP: Duration = Duration::from_secs(30);

#[derive(Clone, Serialize)]
struct Ringing {
    kind: ReminderKind,
    message: Option<String>,
    /// "7:30"-style local time it was due.
    time: String,
}

type Shared = Arc<(Mutex<ReminderStore>, Condvar)>;

pub struct Reminders {
    shared: Shared,
    path: Option<PathBuf>,
}

impl Reminders {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_data_dir().ok().map(|dir| dir.join("reminders.json"));
        let store = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { shared: Arc::new((Mutex::new(store), Condvar::new())), path }
    }

    /// Starts the thread that rings reminders when they're due.
    pub fn start(&self, app: AppHandle) {
        let shared = self.shared.clone();
        let path = self.path.clone();
        let _ = std::thread::Builder::new()
            .name("mimo-reminders".into())
            .spawn(move || loop {
                let due = {
                    let (lock, cvar) = &*shared;
                    let mut store = lock.lock().expect("reminders mutex poisoned");
                    let now = Local::now().timestamp();
                    let due = store.take_due(now);
                    if due.is_empty() {
                        let wait = store
                            .next_due()
                            .map(|next| Duration::from_secs((next - now).max(0) as u64))
                            .unwrap_or(MAX_SLEEP)
                            .min(MAX_SLEEP);
                        let _ = cvar.wait_timeout(store, wait).expect("reminders mutex poisoned");
                        continue;
                    }
                    save(path.as_ref(), &store);
                    due
                };
                for reminder in due {
                    ring(&app, &reminder);
                }
            });
    }

    /// Schedules a request; returns the local (hour, minute) it's due at.
    pub fn add(&self, kind: ReminderKind, when: When, message: Option<String>) -> (u32, u32) {
        let now = Local::now();
        let due = match when {
            When::In { seconds } => now + chrono::Duration::seconds(seconds as i64),
            When::At { hour, minute } => {
                let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_default();
                let mut date = now.date_naive();
                // "à 7 h" means the next 7:00, today if still ahead.
                if now.time() >= time {
                    date = date.succ_opt().unwrap_or(date);
                }
                Local
                    .from_local_datetime(&date.and_time(time))
                    .earliest()
                    .unwrap_or(now)
            }
        };
        self.mutate(|store| {
            store.add(kind, due.timestamp(), message);
        });
        (due.hour(), due.minute())
    }

    pub fn list(&self) -> Vec<Reminder> {
        self.shared.0.lock().expect("reminders mutex poisoned").reminders.clone()
    }

    pub fn cancel(&self, kind: Option<ReminderKind>) -> usize {
        let mut removed = 0;
        self.mutate(|store| removed = store.cancel(kind));
        removed
    }

    pub fn remove(&self, id: u64) {
        self.mutate(|store| {
            store.remove(id);
        });
    }

    fn mutate(&self, change: impl FnOnce(&mut ReminderStore)) {
        let (lock, cvar) = &*self.shared;
        let mut store = lock.lock().expect("reminders mutex poisoned");
        change(&mut store);
        save(self.path.as_ref(), &store);
        // Wake the ringer: the next due time may have changed.
        cvar.notify_all();
    }
}

fn save(path: Option<&PathBuf>, store: &ReminderStore) {
    let Some(path) = path else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(store) {
        let _ = std::fs::write(path, json);
    }
}

fn ring(app: &AppHandle, reminder: &Reminder) {
    let time = Local
        .timestamp_opt(reminder.due, 0)
        .single()
        .map(|t| format!("{}:{:02}", t.hour(), t.minute()))
        .unwrap_or_default();
    eprintln!("[reminders] ringing {:?} {:?}", reminder.kind, reminder.message);
    crate::summon(app, "reminder");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(
            "mimo://ringing",
            Ringing { kind: reminder.kind, message: reminder.message.clone(), time },
        );
    }
}
