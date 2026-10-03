//! What the user does on the PC, learned locally (setting `activity_enabled`, on by default): which app
//! is in front and for how long. The shell samples the foreground window
//! every few seconds and feeds it to a [`Tracker`], which turns samples
//! into [`Session`]s to store; the functions below add those up ("2 h on
//! Discord today") and understand questions about it. Times are unix
//! seconds, passed in, so everything here is testable.

use std::collections::HashMap;

use serde::Serialize;

use crate::info::Lang;
use crate::intent::tokenize;

/// No keyboard/mouse input for this long means the user stepped away —
/// unless the window is fullscreen (a video, a game with a controller).
pub const IDLE_AFTER_SECS: u64 = 120;
/// Sessions are cut at this length, so a crash or power loss never loses
/// more than a few minutes, and the store stays simple (no updates).
pub const MAX_SESSION_SECS: i64 = 300;
/// How long finished sessions are kept.
pub const RETENTION_DAYS: i64 = 30;

/// What's in front of the user at one moment, as sampled by the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Stable key: the executable's name, lowercase, without ".exe"
    /// ("discord").
    pub app: String,
    /// What to call it ("Discord").
    pub label: String,
    pub title: String,
    pub idle_secs: u64,
    pub fullscreen: bool,
}

/// One app continuously in front, from `start` to `end`. `title` is the
/// last window title seen (titles change all the time: tabs, songs,
/// spinners), so it only splits sessions per app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    pub app: String,
    pub label: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
}

/// Turns foreground samples into sessions.
#[derive(Debug, Clone)]
pub struct Tracker {
    current: Option<Session>,
    /// Longer than this between two samples: the PC slept or Mimo was
    /// paused, so the current session ended at its last sample.
    max_gap: i64,
}

impl Tracker {
    pub fn new(poll_secs: i64) -> Self {
        Self { current: None, max_gap: poll_secs * 3 }
    }

    /// The session in progress (not stored yet).
    pub fn current(&self) -> Option<&Session> {
        self.current.as_ref()
    }

    /// Feeds what's in front at `now` (`None`: nothing, e.g. the desktop or
    /// the lock screen). Returns the sessions that just ended, to store.
    pub fn observe(&mut self, now: i64, seen: Option<Observation>) -> Vec<Session> {
        let mut ended = Vec::new();
        if self.current.as_ref().is_some_and(|current| now - current.end > self.max_gap) {
            ended.extend(self.current.take());
        }
        let nothing_in_front = seen.is_none();
        let active = seen.filter(|o| o.fullscreen || o.idle_secs < IDLE_AFTER_SECS);

        match (&mut self.current, active) {
            (Some(current), Some(o)) if current.app == o.app && now - current.start < MAX_SESSION_SECS => {
                current.end = now;
                current.title = o.title;
            }
            (_, Some(o)) => {
                if let Some(mut previous) = self.current.take() {
                    // It stayed in front until this sample.
                    previous.end = now;
                    ended.push(previous);
                }
                self.current = Some(Session { app: o.app, label: o.label, title: o.title, start: now, end: now });
            }
            (_, None) => {
                if let Some(mut previous) = self.current.take() {
                    // Left for the desktop or the lock screen just now; gone
                    // idle, on the other hand, since its last activity.
                    if nothing_in_front {
                        previous.end = now;
                    }
                    ended.push(previous);
                }
            }
        }
        ended.retain(|s| s.end > s.start);
        ended
    }

    /// Ends the session in progress (tracking turned off, Mimo closing).
    pub fn finish(&mut self) -> Option<Session> {
        self.current.take().filter(|s| s.end > s.start)
    }
}

/// Windows' own plumbing that shows up in front without being something the
/// user works in (Start menu, search, lock screen, alt-tab…).
pub fn is_ignored(app: &str) -> bool {
    matches!(
        app,
        "lockapp"
            | "searchhost"
            | "searchapp"
            | "searchui"
            | "startmenuexperiencehost"
            | "shellexperiencehost"
            | "textinputhost"
            | "shellhost"
            | "logonui"
            | "consent"
    )
}

/// Generic processes that run other apps: the window title names the app
/// (Store apps, Java games such as Minecraft).
const HOSTS: &[&str] = &["applicationframehost", "javaw", "java"];

