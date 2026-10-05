//! Suggestions Mimo makes on its own, from what it has learned
//! ([`crate::activity`]) and the PC's load: close an app that's choking the
//! PC, launch the apps usually opened around this time, plug in a low
//! battery, empty an overflowing recycle bin, go to bed, take a break. [`next`] decides what (if anything) is worth interrupting for
//! right now; [`History`] remembers what was shown, snoozed or muted so
//! Mimo never nags. Pure: the shell gathers the [`Context`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::activity::{format_duration, Observation, Session, IDLE_AFTER_SECS};
use crate::apps::AppCatalog;
use crate::info::Lang;
use crate::intent::tokenize;

const MINUTE: i64 = 60;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

/// At most this many suggestions a day…
pub const MAX_PER_DAY: usize = 6;
/// …and never two within this long.
pub const MIN_GAP_SECS: i64 = 20 * MINUTE;

/// A break is suggested after this long at the PC without one…
pub const BREAK_AFTER_SECS: i64 = 2 * HOUR;
/// …a break being at least this long away (idle or PC off).
pub const BREAK_MIN_SECS: i64 = 5 * MINUTE;

/// Routines are learned from this many past days…
pub const ROUTINE_DAYS: i64 = 14;
/// …and need the app first opened on at least this many of them…
pub const ROUTINE_MIN_DAYS: usize = 4;
/// …within this many minutes of its usual time.
const ROUTINE_SPREAD_MIN: i64 = 45;
/// Offered from a little before the usual time to a while after.
const ROUTINE_EARLY_MIN: i64 = 10;
const ROUTINE_LATE_MIN: i64 = 30;
/// A day only counts if the app was used at least this long.
const ROUTINE_MIN_USE_SECS: i64 = 2 * MINUTE;

/// CPU or memory at/above this percentage…
pub const STRAIN_PERCENT: f32 = 90.0;
/// …for this many readings in a row (one a minute) means the PC struggles.
pub const STRAIN_READINGS: usize = 3;

/// "Late" is from 1:00 to 5:00, local time.
const LATE_FROM_MIN: i64 = 60;
const LATE_UNTIL_MIN: i64 = 5 * 60;

/// How long before the same thing can be suggested again.
fn cooldown(key: &str) -> i64 {
    match key.split(':').next().unwrap_or_default() {
        "break" => HOUR,
        "late" => 20 * HOUR,
        "routine" => 20 * HOUR,
        "close" => 2 * HOUR,
        "battery" => 30 * MINUTE,
        "recycle" => 3 * DAY,
        _ => DAY,
    }
}

/// "Later" puts a suggestion off this long.
pub const SNOOZE_SECS: i64 = HOUR;

/// Battery levels (percent) worth a word while unplugged.
pub const BATTERY_LOW: u8 = 20;
pub const BATTERY_CRITICAL: u8 = 10;
/// A recycle bin this big (or with this many items) is worth emptying.
pub const RECYCLE_BIN_FULL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const RECYCLE_BIN_FULL_ITEMS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    /// Plugged in (charging or full).
    pub plugged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecycleBin {
    pub bytes: u64,
    pub items: u64,
}

/// One CPU/memory reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadReading {
    pub cpu: f32,
    pub memory_percent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strain {
    Cpu,
    Memory,
}

/// Whether the last [`STRAIN_READINGS`] readings (oldest first) all show
/// the PC maxed out — memory first, since that's what makes it crawl.
pub fn strain(readings: &[LoadReading]) -> Option<Strain> {
    let recent = readings.get(readings.len().checked_sub(STRAIN_READINGS)?..)?;
    if recent.iter().all(|r| r.memory_percent >= STRAIN_PERCENT) {
        Some(Strain::Memory)
    } else if recent.iter().all(|r| r.cpu >= STRAIN_PERCENT) {
        Some(Strain::Cpu)
    } else {
        None
    }
}

