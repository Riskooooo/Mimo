//! Proactive suggestions (settings `suggestions_enabled` and
//! `activity_enabled`, both on by default): once a minute, a
//! background thread gathers what [`mimo_core::suggest::next`] needs —
//! the last weeks of activity, what's in front, running apps, CPU/memory,
//! the battery, the recycle bin —
//! and, if something is worth saying, brings the pill up *without taking
//! focus* (so typing elsewhere isn't interrupted) with the suggestion and
//! its buttons. What was shown/snoozed/muted is kept in `suggestions.json`.

use std::collections::{HashSet, VecDeque};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Local;
use mimo_core::suggest::{
    self, Action, Battery, Context, History, Hog, LoadReading, RecycleBin, Suggestion, ROUTINE_DAYS,
};
use mimo_core::{Engine, Settings};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

use crate::activity::{midnight, Activity};

const CHECK_EVERY: Duration = Duration::from_secs(60);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Readings kept for spotting a sustained strain.
const READINGS: usize = 5;
/// Heaviest apps considered for closing.
const HOGS: usize = 8;
/// Sizing up the recycle bin walks through it: not every minute.
const RECYCLE_BIN_EVERY: Duration = Duration::from_secs(15 * 60);

pub struct Suggestions {
    history: Mutex<History>,
    /// The suggestion on screen, awaiting an answer.
    pending: Mutex<Option<Suggestion>>,
    path: Option<PathBuf>,
}

impl Suggestions {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_data_dir().ok().map(|dir| dir.join("suggestions.json"));
        let history = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { history: Mutex::new(history), pending: Mutex::new(None), path }
    }

    /// Starts the once-a-minute check.
    pub fn start(app: AppHandle) {
        let _ = std::thread::Builder::new().name("mimo-suggestions".into()).spawn(move || {
            let mut system = System::new();
            let mut readings: VecDeque<LoadReading> = VecDeque::new();
            let mut recycle_bin: Option<(Instant, Option<RecycleBin>)> = None;
            loop {
                std::thread::sleep(CHECK_EVERY);
                let wanted = app.state::<Mutex<Settings>>().lock().expect("settings mutex poisoned").suggestions_enabled;
                if !wanted || !app.state::<Activity>().is_enabled() {
                    readings.clear();
                    continue;
                }
                // CPU usage is measured between two refreshes: here, over
                // the last minute.
                system.refresh_cpu_usage();
                system.refresh_memory();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing().with_memory().with_cpu(),
                );
                let memory_percent = if system.total_memory() == 0 {
                    0.0
                } else {
                    system.used_memory() as f32 * 100.0 / system.total_memory() as f32
                };
                readings.push_back(LoadReading { cpu: system.global_cpu_usage(), memory_percent });
                while readings.len() > READINGS {
                    readings.pop_front();
                }
                if recycle_bin.as_ref().is_none_or(|(at, _)| at.elapsed() >= RECYCLE_BIN_EVERY) {
                    recycle_bin = Some((Instant::now(), recycle_bin_contents()));
                }
                let bin = recycle_bin.as_ref().and_then(|(_, bin)| *bin);
                check(&app, &system, readings.make_contiguous(), memory_percent, bin);
            }
        });
    }

    /// Erase memory: forget what was shown or turned down.
    pub fn forget(&self) {
        *self.history.lock().expect("suggestions mutex poisoned") = History::default();
        *self.pending.lock().expect("suggestions mutex poisoned") = None;
    }

    fn save(&self, history: &History) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(history) {
            let _ = std::fs::write(path, json);
        }
    }

    /// The user's answer to the pending suggestion: `"accept"`, `"later"`,
    /// `"never"` or `"dismiss"`. Returns what the pill should say, if
    /// anything.
    pub fn answer(&self, app: &AppHandle, choice: &str) -> Option<String> {
        let suggestion = self.pending.lock().expect("suggestions mutex poisoned").take()?;
        let now = Local::now().timestamp();
        let mut history = self.history.lock().expect("suggestions mutex poisoned");
        match choice {
            "later" => history.snooze(&suggestion, now),
            "never" => history.mute(&suggestion),
            _ => {}
        }
        self.save(&history);
        drop(history);

        let action = suggestion.action.filter(|_| choice == "accept")?;
        let lang = app.state::<Mutex<Engine>>().lock().expect("engine mutex poisoned").language();
        match run(&action) {
            Ok(()) => Some(suggest::accepted_reply(&action, lang)),
            Err(err) => {
                eprintln!("[suggestions] {err}");
                Some(match lang {
                    mimo_core::Lang::Fr => "Ça n'a pas marché.".to_string(),
                    mimo_core::Lang::En => "That didn't work.".to_string(),
                })
            }
        }
    }
}