/// The stable key and display name of the app behind a window, from its
/// executable's name (lowercase, no ".exe"), file description and title.
/// `None` for what isn't an app the user works in (the desktop, Windows'
/// own overlays).
pub fn identify(exe: &str, description: Option<&str>, title: &str) -> Option<(String, String)> {
    if is_ignored(exe) {
        return None;
    }
    if HOSTS.contains(&exe) {
        let name = title.split(" - ").next().unwrap_or_default().trim();
        let key = tokenize(name).join(" ");
        return (!key.is_empty()).then(|| (key, name.to_string()));
    }
    // Explorer without a title is the desktop itself.
    if exe == "explorer" && title.trim().is_empty() {
        return None;
    }
    Some((exe.to_string(), app_label(exe, description)))
}

/// A window title as stored: private browsing windows are left blank, and
/// the app's name at the end ("… - Google Chrome") is dropped.
pub fn clean_title(title: &str, label: &str) -> String {
    let lower = title.to_lowercase();
    if ["inprivate", "incognito", "navigation privée", "private browsing", "fenêtre privée"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return String::new();
    }
    let title = title.trim();
    for separator in [" - ", " — ", " – "] {
        if let Some(rest) = title.strip_suffix(label).and_then(|rest| rest.strip_suffix(separator)) {
            return rest.trim().to_string();
        }
    }
    title.to_string()
}

/// Apps whose file description isn't what anyone calls them.
const LABELS: &[(&str, &str)] = &[
    ("windowsterminal", "Terminal"),
    ("code", "VS Code"),
    ("msedge", "Microsoft Edge"),
    ("winword", "Word"),
    ("excel", "Excel"),
    ("powerpnt", "PowerPoint"),
];

/// What to call an app: its usual name, its file description ("Google
/// Chrome"), or its executable name, capitalized.
pub fn app_label(app: &str, description: Option<&str>) -> String {
    if let Some((_, label)) = LABELS.iter().find(|(exe, _)| *exe == app) {
        return label.to_string();
    }
    if let Some(description) = description.map(str::trim).filter(|d| !d.is_empty()) {
        return description.to_string();
    }
    let mut chars = app.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppUsage {
    pub app: String,
    pub label: String,
    pub secs: i64,
}

/// Seconds of `session` within `from..to`.
fn overlap(session: &Session, from: i64, to: i64) -> i64 {
    (session.end.min(to) - session.start.max(from)).max(0)
}

/// Time per app within `from..to`, most used first.
pub fn usage_by_app(sessions: &[Session], from: i64, to: i64) -> Vec<AppUsage> {
    let mut by_app: HashMap<&str, AppUsage> = HashMap::new();
    let mut ordered: Vec<&Session> = sessions.iter().collect();
    ordered.sort_by_key(|s| s.start);
    for session in ordered {
        let secs = overlap(session, from, to);
        if secs == 0 {
            continue;
        }
        let usage = by_app
            .entry(&session.app)
            .or_insert_with(|| AppUsage { app: session.app.clone(), label: String::new(), secs: 0 });
        usage.secs += secs;
        // The latest name wins (an update may rename an app).
        usage.label = session.label.clone();
    }
    let mut usage: Vec<AppUsage> = by_app.into_values().collect();
    usage.sort_by(|a, b| b.secs.cmp(&a.secs).then_with(|| a.label.cmp(&b.label)));
    usage
}

/// Total active time within `from..to` (one app is in front at a time, so
/// sessions don't overlap).
pub fn total_secs(sessions: &[Session], from: i64, to: i64) -> i64 {
    sessions.iter().map(|s| overlap(s, from, to)).sum()
}

/// Active minutes in each of the `count` consecutive `bucket_secs`-long
/// buckets starting at `from` (e.g. 24 hours of a day, 7 days of a week —
/// the caller gives local midnights, so daylight saving is its business).
pub fn minutes_per_bucket(sessions: &[Session], from: i64, bucket_secs: i64, count: usize) -> Vec<u32> {
    (0..count as i64)
        .map(|i| {
            let start = from + i * bucket_secs;
            (total_secs(sessions, start, start + bucket_secs) / 60) as u32
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    Today,
    Week,
}

/// A question about the user's activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityQuery {
    /// "combien de temps sur Discord": the app as said.
    pub app: Option<String>,
    pub period: Period,
}

const PERIOD_WORDS: &[&str] = &["aujourd", "hui", "today", "cette", "semaine", "this", "week", "ce", "jour"];

/// "résumé de ma journée", "temps d'écran cette semaine", "combien de temps
/// j'ai passé sur Discord", "screen time", "how long have I been on Steam".
pub fn detect(tokens: &[String]) -> Option<ActivityQuery> {
    let has = |w: &str| tokens.iter().any(|t| t == w);
    let period = if has("semaine") || has("week") { Period::Week } else { Period::Today };

    let screen_time = (has("temps") && has("ecran")) || (has("screen") && has("time"));
    let summary = ["resume", "bilan", "recap", "recapitulatif", "summary", "summarize", "recap"]
        .iter()
        .any(|w| has(w))
        && ["journee", "semaine", "day", "week", "activite", "activity"].iter().any(|w| has(w));
    let my_activity = (has("activite") && ["mon", "ma", "montre", "affiche"].iter().any(|w| has(w)))
        || (has("activity") && ["my", "show"].iter().any(|w| has(w)));
    let how_long = ((has("combien") && has("temps")) && ["passe", "passer", "suis", "ete"].iter().any(|w| has(w)))
        || ((has("how") && (has("long") || has("much")))
            && ["spent", "spend", "been", "on"].iter().any(|w| has(w)));

    if how_long {
        let app = after_last(tokens, &["sur", "on", "dans"]).filter(|app| !app.is_empty());
        return Some(ActivityQuery { app, period });
    }
    (screen_time || summary || my_activity).then_some(ActivityQuery { app: None, period })
}

/// The words after the last of `prepositions`, minus period words.
fn after_last(tokens: &[String], prepositions: &[&str]) -> Option<String> {
    let at = tokens.iter().rposition(|t| prepositions.contains(&t.as_str()))?;
    let words: Vec<&str> = tokens[at + 1..]
        .iter()
        .map(String::as_str)
        .filter(|t| !PERIOD_WORDS.contains(t))
        .collect();
    Some(words.join(" "))
}

/// The app among `usage` that `said` refers to ("discord", "vs code",
/// "chrome" for "Google Chrome").
pub fn find_app<'a>(usage: &'a [AppUsage], said: &str) -> Option<&'a AppUsage> {
    let said = tokenize(said).join(" ");
    if said.is_empty() {
        return None;
    }
    let names = |u: &AppUsage| (tokenize(&u.label).join(" "), u.app.clone());
    usage
        .iter()
        .find(|u| {
            let (label, key) = names(u);
            label == said || key == said
        })
        .or_else(|| {
            usage.iter().find(|u| {
                let (label, key) = names(u);
                label.split(' ').any(|word| word == said) || label.contains(&said) || key.contains(&said)
            })
        })
}

