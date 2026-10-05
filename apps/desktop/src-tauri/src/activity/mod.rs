//! Activity analysis (setting `activity_enabled`, on by default): a background
//! thread samples the foreground window every few seconds, a
//! [`mimo_core::activity::Tracker`] turns that into sessions, and they're
//! kept in `activity.db` (SQLite, app data folder) for
//! [`mimo_core::activity::RETENTION_DAYS`] days. Nothing leaves the PC.

mod foreground;

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{Duration as Days, Local, NaiveTime, TimeZone};
use mimo_core::activity::{Observation, Session, Tracker, RETENTION_DAYS};
use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use self::foreground::Sampler;

const POLL: Duration = Duration::from_secs(5);
const PURGE_EVERY: Duration = Duration::from_secs(6 * 3600);

struct State {
    enabled: bool,
    tracker: Tracker,
    /// Opened on first use, closed by [`Activity::forget`] (Windows won't
    /// delete an open file).
    db: Option<Connection>,
    path: Option<PathBuf>,
    last_purge: Option<Instant>,
    /// The latest sample and when it was taken.
    last_seen: Option<(Instant, Option<Observation>)>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct Activity {
    shared: Shared,
}

impl Activity {
    pub fn new(app: &AppHandle, enabled: bool) -> Self {
        let path = app.path().app_data_dir().ok().map(|dir| dir.join("activity.db"));
        let state = State {
            enabled,
            tracker: Tracker::new(POLL.as_secs() as i64),
            db: None,
            path,
            last_purge: None,
            last_seen: None,
        };
        Self { shared: Arc::new((Mutex::new(state), Condvar::new())) }
    }

    /// Starts the sampling thread (idle while disabled).
    pub fn start(&self) {
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name("mimo-activity".into()).spawn(move || {
            let mut sampler = Sampler::default();
            let (lock, wake) = &*shared;
            loop {
                {
                    let mut state = lock.lock().expect("activity mutex poisoned");
                    while !state.enabled {
                        state = wake.wait(state).expect("activity mutex poisoned");
                    }
                }
                let seen = sampler.sample();
                let now = Local::now().timestamp();
                let mut state = lock.lock().expect("activity mutex poisoned");
                if !state.enabled {
                    continue;
                }
                state.last_seen = Some((Instant::now(), seen.clone()));
                let ended = state.tracker.observe(now, seen);
                state.store(&ended);
                if state.last_purge.is_none_or(|at| at.elapsed() >= PURGE_EVERY) {
                    state.purge(now);
                }
                let _ = wake.wait_timeout(state, POLL).expect("activity mutex poisoned");
            }
        });
    }

    pub fn set_enabled(&self, enabled: bool) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().expect("activity mutex poisoned");
        if !enabled {
            let last = state.tracker.finish();
            state.store(&last.into_iter().collect::<Vec<_>>());
        }
        state.enabled = enabled;
        wake.notify_all();
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().enabled
    }

    /// Stops and lets go of everything (erase memory): the session in
    /// progress is dropped and the database closed so it can be deleted.
    pub fn forget(&self) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().expect("activity mutex poisoned");
        state.enabled = false;
        state.tracker = Tracker::new(POLL.as_secs() as i64);
        state.db = None;
        state.last_seen = None;
        wake.notify_all();
    }

    /// What's in front, from the latest sample — `None` if nothing is, or
    /// if sampling isn't running.
    pub fn in_front(&self) -> Option<Observation> {
        let state = self.lock();
        let (at, seen) = state.last_seen.as_ref()?;
        (state.enabled && at.elapsed() < POLL * 3).then(|| seen.clone()).flatten()
    }

    /// Sessions overlapping `from..to`, the one in progress included.
    pub fn sessions(&self, from: i64, to: i64) -> Vec<Session> {
        let mut state = self.lock();
        let mut sessions = state.load(from, to);
        sessions.extend(state.tracker.current().filter(|s| s.end > from && s.start < to).cloned());
        sessions
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.0.lock().expect("activity mutex poisoned")
    }
}