/// A running app's total footprint (all its processes).
#[derive(Debug, Clone, PartialEq)]
pub struct Hog {
    /// Executable file name, as given to `taskkill` ("chrome.exe").
    pub process: String,
    /// Bytes.
    pub memory: u64,
    /// Share of the whole CPU, 0–100.
    pub cpu: f32,
}

/// What Mimo knows at the moment it considers suggesting something.
pub struct Context<'a> {
    pub now: i64,
    /// Local midnight today.
    pub day_start: i64,
    pub lang: Lang,
    /// The last [`ROUTINE_DAYS`] days, the session in progress included.
    pub sessions: &'a [Session],
    /// What's in front (`None`: nothing, or not sampled lately).
    pub in_front: Option<&'a Observation>,
    /// Running executables, lowercase, without ".exe".
    pub running: &'a HashSet<String>,
    pub strain: Option<Strain>,
    pub memory_percent: f32,
    /// Heaviest running apps (only needed under strain).
    pub hogs: &'a [Hog],
    pub apps: &'a AppCatalog,
    /// `None` on a PC without a battery.
    pub battery: Option<Battery>,
    pub recycle_bin: Option<RecycleBin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Action {
    /// Launch Start menu apps by their Windows app id.
    Launch { apps: Vec<LaunchTarget> },
    /// Ask an app to close (like clicking its ×), by executable name.
    Close { label: String, process: String },
    /// Empty the recycle bin (all drives).
    #[serde(rename = "empty_recycle_bin")]
    EmptyRecycleBin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchTarget {
    pub name: String,
    pub app_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// What it's about, for muting/snoozing: "break", "late",
    /// "routine:spotify", "close:chrome" (one per app).
    pub keys: Vec<String>,
    pub message: String,
    /// What "yes" does; `None`: it's advice, "OK" just dismisses it.
    pub action: Option<Action>,
}

/// What was shown, put off or turned down — persisted by the shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// "Don't suggest this again."
    pub muted: BTreeSet<String>,
    pub last_shown: BTreeMap<String, i64>,
    pub snoozed_until: BTreeMap<String, i64>,
    /// When each suggestion was shown, for the daily cap (recent ones only).
    pub shown_at: Vec<i64>,
}

impl History {
    pub fn record_shown(&mut self, suggestion: &Suggestion, now: i64) {
        for key in &suggestion.keys {
            self.last_shown.insert(key.clone(), now);
        }
        self.shown_at.push(now);
        self.shown_at.retain(|at| now - at < DAY);
        self.snoozed_until.retain(|_, until| *until > now);
    }

    pub fn snooze(&mut self, suggestion: &Suggestion, now: i64) {
        for key in &suggestion.keys {
            self.snoozed_until.insert(key.clone(), now + SNOOZE_SECS);
        }
    }

    pub fn mute(&mut self, suggestion: &Suggestion) {
        self.muted.extend(suggestion.keys.iter().cloned());
    }

    fn allows(&self, key: &str, now: i64) -> bool {
        !self.muted.contains(key)
            && self.snoozed_until.get(key).is_none_or(|until| now >= *until)
            && self.last_shown.get(key).is_none_or(|at| now - at >= cooldown(key))
    }
}

/// The suggestion worth showing now, if any. Never while the user is away,
/// in a fullscreen app (game, video), within [`MIN_GAP_SECS`] of the last
/// one, or past [`MAX_PER_DAY`] — except a low battery, which can't wait.
pub fn next(ctx: &Context, history: &History) -> Option<Suggestion> {
    let present = ctx.in_front?;
    if present.idle_secs >= IDLE_AFTER_SECS {
        return None;
    }
    let allowed = |s: &Suggestion| s.keys.iter().all(|key| history.allows(key, ctx.now));
    if let Some(battery) = low_battery(ctx).filter(|s| allowed(s)) {
        return Some(battery);
    }
    if present.fullscreen {
        return None;
    }
    let today = history.shown_at.iter().filter(|at| **at >= ctx.day_start).count();
    if today >= MAX_PER_DAY || history.shown_at.iter().any(|at| ctx.now - at < MIN_GAP_SECS) {
        return None;
    }

    close_hog(ctx, history)
        .or_else(|| routine(ctx, history))
        .filter(|s| allowed(s))
        .or_else(|| full_recycle_bin(ctx).filter(|s| allowed(s)))
        .or_else(|| late_night(ctx).filter(|s| allowed(s)))
        .or_else(|| take_a_break(ctx).filter(|s| allowed(s)))
}