/// "2 h 05", "45 min", "moins d'une minute".
pub fn format_duration(secs: i64, lang: Lang) -> String {
    let minutes = secs / 60;
    match (lang, minutes / 60, minutes % 60) {
        (Lang::Fr, 0, 0) => "moins d'une minute".to_string(),
        (Lang::En, 0, 0) => "under a minute".to_string(),
        (_, 0, m) => format!("{m} min"),
        (Lang::Fr, h, m) => format!("{h} h {m:02}"),
        (Lang::En, h, 0) => format!("{h} h"),
        (Lang::En, h, m) => format!("{h} h {m} min"),
    }
}

/// One line for the pill, about the whole period.
pub fn format_summary(usage: &[AppUsage], period: Period, lang: Lang) -> String {
    let total: i64 = usage.iter().map(|u| u.secs).sum();
    let when = match (lang, period) {
        (Lang::Fr, Period::Today) => "Aujourd'hui",
        (Lang::Fr, Period::Week) => "Ces 7 derniers jours",
        (Lang::En, Period::Today) => "Today",
        (Lang::En, Period::Week) => "Over the last 7 days",
    };
    if total < 60 {
        return match lang {
            Lang::Fr => format!("{when}, je n'ai encore presque rien vu passer."),
            Lang::En => format!("{when}, I've barely seen any activity yet."),
        };
    }
    let top: Vec<String> = usage
        .iter()
        .take(2)
        .filter(|u| u.secs >= 60)
        .map(|u| format!("{} ({})", u.label, format_duration(u.secs, lang)))
        .collect();
    let total = format_duration(total, lang);
    match (lang, top.as_slice()) {
        (Lang::Fr, [one]) => format!("{when} : {total} d'activité, surtout sur {one}."),
        (Lang::Fr, [one, two]) => format!("{when} : {total} d'activité, surtout sur {one} et {two}."),
        (Lang::Fr, _) => format!("{when} : {total} d'activité."),
        (Lang::En, [one]) => format!("{when}: {total} of activity, mostly {one}."),
        (Lang::En, [one, two]) => format!("{when}: {total} of activity, mostly {one} and {two}."),
        (Lang::En, _) => format!("{when}: {total} of activity."),
    }
}