fn check(
    app: &AppHandle,
    system: &System,
    readings: &[LoadReading],
    memory_percent: f32,
    recycle_bin: Option<RecycleBin>,
) {
    // The pill is in use (or a suggestion is already up): not now.
    let busy = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
    let suggestions = app.state::<Suggestions>();
    if busy || suggestions.pending.lock().expect("suggestions mutex poisoned").is_some() {
        return;
    }

    let activity = app.state::<Activity>();
    let now = Local::now().timestamp();
    let day_start = midnight(0);
    let sessions = activity.sessions(day_start - ROUTINE_DAYS * 86_400, now);
    let in_front = activity.in_front();
    let running: HashSet<String> = system
        .processes()
        .values()
        .map(|p| exe_key(&p.name().to_string_lossy()))
        .collect();
    let strain = suggest::strain(readings);
    let hogs = if strain.is_some() { hogs(system) } else { Vec::new() };
    let (lang, apps) = {
        let engine = app.state::<Mutex<Engine>>();
        let engine = engine.lock().expect("engine mutex poisoned");
        (engine.language(), engine.installed_apps().clone())
    };

    let ctx = Context {
        now,
        day_start,
        lang,
        sessions: &sessions,
        in_front: in_front.as_ref(),
        running: &running,
        strain,
        memory_percent,
        hogs: &hogs,
        apps: &apps,
        battery: battery(),
        recycle_bin,
    };
    let mut history = suggestions.history.lock().expect("suggestions mutex poisoned");
    let Some(suggestion) = suggest::next(&ctx, &history) else { return };
    eprintln!("[suggestions] {:?}: {}", suggestion.keys, suggestion.message);
    history.record_shown(&suggestion, now);
    suggestions.save(&history);
    drop(history);
    *suggestions.pending.lock().expect("suggestions mutex poisoned") = Some(suggestion.clone());
    present(app, &suggestion);
}

/// Keys of the running processes ("chrome"), as activity names apps.
pub(crate) fn running_apps() -> HashSet<String> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system.processes().values().map(|p| exe_key(&p.name().to_string_lossy())).collect()
}

/// "Chrome.exe" → "chrome", matching activity's app keys.
fn exe_key(name: &str) -> String {
    let lower = name.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// Running apps by total memory (all their processes added up).
fn hogs(system: &System) -> Vec<Hog> {
    let cores = system.cpus().len().max(1) as f32;
    let mut by_name: std::collections::HashMap<String, Hog> = std::collections::HashMap::new();
    for process in system.processes().values() {
        let name = process.name().to_string_lossy().to_string();
        let hog = by_name
            .entry(name.to_lowercase())
            .or_insert_with(|| Hog { process: name, memory: 0, cpu: 0.0 });
        hog.memory += process.memory();
        hog.cpu += process.cpu_usage() / cores;
    }
    let mut hogs: Vec<Hog> = by_name.into_values().collect();
    hogs.sort_by_key(|h| std::cmp::Reverse(h.memory));
    hogs.truncate(HOGS);
    hogs
}

/// Brings the pill up with the suggestion, without stealing focus.
fn present(app: &AppHandle, suggestion: &Suggestion) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.set_focusable(false);
    crate::place_island(&window);
    let _ = window.show();
    let _ = window.emit("mimo://suggestion", suggestion);
}

/// Carries out an accepted suggestion. Apps are launched by the id Windows
/// lists them under and closed by executable name (politely: like clicking
/// their ×, so they can ask to save) — both one argument, no shell.
fn run(action: &Action) -> Result<(), String> {
    match action {
        Action::Launch { apps } => {
            for app in apps {
                std::process::Command::new("explorer.exe")
                    .arg(format!("shell:AppsFolder\\{}", app.app_id))
                    .spawn()
                    .map_err(|err| format!("can't launch {}: {err}", app.name))?;
            }
            Ok(())
        }
        Action::Close { process, .. } => {
            let status = std::process::Command::new("taskkill")
                .args(["/IM", process])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|err| format!("can't run taskkill: {err}"))?;
            status.success().then_some(()).ok_or_else(|| format!("taskkill {process} failed: {status}"))
        }
        Action::EmptyRecycleBin => {
            const SHERB_NOCONFIRMATION: u32 = 0x1;
            const SHERB_NOPROGRESSUI: u32 = 0x2;
            // All drives; Windows plays its usual "emptied" sound.
            let result = unsafe {
                SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI)
            };
            (result >= 0).then_some(()).ok_or_else(|| format!("emptying the recycle bin failed: {result:#x}"))
        }
    }
}

/// The battery's charge, from `GetSystemPowerStatus` (instant, no WMI).
/// `None` on a desktop without a battery, or when Windows doesn't know.
fn battery() -> Option<Battery> {
    const NO_BATTERY: u8 = 128;
    const UNKNOWN: u8 = 255;
    let mut status = SystemPowerStatus::default();
    if unsafe { GetSystemPowerStatus(&mut status) } == 0
        || status.battery_flag & NO_BATTERY != 0
        || status.battery_flag == UNKNOWN
        || status.battery_life_percent > 100
    {
        return None;
    }
    Some(Battery { percent: status.battery_life_percent, plugged: status.ac_line_status == 1 })
}

/// What's in the recycle bin, all drives together.
fn recycle_bin_contents() -> Option<RecycleBin> {
    let mut info = ShQueryRbInfo { size: std::mem::size_of::<ShQueryRbInfo>() as u32, ..Default::default() };
    let result = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
    (result >= 0).then(|| RecycleBin { bytes: info.bytes.max(0) as u64, items: info.items.max(0) as u64 })
}

#[repr(C)]
#[derive(Default)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[repr(C)]
#[derive(Default)]
struct ShQueryRbInfo {
    size: u32,
    bytes: i64,
    items: i64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn SHQueryRecycleBinW(root_path: *const u16, info: *mut ShQueryRbInfo) -> i32;
    fn SHEmptyRecycleBinW(hwnd: *mut std::ffi::c_void, root_path: *const u16, flags: u32) -> i32;
}

#[cfg(test)]
mod tests {
    /// `cargo test -p desktop print_power_and_recycle_bin -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_power_and_recycle_bin() {
        println!("battery: {:?}", super::battery());
        println!("recycle bin: {:?}", super::recycle_bin_contents());
    }
}