/// Unplugged and running low: plug it in. Critical gets its own key so it
/// still comes up after "later" on the first warning.
fn low_battery(ctx: &Context) -> Option<Suggestion> {
    let battery = ctx.battery.filter(|b| !b.plugged && b.percent <= BATTERY_LOW)?;
    let critical = battery.percent <= BATTERY_CRITICAL;
    let percent = battery.percent;
    let message = match (ctx.lang, critical) {
        (Lang::Fr, false) => format!("Batterie à {percent} % — pense à brancher ton chargeur."),
        (Lang::Fr, true) => format!("Batterie presque vide ({percent} %) ! Branche ton PC pour ne rien perdre."),
        (Lang::En, false) => format!("Battery at {percent}% — time to plug in your charger."),
        (Lang::En, true) => format!("Battery almost empty ({percent}%)! Plug in now so you don't lose anything."),
    };
    let key = if critical { "battery:critical" } else { "battery:low" };
    Some(Suggestion { keys: vec![key.into()], message, action: None })
}

/// The recycle bin has piled up: offer to empty it.
fn full_recycle_bin(ctx: &Context) -> Option<Suggestion> {
    let bin = ctx
        .recycle_bin
        .filter(|b| b.bytes >= RECYCLE_BIN_FULL_BYTES || b.items >= RECYCLE_BIN_FULL_ITEMS)?;
    let gb = bin.bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let size = if gb >= 1.0 { format!("{gb:.1} Go") } else { format!("{:.0} Mo", gb * 1024.0) };
    let message = match ctx.lang {
        Lang::Fr => format!(
            "Ta corbeille déborde : {} dans {} éléments. Je la vide ? (C'est définitif.)",
            size.replace('.', ","),
            bin.items
        ),
        Lang::En => format!(
            "Your recycle bin is piling up: {} in {} items. Empty it? (This is permanent.)",
            size.replace("Go", "GB").replace("Mo", "MB"),
            bin.items
        ),
    };
    Some(Suggestion { keys: vec!["recycle".into()], message, action: Some(Action::EmptyRecycleBin) })
}

/// Labels of apps seen in front lately, by key.
fn known_apps(sessions: &[Session]) -> HashMap<&str, &str> {
    sessions.iter().map(|s| (s.app.as_str(), s.label.as_str())).collect()
}

/// The PC is maxed out: offer to close the heaviest app the user actually
/// uses (never the one in front, never Windows' own processes).
fn close_hog(ctx: &Context, history: &History) -> Option<Suggestion> {
    let strain = ctx.strain?;
    let known = known_apps(ctx.sessions);
    let front = ctx.in_front.map(|o| o.app.as_str());
    let mut candidates: Vec<(&Hog, String, &str)> = ctx
        .hogs
        .iter()
        .filter_map(|hog| {
            let key = hog.process.to_lowercase().trim_end_matches(".exe").to_string();
            let label = *known.get(key.as_str())?;
            (Some(key.as_str()) != front && history.allows(&format!("close:{key}"), ctx.now))
                .then_some((hog, key, label))
        })
        .collect();
    match strain {
        Strain::Memory => candidates.sort_by_key(|(hog, ..)| std::cmp::Reverse(hog.memory)),
        Strain::Cpu => candidates.sort_by(|(a, ..), (b, ..)| b.cpu.total_cmp(&a.cpu)),
    }
    let (hog, key, label) = candidates.into_iter().next()?;
    let gb = hog.memory as f64 / (1024.0 * 1024.0 * 1024.0);
    let message = match (ctx.lang, strain) {
        (Lang::Fr, Strain::Memory) => format!(
            "Ton PC sature (mémoire à {:.0} %). {label} utilise {} Go — je le ferme ?",
            ctx.memory_percent,
            format!("{gb:.1}").replace('.', ",")
        ),
        (Lang::En, Strain::Memory) => format!(
            "Your PC is struggling (memory at {:.0}%). {label} is using {gb:.1} GB — close it?",
            ctx.memory_percent
        ),
        (Lang::Fr, Strain::Cpu) => {
            format!("Ton processeur tourne à fond depuis quelques minutes, surtout à cause de {label}. Je le ferme ?")
        }
        (Lang::En, Strain::Cpu) => {
            format!("Your CPU has been maxed out for a few minutes, mostly by {label}. Close it?")
        }
    };
    Some(Suggestion {
        keys: vec![format!("close:{key}")],
        message,
        action: Some(Action::Close { label: label.to_string(), process: hog.process.clone() }),
    })
}