/// The answer to "how long on <app>".
pub fn format_app_time(said: &str, found: Option<&AppUsage>, period: Period, lang: Lang) -> String {
    let when = match (lang, period) {
        (Lang::Fr, Period::Today) => "aujourd'hui",
        (Lang::Fr, Period::Week) => "ces 7 derniers jours",
        (Lang::En, Period::Today) => "today",
        (Lang::En, Period::Week) => "over the last 7 days",
    };
    match (found, lang) {
        (Some(u), Lang::Fr) => format!("{} : {} {when}.", u.label, format_duration(u.secs, lang)),
        (Some(u), Lang::En) => format!("{}: {} {when}.", u.label, format_duration(u.secs, lang)),
        (None, Lang::Fr) => format!("Je ne t'ai pas vu sur « {said} » {when}."),
        (None, Lang::En) => format!("I haven't seen you on “{said}” {when}."),
    }
}

/// When activity analysis is off.
pub fn disabled_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "L'analyse d'activité est désactivée — active-la dans les réglages de Mimo.",
        Lang::En => "Activity analysis is off — turn it on in Mimo's settings.",
    }
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "screen time", "my screen time", "what's my screen time", "screen time this week",
            "summary of my day", "summarize my day", "summary of my week", "my activity", "show my activity",
            "how long have i been on [unk]", "how much time have i spent on [unk]",
        ],
        _ => &[
            "temps d'écran", "mon temps d'écran", "temps d'écran cette semaine", "résumé de ma journée",
            "fais le résumé de ma journée", "bilan de ma journée", "résumé de ma semaine", "mon activité",
            "montre mon activité", "combien de temps j'ai passé sur [unk]",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(app: &str, title: &str) -> Option<Observation> {
        Some(Observation {
            app: app.to_string(),
            label: app_label(app, None),
            title: title.to_string(),
            idle_secs: 0,
            fullscreen: false,
        })
    }

    fn session(app: &str, start: i64, end: i64) -> Session {
        Session { app: app.into(), label: app_label(app, None), title: String::new(), start, end }
    }

    #[test]
    fn samples_become_sessions() {
        let mut tracker = Tracker::new(5);
        assert!(tracker.observe(0, seen("discord", "général")).is_empty());
        // A new title (another channel) is still the same session.
        assert!(tracker.observe(5, seen("discord", "jeux")).is_empty());
        let ended = tracker.observe(10, seen("code", "main.rs"));
        assert_eq!(ended, vec![Session { label: "Discord".into(), title: "jeux".into(), ..session("discord", 0, 10) }]);
        assert_eq!(tracker.current().map(|s| s.app.as_str()), Some("code"));
        // Nothing in front (desktop, lock screen) ends it at that sample.
        assert_eq!(tracker.observe(15, None).first().map(|s| s.end), Some(15));
        assert!(tracker.current().is_none());
    }

    #[test]
    fn idle_ends_a_session_unless_fullscreen() {
        let mut tracker = Tracker::new(5);
        tracker.observe(0, seen("chrome", "doc"));
        tracker.observe(5, seen("chrome", "doc"));
        let away = Observation { idle_secs: IDLE_AFTER_SECS, ..seen("chrome", "doc").unwrap() };
        assert_eq!(tracker.observe(10, Some(away.clone())), vec![Session { title: "doc".into(), ..session("chrome", 0, 5) }]);
        // A fullscreen video counts even without input.
        tracker.observe(15, Some(Observation { fullscreen: true, ..away.clone() }));
        tracker.observe(20, Some(Observation { fullscreen: true, ..away }));
        assert_eq!(tracker.current().map(|s| (s.start, s.end)), Some((15, 20)));
    }

    #[test]
    fn long_sessions_are_cut_and_gaps_close_them() {
        let mut tracker = Tracker::new(5);
        let mut ended = Vec::new();
        for now in (0..=MAX_SESSION_SECS + 5).step_by(5) {
            ended.extend(tracker.observe(now, seen("steam", "")));
        }
        assert_eq!(ended, vec![session("steam", 0, MAX_SESSION_SECS)]);
        // The PC slept: the session ended at its last sample.
        let last = tracker.current().unwrap().end;
        let ended = tracker.observe(last + 3600, seen("steam", ""));
        assert_eq!(ended.first().map(|s| s.end), Some(last));
        assert_eq!(tracker.current().map(|s| s.start), Some(last + 3600));
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("Mimo — README.md - Google Chrome", "Google Chrome"), "Mimo — README.md");
        assert_eq!(clean_title("Nouvel onglet - Navigation InPrivate - Microsoft Edge", "Microsoft Edge"), "");
        assert_eq!(clean_title("général | Serveur", "Discord"), "général | Serveur");
        assert_eq!(app_label("discord", Some(" ")), "Discord");
        assert_eq!(app_label("chrome", Some("Google Chrome")), "Google Chrome");
        assert_eq!(app_label("windowsterminal", Some("Windows Terminal Host")), "Terminal");
        assert!(is_ignored("lockapp") && !is_ignored("discord"));
    }

    #[test]
    fn identifies_the_app_behind_a_window() {
        assert_eq!(identify("chrome", Some("Google Chrome"), "x"), Some(("chrome".into(), "Google Chrome".into())));
        assert_eq!(identify("applicationframehost", None, "Calculatrice"), Some(("calculatrice".into(), "Calculatrice".into())));
        assert_eq!(identify("javaw", Some("Java(TM) Platform SE binary"), "Minecraft 1.21 - Multijoueur"),
            Some(("minecraft 1.21".into(), "Minecraft 1.21".into())));
        assert_eq!(identify("explorer", Some("Explorateur Windows"), ""), None);
        assert_eq!(identify("explorer", Some("Explorateur Windows"), "Téléchargements").map(|(k, _)| k), Some("explorer".into()));
        assert_eq!(identify("searchhost", None, "Rechercher"), None);
    }

    #[test]
    fn usage_adds_up_within_the_period() {
        let sessions = [
            session("discord", 0, 3600),
            session("code", 3600, 5400),
            session("discord", 5400, 6000),
            session("steam", 90_000, 95_000),
        ];
        let usage = usage_by_app(&sessions, 0, 86_400);
        assert_eq!(
            usage.iter().map(|u| (u.app.as_str(), u.secs)).collect::<Vec<_>>(),
            vec![("discord", 4200), ("code", 1800)]
        );
        assert_eq!(total_secs(&sessions, 1800, 86_400), 4200);
        assert_eq!(minutes_per_bucket(&sessions, 0, 3600, 3), vec![60, 40, 0]);
    }

    #[test]
    fn understands_activity_questions() {
        let q = |text: &str| detect(&tokenize(text));
        for text in ["résumé de ma journée", "mon temps d'écran", "montre mon activité", "screen time", "summary of my day"] {
            assert_eq!(q(text), Some(ActivityQuery { app: None, period: Period::Today }), "{text}");
        }
        assert_eq!(q("temps d'écran cette semaine").map(|a| a.period), Some(Period::Week));
        assert_eq!(
            q("combien de temps j'ai passé sur Discord aujourd'hui"),
            Some(ActivityQuery { app: Some("discord".into()), period: Period::Today })
        );
        assert_eq!(
            q("how much time have I spent on VS Code this week"),
            Some(ActivityQuery { app: Some("vs code".into()), period: Period::Week })
        );
        for text in ["ouvre youtube", "fais un bilan du pc", "rappelle-moi dans combien de temps", "quel temps fait-il"] {
            assert_eq!(q(text), None, "{text}");
        }
    }

    #[test]
    fn finds_the_app_meant() {
        let usage = usage_by_app(
            &[
                Session { label: "Google Chrome".into(), ..session("chrome", 0, 60) },
                Session { label: "Visual Studio Code".into(), ..session("visualstudio", 60, 120) },
            ],
            0,
            200,
        );
        assert_eq!(find_app(&usage, "chrome").map(|u| u.app.as_str()), Some("chrome"));
        assert_eq!(find_app(&usage, "Visual Studio Code").map(|u| u.app.as_str()), Some("visualstudio"));
        assert_eq!(find_app(&usage, "code").map(|u| u.app.as_str()), Some("visualstudio"));
        assert_eq!(find_app(&usage, "spotify"), None);
    }

    #[test]
    fn words_durations_and_summaries() {
        assert_eq!(format_duration(30, Lang::Fr), "moins d'une minute");
        assert_eq!(format_duration(45 * 60, Lang::En), "45 min");
        assert_eq!(format_duration(2 * 3600 + 5 * 60, Lang::Fr), "2 h 05");
        assert_eq!(format_duration(2 * 3600 + 5 * 60, Lang::En), "2 h 5 min");
        let usage = usage_by_app(&[session("discord", 0, 7380), session("code", 7380, 9000)], 0, 86_400);
        assert_eq!(
            format_summary(&usage, Period::Today, Lang::Fr),
            "Aujourd'hui : 2 h 30 d'activité, surtout sur Discord (2 h 03) et VS Code (27 min)."
        );
        assert_eq!(format_summary(&[], Period::Week, Lang::En), "Over the last 7 days, I've barely seen any activity yet.");
        assert_eq!(
            format_app_time("discord", usage.first(), Period::Today, Lang::En),
            "Discord: 2 h 3 min today."
        );
    }
}