impl State {
    fn db(&mut self) -> Option<&Connection> {
        if self.db.is_none() {
            let path = self.path.as_ref()?;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let db = Connection::open(path)
                .and_then(|db| {
                    db.execute_batch(
                        "CREATE TABLE IF NOT EXISTS sessions (
                             start INTEGER NOT NULL,
                             end INTEGER NOT NULL,
                             app TEXT NOT NULL,
                             label TEXT NOT NULL,
                             title TEXT NOT NULL
                         );
                         CREATE INDEX IF NOT EXISTS sessions_end ON sessions(end);",
                    )?;
                    Ok(db)
                })
                .map_err(|err| eprintln!("[activity] can't open {}: {err}", path.display()))
                .ok()?;
            self.db = Some(db);
        }
        self.db.as_ref()
    }

    fn store(&mut self, sessions: &[Session]) {
        if sessions.is_empty() {
            return;
        }
        let Some(db) = self.db() else { return };
        for s in sessions {
            if let Err(err) = db.execute(
                "INSERT INTO sessions (start, end, app, label, title) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![s.start, s.end, s.app, s.label, s.title],
            ) {
                eprintln!("[activity] can't store a session: {err}");
            }
        }
    }

    fn load(&mut self, from: i64, to: i64) -> Vec<Session> {
        let Some(db) = self.db() else { return Vec::new() };
        let query = |db: &Connection| -> rusqlite::Result<Vec<Session>> {
            let mut statement = db.prepare(
                "SELECT app, label, title, start, end FROM sessions WHERE end > ?1 AND start < ?2 ORDER BY start",
            )?;
            let rows = statement.query_map(params![from, to], |row| {
                Ok(Session {
                    app: row.get(0)?,
                    label: row.get(1)?,
                    title: row.get(2)?,
                    start: row.get(3)?,
                    end: row.get(4)?,
                })
            })?;
            rows.collect()
        };
        query(db).unwrap_or_else(|err| {
            eprintln!("[activity] can't read sessions: {err}");
            Vec::new()
        })
    }

    fn purge(&mut self, now: i64) {
        self.last_purge = Some(Instant::now());
        let Some(db) = self.db() else { return };
        let cutoff = now - RETENTION_DAYS * 86_400;
        if let Err(err) = db.execute("DELETE FROM sessions WHERE end < ?1", params![cutoff]) {
            eprintln!("[activity] can't purge old sessions: {err}");
        }
    }
}

/// Unix time of local midnight, `days_ago` days back (0 = today).
pub fn midnight(days_ago: i64) -> i64 {
    let date = Local::now().date_naive() - Days::days(days_ago);
    Local
        .from_local_datetime(&date.and_time(NaiveTime::MIN))
        .earliest()
        .map(|t| t.timestamp())
        .unwrap_or_else(|| Local::now().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimo_core::recall::{answer, last_stretch, Moment};
    use mimo_core::Lang;

    /// Answers "what was I doing" from the real `activity.db` (read-only):
    /// `cargo test -p desktop print_recall -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_recall() {
        let path = std::path::Path::new(&std::env::var("APPDATA").unwrap()).join("com.fabian.mimo").join("activity.db");
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).expect("activity.db");
        let mut state = State { enabled: false, tracker: Tracker::new(5), db: Some(db), path: None, last_purge: None, last_seen: None };
        let now = Local::now();
        let unix = |at: chrono::NaiveDateTime| Local.from_local_datetime(&at).earliest().unwrap().timestamp();
        let all = state.load(0, now.timestamp());
        println!("{} sessions stored", all.len());
        for moment in [Moment::Day { days_ago: 1 }, Moment::Day { days_ago: 0 }, Moment::At { days_ago: 1, hour: 17, minute: 0 }, Moment::Earlier] {
            let span = moment.resolve(now.naive_local());
            let (from, to) = (unix(span.from), unix(span.to));
            println!("{}", answer(&state.load(from, to), from, to, span.point.map(unix), span.moment, Lang::Fr));
        }
        let (from, to) = last_stretch(&all, now.timestamp());
        println!("last stretch: {} → {}", Local.timestamp_opt(from, 0).unwrap(), Local.timestamp_opt(to, 0).unwrap());
    }
}