/// When `app` was first used each past day (minutes after midnight), for
/// days it was used at least [`ROUTINE_MIN_USE_SECS`].
fn first_uses<'a>(ctx: &Context<'a>) -> HashMap<&'a str, Vec<i64>> {
    let mut by_app: HashMap<&str, Vec<i64>> = HashMap::new();
    for days_ago in 1..=ROUTINE_DAYS {
        let start = ctx.day_start - days_ago * DAY;
        let end = start + DAY;
        let mut first: HashMap<&str, i64> = HashMap::new();
        let mut used: HashMap<&str, i64> = HashMap::new();
        for s in ctx.sessions.iter().filter(|s| s.start >= start && s.start < end) {
            let at = first.entry(s.app.as_str()).or_insert(s.start);
            *at = (*at).min(s.start);
            *used.entry(s.app.as_str()).or_default() += s.end - s.start;
        }
        for (app, at) in first {
            if used[app] >= ROUTINE_MIN_USE_SECS {
                by_app.entry(app).or_default().push((at - start) / MINUTE);
            }
        }
    }
    by_app
}

/// The usual time (minutes after midnight) `minutes` cluster around, if
/// enough of them agree.
fn usual_time(minutes: &[i64]) -> Option<i64> {
    let mut sorted = minutes.to_vec();
    sorted.sort_unstable();
    let median = *sorted.get(sorted.len() / 2)?;
    let close = sorted.iter().filter(|m| (*m - median).abs() <= ROUTINE_SPREAD_MIN).count();
    (close >= ROUTINE_MIN_DAYS).then_some(median)
}

/// Apps usually opened around now that aren't open yet today: offer to
/// launch them (all at once if several are due).
fn routine(ctx: &Context, history: &History) -> Option<Suggestion> {
    let now_min = (ctx.now - ctx.day_start) / MINUTE;
    let used_today: HashSet<&str> =
        ctx.sessions.iter().filter(|s| s.end > ctx.day_start).map(|s| s.app.as_str()).collect();
    let known = known_apps(ctx.sessions);

    let mut due: Vec<(&str, i64, LaunchTarget)> = first_uses(ctx)
        .into_iter()
        .filter_map(|(app, minutes)| {
            let usual = usual_time(&minutes)?;
            let in_window = now_min >= usual - ROUTINE_EARLY_MIN && now_min <= usual + ROUTINE_LATE_MIN;
            if !in_window
                || used_today.contains(app)
                || ctx.running.contains(app)
                || !history.allows(&format!("routine:{app}"), ctx.now)
            {
                return None;
            }
            let label = known.get(app)?;
            let words = tokenize(label);
            let installed = ctx.apps.exact(&words).or_else(|| ctx.apps.partial(&words))?;
            Some((app, usual, LaunchTarget { name: installed.name.clone(), app_id: installed.id.clone() }))
        })
        .collect();
    if due.is_empty() {
        return None;
    }
    due.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.name.cmp(&b.2.name)));
    due.truncate(3);

    let usual = due[0].1;
    let names: Vec<&str> = due.iter().map(|(_, _, t)| t.name.as_str()).collect();
    let message = match ctx.lang {
        Lang::Fr => format!(
            "Tu lances souvent {} vers {}. Je {} ?",
            join(&names, "et"),
            clock(usual, Lang::Fr),
            if names.len() == 1 { "l'ouvre" } else { "les ouvre" }
        ),
        Lang::En => format!(
            "You often open {} around {}. Want me to open {}?",
            join(&names, "and"),
            clock(usual, Lang::En),
            if names.len() == 1 { "it" } else { "them" }
        ),
    };
    Some(Suggestion {
        keys: due.iter().map(|(app, ..)| format!("routine:{app}")).collect(),
        message,
        action: Some(Action::Launch { apps: due.into_iter().map(|(_, _, t)| t).collect() }),
    })
}

/// Still at the PC in the middle of the night.
fn late_night(ctx: &Context) -> Option<Suggestion> {
    let now_min = (ctx.now - ctx.day_start) / MINUTE;
    if !(LATE_FROM_MIN..LATE_UNTIL_MIN).contains(&now_min) {
        return None;
    }
    let time = clock(now_min, ctx.lang);
    let message = match ctx.lang {
        Lang::Fr => format!("Il est {time} — il serait peut-être temps d'aller dormir."),
        Lang::En => format!("It's {time} — maybe time to get some sleep?"),
    };
    Some(Suggestion { keys: vec!["late".into()], message, action: None })
}

/// How long the user has been at the PC without a [`BREAK_MIN_SECS`] gap.
pub fn time_without_break(sessions: &[Session], now: i64) -> i64 {
    let mut sorted: Vec<&Session> = sessions.iter().filter(|s| s.start <= now).collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.end));
    let Some(latest) = sorted.first() else { return 0 };
    if now - latest.end >= BREAK_MIN_SECS {
        return 0;
    }
    let mut since = latest.start;
    for s in &sorted[1..] {
        if since - s.end >= BREAK_MIN_SECS {
            break;
        }
        since = since.min(s.start);
    }
    now - since
}

fn take_a_break(ctx: &Context) -> Option<Suggestion> {
    let at_it = time_without_break(ctx.sessions, ctx.now);
    if at_it < BREAK_AFTER_SECS {
        return None;
    }
    let duration = format_duration(at_it, ctx.lang);
    let message = match ctx.lang {
        Lang::Fr => format!("Ça fait {duration} que tu es devant l'écran sans pause. Une petite pause ?"),
        Lang::En => format!("You've been at your screen for {duration} without a break. Time for a short one?"),
    };
    Some(Suggestion { keys: vec!["break".into()], message, action: None })
}

/// "21 h", "21 h 30" / "9 pm", "9:30 pm" (minutes after midnight, rounded
/// to a quarter hour).
fn clock(minutes: i64, lang: Lang) -> String {
    let minutes = ((minutes + 7) / 15 * 15).rem_euclid(24 * 60);
    let (h, m) = (minutes / 60, minutes % 60);
    match lang {
        Lang::Fr if m == 0 => format!("{h} h"),
        Lang::Fr => format!("{h} h {m:02}"),
        Lang::En => {
            let suffix = if h < 12 { "am" } else { "pm" };
            let h12 = if h % 12 == 0 { 12 } else { h % 12 };
            if m == 0 { format!("{h12} {suffix}") } else { format!("{h12}:{m:02} {suffix}") }
        }
    }
}

pub(crate) fn join(names: &[&str], and: &str) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} {and} {last}", rest.join(", ")),
    }
}

/// What the pill says once a suggestion was accepted.
pub fn accepted_reply(action: &Action, lang: Lang) -> String {
    match (action, lang) {
        (Action::Launch { apps }, Lang::Fr) => {
            format!("Ouverture de {}…", join(&apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), "et"))
        }
        (Action::Launch { apps }, Lang::En) => {
            format!("Opening {}…", join(&apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), "and"))
        }
        (Action::Close { label, .. }, Lang::Fr) => format!("Fermeture de {label}…"),
        (Action::Close { label, .. }, Lang::En) => format!("Closing {label}…"),
        (Action::EmptyRecycleBin, Lang::Fr) => "Corbeille vidée.".to_string(),
        (Action::EmptyRecycleBin, Lang::En) => "Recycle bin emptied.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::InstalledApp;

    const MIDNIGHT: i64 = 100 * DAY;

    fn session(app: &str, start: i64, end: i64) -> Session {
        Session { app: app.into(), label: crate::activity::app_label(app, None), title: String::new(), start, end }
    }

    fn front(app: &str) -> Observation {
        Observation { app: app.into(), label: app.into(), title: String::new(), idle_secs: 0, fullscreen: false }
    }

    fn catalog() -> AppCatalog {
        AppCatalog::new(
            [("Spotify", "SpotifyAB.SpotifyMusic!Spotify"), ("Discord", "com.squirrel.Discord.Discord")]
                .into_iter()
                .filter_map(|(name, id)| InstalledApp::new(name, id))
                .collect(),
        )
    }

    struct World {
        sessions: Vec<Session>,
        in_front: Observation,
        running: HashSet<String>,
        hogs: Vec<Hog>,
        apps: AppCatalog,
    }

    impl World {
        fn new() -> Self {
            Self {
                sessions: Vec::new(),
                in_front: front("code"),
                running: HashSet::new(),
                hogs: Vec::new(),
                apps: catalog(),
            }
        }

        fn at(&self, now: i64, strain: Option<Strain>, lang: Lang) -> Context<'_> {
            Context {
                now,
                day_start: MIDNIGHT,
                lang,
                sessions: &self.sessions,
                in_front: Some(&self.in_front),
                running: &self.running,
                strain,
                memory_percent: 94.0,
                hogs: &self.hogs,
                apps: &self.apps,
                battery: None,
                recycle_bin: None,
            }
        }
    }

    /// Spotify opened around 21:00 on the last `days` days.
    fn spotify_evenings(world: &mut World, days: i64) {
        for d in 1..=days {
            let start = MIDNIGHT - d * DAY + 21 * HOUR + (d % 3) * 10 * MINUTE;
            world.sessions.push(session("spotify", start, start + 30 * MINUTE));
        }
    }

    #[test]
    fn suggests_launching_a_routine_app_at_its_usual_time() {
        let mut world = World::new();
        spotify_evenings(&mut world, 6);
        let s = next(&world.at(MIDNIGHT + 21 * HOUR, None, Lang::Fr), &History::default()).unwrap();
        assert_eq!(s.message, "Tu lances souvent Spotify vers 21 h 15. Je l'ouvre ?");
        assert_eq!(s.keys, vec!["routine:spotify"]);
        assert_eq!(
            s.action,
            Some(Action::Launch {
                apps: vec![LaunchTarget { name: "Spotify".into(), app_id: "SpotifyAB.SpotifyMusic!Spotify".into() }]
            })
        );
        // Not at noon, not if already running or used today, not with too few days.
        assert_eq!(next(&world.at(MIDNIGHT + 12 * HOUR, None, Lang::Fr), &History::default()), None);
        world.running.insert("spotify".into());
        assert_eq!(next(&world.at(MIDNIGHT + 21 * HOUR, None, Lang::Fr), &History::default()), None);
        let mut world = World::new();
        spotify_evenings(&mut world, 3);
        assert_eq!(next(&world.at(MIDNIGHT + 21 * HOUR, None, Lang::Fr), &History::default()), None);
    }

    #[test]
    fn several_routine_apps_come_together() {
        let mut world = World::new();
        spotify_evenings(&mut world, 5);
        for d in 1..=5 {
            let start = MIDNIGHT - d * DAY + 21 * HOUR + 5 * MINUTE;
            world.sessions.push(session("discord", start, start + HOUR));
        }
        let s = next(&world.at(MIDNIGHT + 21 * HOUR, None, Lang::En), &History::default()).unwrap();
        assert_eq!(s.message, "You often open Discord and Spotify around 9 pm. Want me to open them?");
        assert_eq!(s.keys, vec!["routine:discord", "routine:spotify"]);
    }

    #[test]
    fn offers_to_close_the_app_choking_the_pc() {
        let mut world = World::new();
        world.sessions = vec![
            Session { label: "Google Chrome".into(), ..session("chrome", MIDNIGHT + 12 * HOUR, MIDNIGHT + 13 * HOUR) },
            session("code", MIDNIGHT + 13 * HOUR + 30 * MINUTE, MIDNIGHT + 14 * HOUR),
        ];
        world.hogs = vec![
            Hog { process: "Code.exe".into(), memory: 6 << 30, cpu: 5.0 },
            Hog { process: "chrome.exe".into(), memory: 4 << 30, cpu: 30.0 },
            Hog { process: "MsMpEng.exe".into(), memory: 8 << 30, cpu: 50.0 },
        ];
        let s = next(&world.at(MIDNIGHT + 14 * HOUR, Some(Strain::Memory), Lang::Fr), &History::default()).unwrap();
        // Not the app in front (VS Code), not Windows' own (Defender).
        assert_eq!(s.message, "Ton PC sature (mémoire à 94 %). Google Chrome utilise 4,0 Go — je le ferme ?");
        assert_eq!(s.action, Some(Action::Close { label: "Google Chrome".into(), process: "chrome.exe".into() }));
        assert_eq!(next(&world.at(MIDNIGHT + 14 * HOUR, None, Lang::Fr), &History::default()), None);
    }

    #[test]
    fn suggests_a_break_after_two_hours_straight() {
        let mut world = World::new();
        // Two hours with only short gaps, after a long break.
        world.sessions = vec![
            session("discord", MIDNIGHT + 8 * HOUR, MIDNIGHT + 9 * HOUR),
            session("code", MIDNIGHT + 10 * HOUR, MIDNIGHT + 11 * HOUR),
            session("chrome", MIDNIGHT + 11 * HOUR + 2 * MINUTE, MIDNIGHT + 12 * HOUR + 5 * MINUTE),
        ];
        let now = MIDNIGHT + 12 * HOUR + 5 * MINUTE;
        assert_eq!(time_without_break(&world.sessions, now), 2 * HOUR + 5 * MINUTE);
        let s = next(&world.at(now, None, Lang::Fr), &History::default()).unwrap();
        assert_eq!(s.message, "Ça fait 2 h 05 que tu es devant l'écran sans pause. Une petite pause ?");
        assert_eq!(s.action, None);
        // Back from a break: the count restarts.
        assert_eq!(time_without_break(&world.sessions, now + 10 * MINUTE), 0);
    }

    #[test]
    fn says_good_night() {
        let world = World::new();
        let s = next(&world.at(MIDNIGHT + HOUR + 20 * MINUTE, None, Lang::En), &History::default()).unwrap();
        assert_eq!(s.message, "It's 1:15 am — maybe time to get some sleep?");
        assert_eq!(next(&world.at(MIDNIGHT + 6 * HOUR, None, Lang::En), &History::default()), None);
    }

    #[test]
    fn never_interrupts_fullscreen_or_an_absent_user() {
        let mut world = World::new();
        let late = MIDNIGHT + 2 * HOUR;
        world.in_front.fullscreen = true;
        assert_eq!(next(&world.at(late, None, Lang::Fr), &History::default()), None);
        world.in_front = Observation { idle_secs: IDLE_AFTER_SECS, ..front("code") };
        assert_eq!(next(&world.at(late, None, Lang::Fr), &History::default()), None);
        let ctx = Context { in_front: None, ..world.at(late, None, Lang::Fr) };
        assert_eq!(next(&ctx, &History::default()), None);
    }

    #[test]
    fn history_spaces_out_snoozes_and_mutes() {
        let world = World::new();
        let late = MIDNIGHT + 2 * HOUR;
        let mut history = History::default();
        let s = next(&world.at(late, None, Lang::Fr), &history).unwrap();
        history.record_shown(&s, late);
        // Not again right away, nor later that night (cooldown).
        assert_eq!(next(&world.at(late + 30 * MINUTE, None, Lang::Fr), &history), None);
        // Snoozed, then muted for good.
        let mut history = History::default();
        history.snooze(&s, late);
        assert_eq!(next(&world.at(late + 30 * MINUTE, None, Lang::Fr), &history), None);
        assert!(next(&world.at(late + 61 * MINUTE, None, Lang::Fr), &history).is_some());
        history.mute(&s);
        assert_eq!(next(&world.at(late + 3 * HOUR, None, Lang::Fr), &history), None);
        // Daily cap.
        let history = History { shown_at: vec![MIDNIGHT + 1; MAX_PER_DAY], ..History::default() };
        assert_eq!(next(&world.at(late, None, Lang::Fr), &history), None);
    }

    #[test]
    fn detects_strain_and_words_times() {
        let busy = LoadReading { cpu: 97.0, memory_percent: 60.0 };
        let full = LoadReading { cpu: 20.0, memory_percent: 93.0 };
        assert_eq!(strain(&[busy, busy, busy]), Some(Strain::Cpu));
        assert_eq!(strain(&[busy, full, full, full]), Some(Strain::Memory));
        assert_eq!(strain(&[busy, busy]), None);
        assert_eq!(strain(&[busy, full, busy]), None);
        assert_eq!(clock(21 * 60 + 38, Lang::Fr), "21 h 45");
        assert_eq!(clock(9 * 60 + 2, Lang::En), "9 am");
        assert_eq!(clock(12 * 60 + 30, Lang::En), "12:30 pm");
    }

    #[test]
    fn low_battery_comes_first_even_in_fullscreen() {
        let world = World::new();
        let noon = MIDNIGHT + 12 * HOUR;
        let mut gaming = world.in_front.clone();
        gaming.fullscreen = true;
        let unplugged = Some(Battery { percent: 15, plugged: false });
        let ctx = Context { battery: unplugged, in_front: Some(&gaming), ..world.at(noon, None, Lang::Fr) };
        let s = next(&ctx, &History::default()).unwrap();
        assert_eq!(s.keys, vec!["battery:low".to_string()]);
        assert_eq!(s.message, "Batterie à 15 % — pense à brancher ton chargeur.");

        // Plugged in, or still fine: nothing to say about it.
        let plugged = Context { battery: Some(Battery { percent: 15, plugged: true }), ..world.at(noon, None, Lang::Fr) };
        assert!(next(&plugged, &History::default()).is_none_or(|s| !s.keys[0].starts_with("battery")));
        let fine = Context { battery: Some(Battery { percent: 60, plugged: false }), ..world.at(noon, None, Lang::Fr) };
        assert!(next(&fine, &History::default()).is_none_or(|s| !s.keys[0].starts_with("battery")));

        // Snoozed at 15 %, but critical at 8 % still warns.
        let mut history = History::default();
        history.snooze(&s, noon);
        let critical = Context { battery: Some(Battery { percent: 8, plugged: false }), ..world.at(noon + 10 * MINUTE, None, Lang::En) };
        assert_eq!(next(&critical, &history).unwrap().keys, vec!["battery:critical".to_string()]);
    }

    #[test]
    fn a_full_recycle_bin_can_be_emptied() {
        let world = World::new();
        let noon = MIDNIGHT + 12 * HOUR;
        let bin = Some(RecycleBin { bytes: 3 * 1024 * 1024 * 1024 + 300 * 1024 * 1024, items: 420 });
        let ctx = Context { recycle_bin: bin, ..world.at(noon, None, Lang::Fr) };
        let s = next(&ctx, &History::default()).unwrap();
        assert_eq!(s.action, Some(Action::EmptyRecycleBin));
        assert_eq!(s.message, "Ta corbeille déborde : 3,3 Go dans 420 éléments. Je la vide ? (C'est définitif.)");

        let small = Context { recycle_bin: Some(RecycleBin { bytes: 1024, items: 3 }), ..world.at(noon, None, Lang::Fr) };
        assert!(next(&small, &History::default()).is_none_or(|s| s.action != Some(Action::EmptyRecycleBin)));
    }
}
