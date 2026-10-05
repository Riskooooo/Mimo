//! Looking back at what the user did, from the activity sessions
//! ([`crate::activity`]): "qu'est-ce que je faisais hier vers 15h ?",
//! "what was I doing an hour ago", "qu'est-ce que j'ai fait ce matin", and
//! "rouvre ce que j'avais ouvert hier" (relaunches those apps — only apps:
//! Mimo keeps window titles, not documents or tabs). Times are passed in,
//! so everything here is testable.

use std::collections::HashSet;

use chrono::{Duration, NaiveDateTime, NaiveTime};

use crate::activity::{format_duration, usage_by_app, Session};
use crate::apps::AppCatalog;
use crate::info::Lang;
use crate::intent::tokenize;
use crate::reminders::{duration, time_of_day};
use crate::suggest::{join, LaunchTarget};

/// Around a precise moment ("vers 15h", "il y a une heure"), this much
/// either side is looked at.
const AROUND_MIN: i64 = 20;
/// "Tout à l'heure" / "earlier": the last two hours, minus the last few
/// minutes (what's on screen now isn't "earlier").
const EARLIER_HOURS: i64 = 2;
const EARLIER_SKIP_MIN: i64 = 5;
/// Apps used less than this in the period aren't reopened.
pub const REOPEN_MIN_SECS: i64 = 120;
/// At most this many apps are reopened at once.
pub const REOPEN_MAX: usize = 5;
/// Without a moment, "rouvre ce que j'avais ouvert" means the last stretch
/// of use before a break this long (the PC was off, asleep, or left).
const BREAK_MIN: i64 = 30;
/// …looking at most this far back before that break.
const LAST_STRETCH_HOURS: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayPart {
    Morning,
    Afternoon,
    Evening,
}

impl DayPart {
    fn hours(self) -> (u32, u32) {
        match self {
            DayPart::Morning => (5, 12),
            DayPart::Afternoon => (12, 18),
            DayPart::Evening => (18, 24),
        }
    }
}

/// When the user means. `days_ago`: 0 today, 1 yesterday, 2 the day before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    At { days_ago: u32, hour: u32, minute: u32 },
    Ago { secs: u64 },
    Part { days_ago: u32, part: DayPart },
    Day { days_ago: u32 },
    Earlier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallCommand {
    /// "Qu'est-ce que je faisais …"
    WhatWasI(Moment),
    /// "Rouvre ce que j'avais ouvert …"; `None`: the last stretch of use.
    Reopen(Option<Moment>),
}

const REOPEN_VERBS: &[&str] = &[
    "rouvre", "rouvrir", "reouvre", "reouvrir", "restaure", "restaurer", "relance", "relancer", "reopen",
    "restore", "relaunch",
];
/// What follows a reopen verb when it's about the past ("rouvre *ce que*
/// j'avais", "*mes* applis", "restore *my last session*") rather than one
/// app ("relance spotify").
const REOPEN_OBJECTS: &[&str] = &[
    "ce", "tout", "mes", "what", "everything", "my", "apps", "applis", "applications", "logiciels", "programmes",
    "session", "derniere", "last", "previous",
];

/// Recognizes a question about the past or a request to reopen it.
/// `tokens` as from `intent::tokenize`.
pub fn detect(tokens: &[String]) -> Option<RecallCommand> {
    let has = |w: &str| tokens.iter().any(|t| t == w);
    let has_seq = |seq: &[&str]| tokens.windows(seq.len()).any(|w| w == seq);
    let moment = find_moment(tokens);

    if tokens.iter().any(|t| REOPEN_VERBS.contains(&t.as_str()))
        && (moment.is_some() || tokens.iter().any(|t| REOPEN_OBJECTS.contains(&t.as_str())))
    {
        return Some(RecallCommand::Reopen(moment));
    }

    let what_fr = has_seq(&["je", "faisais"])
        || has_seq(&["je", "bossais"])
        || has_seq(&["je", "travaillais"])
        || (has_seq(&["j", "etais"]) && (has("quoi") || has("ou")))
        || (has_seq(&["j", "ai", "fait"]) && (has("qu") || has("quoi")))
        || has_seq(&["ai", "je", "fait"]);
    let what_en = has_seq(&["was", "i", "doing"])
        || has_seq(&["was", "i", "on"])
        || has_seq(&["was", "i", "working"])
        || has_seq(&["was", "i", "up", "to"])
        || has_seq(&["did", "i", "do"]);
    (what_fr || what_en).then(|| RecallCommand::WhatWasI(moment.unwrap_or(Moment::Earlier)))
}

fn find_moment(tokens: &[String]) -> Option<Moment> {
    let has = |w: &str| tokens.iter().any(|t| t == w);
    let has_seq = |seq: &[&str]| tokens.windows(seq.len()).any(|w| w == seq);

    // "il y a 2 heures", "il y a une demi-heure"
    if let Some(at) = tokens.windows(3).position(|w| w == ["il", "y", "a"]) {
        if let Some((secs, _)) = duration(tokens, at + 3) {
            return Some(Moment::Ago { secs });
        }
    }
    // "2 hours ago", "an hour ago" ("an"/"a" read as "one")
    if let Some(ago) = tokens.iter().position(|t| t == "ago") {
        let words: Vec<String> =
            tokens.iter().map(|t| if t == "an" || t == "a" { "one".to_string() } else { t.clone() }).collect();
        if let Some(secs) = (0..ago).find_map(|i| duration(&words, i).filter(|(_, end)| *end == ago).map(|(s, _)| s)) {
            return Some(Moment::Ago { secs });
        }
    }

    let days_ago = if has_seq(&["avant", "hier"]) || has_seq(&["day", "before", "yesterday"]) {
        Some(2)
    } else if has("hier") || has("yesterday") || has_seq(&["last", "night"]) {
        Some(1)
    } else if has("aujourd") || has("today") {
        Some(0)
    } else {
        None
    };
    let part = if has("matin") || has("matinee") || has("morning") {
        Some(DayPart::Morning)
    } else if has_seq(&["apres", "midi"]) || has("aprem") || has("afternoon") {
        Some(DayPart::Afternoon)
    } else if has("soir") || has("soiree") || has("evening") || has("tonight") || has("night") {
        Some(DayPart::Evening)
    } else {
        None
    };

    // "vers 15h", "à 10 h 30", "at 3 pm", "around noon" — not the "midi" of
    // "après-midi".
    let time = tokens.iter().enumerate().find_map(|(i, t)| {
        if !matches!(t.as_str(), "vers" | "a" | "at" | "around" | "about") || tokens.get(i + 1).map(String::as_str) == Some("l") {
            return None;
        }
        let (hour, minute, _) = time_of_day(tokens, i + 1)?;
        Some((hour, minute))
    });
    if let Some((mut hour, minute)) = time {
        // "ce soir vers 8h" is 20:00.
        if matches!(part, Some(DayPart::Afternoon | DayPart::Evening)) && hour < 12 {
            hour += 12;
        }
        return Some(Moment::At { days_ago: days_ago.unwrap_or(0), hour, minute });
    }
    if let Some(part) = part {
        return Some(Moment::Part { days_ago: days_ago.unwrap_or(0), part });
    }
    if let Some(days_ago) = days_ago {
        return Some(Moment::Day { days_ago });
    }
    if has_seq(&["tout", "a", "l", "heure"]) || has("earlier") || has_seq(&["just", "now"]) || has("avant") {
        return Some(Moment::Earlier);
    }
    None
}

/// A moment turned into actual times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,
    /// The precise moment asked about, if one was.
    pub point: Option<NaiveDateTime>,
    /// The moment, adjusted: "vers 23h" asked at 10:00 means yesterday.
    pub moment: Moment,
}

impl Moment {
    pub fn resolve(self, now: NaiveDateTime) -> Span {
        let day = |days_ago: u32| now.date() - Duration::days(i64::from(days_ago));
        let around = |point: NaiveDateTime, moment: Moment| Span {
            from: point - Duration::minutes(AROUND_MIN),
            to: (point + Duration::minutes(AROUND_MIN)).min(now),
            point: Some(point),
            moment,
        };
        match self {
            Moment::At { mut days_ago, hour, minute } => {
                let time = NaiveTime::from_hms_opt(hour.min(23), minute.min(59), 0).unwrap_or(NaiveTime::MIN);
                let mut point = day(days_ago).and_time(time);
                if point > now {
                    days_ago += 1;
                    point -= Duration::days(1);
                }
                around(point, Moment::At { days_ago, hour, minute })
            }
            Moment::Ago { secs } => around(now - Duration::seconds(secs as i64), self),
            Moment::Part { days_ago, part } => {
                let (start, end) = part.hours();
                let from = day(days_ago).and_time(NaiveTime::from_hms_opt(start, 0, 0).unwrap_or(NaiveTime::MIN));
                let to = from + Duration::hours(i64::from(end - start));
                Span { from, to: to.min(now), point: None, moment: self }
            }
            Moment::Day { days_ago } => {
                let from = day(days_ago).and_time(NaiveTime::MIN);
                Span { from, to: (from + Duration::days(1)).min(now), point: None, moment: self }
            }
            Moment::Earlier => Span {
                from: now - Duration::hours(EARLIER_HOURS),
                to: now - Duration::minutes(EARLIER_SKIP_MIN),
                point: None,
                moment: self,
            },
        }
    }

    /// "hier vers 15 h", "il y a 2 h", "ce matin" / "yesterday around 3 pm"…
    pub fn describe(self, lang: Lang) -> String {
        let day_fr = |days_ago: u32| match days_ago {
            0 => "",
            1 => "hier ",
            _ => "avant-hier ",
        };
        let day_en = |days_ago: u32| match days_ago {
            0 => "",
            1 => "yesterday ",
            _ => "the day before yesterday ",
        };
        match (self, lang) {
            (Moment::At { days_ago, hour, minute }, Lang::Fr) => {
                let minutes = if minute > 0 { format!(" {minute:02}") } else { String::new() };
                format!("{}vers {hour} h{minutes}", day_fr(days_ago))
            }
            (Moment::At { days_ago, hour, minute }, Lang::En) => {
                let (h12, meridiem) = match hour {
                    0 => (12, "am"),
                    1..=11 => (hour, "am"),
                    12 => (12, "pm"),
                    _ => (hour - 12, "pm"),
                };
                let minutes = if minute > 0 { format!(":{minute:02}") } else { String::new() };
                format!("{}around {h12}{minutes} {meridiem}", day_en(days_ago))
            }
            (Moment::Ago { secs }, Lang::Fr) => match secs / 3600 {
                h if h > 0 && secs % 3600 == 0 => format!("il y a {h} heure{}", if h > 1 { "s" } else { "" }),
                _ => format!("il y a {}", format_duration(secs as i64, lang)),
            },
            (Moment::Ago { secs }, Lang::En) => match secs / 3600 {
                1 if secs == 3600 => "an hour ago".to_string(),
                h if h > 0 && secs % 3600 == 0 => format!("{h} hours ago"),
                _ => format!("{} ago", format_duration(secs as i64, lang)),
            },
            (Moment::Part { days_ago, part }, Lang::Fr) => {
                let part = match part {
                    DayPart::Morning => "matin",
                    DayPart::Afternoon => "après-midi",
                    DayPart::Evening => "soir",
                };
                match (days_ago, part) {
                    (0, "après-midi") => "cet après-midi".to_string(),
                    (0, part) => format!("ce {part}"),
                    (1, part) => format!("hier {part}"),
                    (_, part) => format!("avant-hier {part}"),
                }
            }
            (Moment::Part { days_ago, part }, Lang::En) => {
                let part = match part {
                    DayPart::Morning => "morning",
                    DayPart::Afternoon => "afternoon",
                    DayPart::Evening => "evening",
                };
                match (days_ago, part) {
                    (0, part) => format!("this {part}"),
                    (1, "evening") => "last night".to_string(),
                    (1, part) => format!("yesterday {part}"),
                    (_, part) => format!("the day before yesterday in the {part}"),
                }
            }
            (Moment::Day { days_ago: 0 }, Lang::Fr) => "aujourd'hui".to_string(),
            (Moment::Day { days_ago }, Lang::Fr) => day_fr(days_ago).trim_end().to_string(),
            (Moment::Day { days_ago: 0 }, Lang::En) => "today".to_string(),
            (Moment::Day { days_ago }, Lang::En) => day_en(days_ago).trim_end().to_string(),
            (Moment::Earlier, Lang::Fr) => "tout à l'heure".to_string(),
            (Moment::Earlier, Lang::En) => "earlier".to_string(),
        }
    }
}

/// "la dernière fois" / "last time": what a reopen without a moment covers.
pub fn describe_last_time(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "la dernière fois",
        Lang::En => "last time",
    }
}

/// The last stretch of use before a break of [`BREAK_MIN`] minutes or more
/// (unix seconds), for "rouvre ce que j'avais ouvert": after a restart,
/// that's what was open before. Without such a break, the last few hours.
pub fn last_stretch(sessions: &[Session], now: i64) -> (i64, i64) {
    let mut ordered: Vec<&Session> = sessions.iter().filter(|s| s.start < now).collect();
    ordered.sort_by_key(|s| s.start);
    // Walking back from now, the first gap long enough is the break.
    let mut next_start = now;
    for s in ordered.iter().rev() {
        if next_start - s.end >= BREAK_MIN * 60 {
            return (s.end - LAST_STRETCH_HOURS * 3600, s.end);
        }
        next_start = next_start.min(s.start);
    }
    (now - LAST_STRETCH_HOURS * 3600, now)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Window titles can be long ("README.md — Mimo — Visual Studio Code…").
fn shorten(title: &str) -> String {
    const MAX: usize = 50;
    if title.chars().count() <= MAX {
        return title.to_string();
    }
    format!("{}…", title.chars().take(MAX - 1).collect::<String>().trim_end())
}

/// The answer to "what was I doing <when>", from the sessions around it
/// (unix seconds; `point`: the precise moment, if one was asked about).
pub fn answer(sessions: &[Session], from: i64, to: i64, point: Option<i64>, moment: Moment, lang: Lang) -> String {
    let when = moment.describe(lang);
    let usage: Vec<_> = usage_by_app(sessions, from, to).into_iter().filter(|u| u.secs >= 60).collect();
    if usage.is_empty() {
        return match lang {
            Lang::Fr => format!("Je ne t'ai pas vu sur le PC {when}."),
            Lang::En => format!("I didn't see you on the PC {when}."),
        };
    }

    if let Some(point) = point {
        // What was in front at that moment, or else what was used most around it.
        let at_point = sessions.iter().find(|s| s.start <= point && point < s.end);
        let main = at_point
            .and_then(|s| usage.iter().find(|u| u.app == s.app))
            .unwrap_or(&usage[0]);
        let title = at_point
            .filter(|s| s.app == main.app && !s.title.is_empty())
            .or_else(|| {
                sessions
                    .iter()
                    .filter(|s| s.app == main.app && !s.title.is_empty() && s.end > from && s.start < to)
                    .max_by_key(|s| s.end - s.start)
            })
            .map(|s| shorten(&s.title));
        let other = usage.iter().find(|u| u.app != main.app && u.secs >= 3 * 60);
        let mut reply = match (lang, &title) {
            (Lang::Fr, Some(title)) => format!("{}, tu étais sur {} (« {title} »)", capitalize(&when), main.label),
            (Lang::Fr, None) => format!("{}, tu étais sur {}", capitalize(&when), main.label),
            (Lang::En, Some(title)) => format!("{}, you were on {} (“{title}”)", capitalize(&when), main.label),
            (Lang::En, None) => format!("{}, you were on {}", capitalize(&when), main.label),
        };
        if let Some(other) = other {
            reply.push_str(&match lang {
                Lang::Fr => format!(", et un peu sur {}", other.label),
                Lang::En => format!(", and a bit on {}", other.label),
            });
        }
        reply.push('.');
        return reply;
    }

    let top: Vec<String> =
        usage.iter().take(3).map(|u| format!("{} ({})", u.label, format_duration(u.secs, lang))).collect();
    let top: Vec<&str> = top.iter().map(String::as_str).collect();
    match lang {
        Lang::Fr => format!("{} : surtout {}.", capitalize(&when), join(&top, "et")),
        Lang::En => format!("{}: mostly {}.", capitalize(&when), join(&top, "and")),
    }
}

/// What "rouvre ce que j'avais ouvert" will do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReopenPlan {
    /// Start menu apps to launch, most used first.
    pub launch: Vec<LaunchTarget>,
    /// Apps of that period that are still running (by label).
    pub already_open: Vec<String>,
}

/// Mimo itself (and its dev build) is never reopened.
const SELF_KEYS: &[&str] = &["mimo", "desktop"];

/// Apps used at least [`REOPEN_MIN_SECS`] within `from..to`, matched to the
/// Start menu (`apps`); `running` holds the keys of running processes
/// ("chrome"), whose apps are left alone.
pub fn plan_reopen(sessions: &[Session], from: i64, to: i64, running: &HashSet<String>, apps: &AppCatalog) -> ReopenPlan {
    let mut plan = ReopenPlan::default();
    for usage in usage_by_app(sessions, from, to) {
        if usage.secs < REOPEN_MIN_SECS || SELF_KEYS.contains(&usage.app.as_str()) {
            continue;
        }
        if running.contains(&usage.app) {
            plan.already_open.push(usage.label);
            continue;
        }
        let label = tokenize(&usage.label);
        let key = tokenize(&usage.app);
        let Some(installed) = apps
            .exact(&label)
            .or_else(|| apps.exact(&key))
            .or_else(|| apps.partial(&label))
            .or_else(|| apps.partial(&key))
        else {
            continue;
        };
        if plan.launch.len() < REOPEN_MAX && !plan.launch.iter().any(|t| t.app_id == installed.id) {
            plan.launch.push(LaunchTarget { name: installed.name.clone(), app_id: installed.id.clone() });
        }
    }
    plan
}

pub fn format_reopen(plan: &ReopenPlan, when: &str, lang: Lang) -> String {
    let launch: Vec<&str> = plan.launch.iter().map(|t| t.name.as_str()).collect();
    let open: Vec<&str> = plan.already_open.iter().map(String::as_str).collect();
    match (lang, launch.is_empty(), open.is_empty()) {
        (Lang::Fr, true, true) => format!("Je n'ai rien trouvé à rouvrir ({when})."),
        (Lang::En, true, true) => format!("I found nothing to reopen ({when})."),
        (Lang::Fr, true, false) => format!("Tout ce que tu utilisais {when} est déjà ouvert."),
        (Lang::En, true, false) => format!("Everything you were using {when} is already open."),
        (Lang::Fr, false, _) => {
            let mut reply = format!("Je rouvre {}.", join(&launch, "et"));
            match open.as_slice() {
                [] => {}
                [one] => reply.push_str(&format!(" {one} est déjà ouvert.")),
                _ => reply.push_str(&format!(" {} sont déjà ouverts.", join(&open, "et"))),
            }
            reply
        }
        (Lang::En, false, _) => {
            let mut reply = format!("Reopening {}.", join(&launch, "and"));
            match open.as_slice() {
                [] => {}
                [one] => reply.push_str(&format!(" {one} is already open.")),
                _ => reply.push_str(&format!(" {} are already open.", join(&open, "and"))),
            }
            reply
        }
    }
}

/// As the speech models write them; hours are enumerated in words.
pub fn voice_phrases(lang: &str) -> Vec<String> {
    let mut phrases: Vec<String> = Vec::new();
    match lang {
        "en" => {
            for when in [
                "yesterday", "earlier", "this morning", "this afternoon", "yesterday morning",
                "yesterday afternoon", "last night", "an hour ago", "two hours ago", "three hours ago",
                "ten minutes ago", "twenty minutes ago", "thirty minutes ago", "half an hour ago",
            ] {
                phrases.push(format!("what was i doing {when}"));
            }
            let hours = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
            for h in hours {
                for meridiem in ["am", "pm"] {
                    phrases.push(format!("what was i doing at {h} {meridiem}"));
                    phrases.push(format!("what was i doing yesterday at {h} {meridiem}"));
                }
            }
            phrases.extend(
                [
                    "what was i doing", "what did i do today", "what did i do yesterday", "what did i do this morning",
                    "reopen what i had open", "reopen what i had open yesterday", "reopen what i had open this morning",
                    "reopen my apps", "reopen my apps from yesterday", "restore my last session",
                ]
                .map(String::from),
            );
        }
        _ => {
            for when in [
                "hier", "tout à l'heure", "ce matin", "cet après-midi", "hier matin", "hier après-midi",
                "hier soir", "il y a une heure", "il y a deux heures", "il y a trois heures",
                "il y a dix minutes", "il y a vingt minutes", "il y a trente minutes", "il y a une demi-heure",
            ] {
                phrases.push(format!("qu'est-ce que je faisais {when}"));
            }
            let hours = [
                "huit", "neuf", "dix", "onze", "douze", "treize", "quatorze", "quinze", "seize", "dix-sept",
                "dix-huit", "dix-neuf", "vingt", "vingt et une", "vingt-deux", "vingt-trois",
            ];
            for h in hours {
                let unit = if h == "vingt et une" { "heure" } else { "heures" };
                phrases.push(format!("qu'est-ce que je faisais à {h} {unit}"));
                phrases.push(format!("qu'est-ce que je faisais hier à {h} {unit}"));
                phrases.push(format!("qu'est-ce que je faisais hier vers {h} {unit}"));
            }
            phrases.extend(
                [
                    "qu'est-ce que je faisais", "qu'est-ce que j'ai fait aujourd'hui", "qu'est-ce que j'ai fait hier",
                    "qu'est-ce que j'ai fait ce matin", "rouvre ce que j'avais ouvert",
                    "rouvre ce que j'avais ouvert hier", "rouvre ce que j'avais ouvert ce matin", "rouvre mes applis",
                    "rouvre mes applis d'hier", "relance ma dernière session",
                ]
                .map(String::from),
            );
        }
    }
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::app_label;
    use crate::apps::InstalledApp;

    fn detect_text(text: &str) -> Option<RecallCommand> {
        detect(&tokenize(text))
    }

    fn at(date: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M").unwrap()
    }

    fn session(app: &str, title: &str, start: i64, end: i64) -> Session {
        Session { app: app.into(), label: app_label(app, None), title: title.into(), start, end }
    }

    #[test]
    fn understands_what_was_i_doing() {
        use Moment::*;
        use RecallCommand::WhatWasI;
        let cases = [
            ("qu'est-ce que je faisais hier vers 15h", At { days_ago: 1, hour: 15, minute: 0 }),
            ("qu'est-ce que je faisais à 10 h 30", At { days_ago: 0, hour: 10, minute: 30 }),
            ("qu'est-ce que je faisais hier à quinze heures", At { days_ago: 1, hour: 15, minute: 0 }),
            ("je faisais quoi ce soir vers 8h", At { days_ago: 0, hour: 20, minute: 0 }),
            ("qu'est-ce que je faisais il y a une heure", Ago { secs: 3600 }),
            ("what was I doing 2 hours ago", Ago { secs: 7200 }),
            ("what was I doing an hour ago", Ago { secs: 3600 }),
            ("what was i doing yesterday at 3 pm", At { days_ago: 1, hour: 15, minute: 0 }),
            ("qu'est-ce que j'ai fait ce matin", Part { days_ago: 0, part: DayPart::Morning }),
            ("qu'est-ce que je faisais cet après-midi", Part { days_ago: 0, part: DayPart::Afternoon }),
            ("what did I do last night", Part { days_ago: 1, part: DayPart::Evening }),
            ("qu'est-ce que j'ai fait hier", Day { days_ago: 1 }),
            ("qu'est-ce que j'ai fait avant-hier", Day { days_ago: 2 }),
            ("what did I do today", Day { days_ago: 0 }),
            ("qu'est-ce que je faisais tout à l'heure", Earlier),
            ("sur quoi j'étais tout à l'heure", Earlier),
            ("qu'est-ce que je faisais", Earlier),
        ];
        for (text, moment) in cases {
            assert_eq!(detect_text(text), Some(WhatWasI(moment)), "{text}");
        }
    }

    #[test]
    fn understands_reopening() {
        use RecallCommand::Reopen;
        assert_eq!(detect_text("rouvre ce que j'avais ouvert"), Some(Reopen(None)));
        assert_eq!(detect_text("restore my last session"), Some(Reopen(None)));
        assert_eq!(
            detect_text("rouvre ce que j'avais ouvert ce matin"),
            Some(Reopen(Some(Moment::Part { days_ago: 0, part: DayPart::Morning })))
        );
        assert_eq!(detect_text("rouvre mes applis d'hier"), Some(Reopen(Some(Moment::Day { days_ago: 1 }))));
        assert_eq!(detect_text("reopen what I had open yesterday"), Some(Reopen(Some(Moment::Day { days_ago: 1 }))));
    }

    #[test]
    fn other_requests_are_left_alone() {
        for text in [
            "relance spotify", "relance la musique", "j'ai fait les courses", "qu'est-ce que j'ai à faire aujourd'hui",
            "ouvre youtube", "résumé de ma journée", "what do I have to do today", "rappelle-moi dans une heure",
        ] {
            assert_eq!(detect_text(text), None, "{text}");
        }
    }

    #[test]
    fn moments_become_times() {
        let now = at("2026-10-05 14:00");
        let span = Moment::At { days_ago: 1, hour: 15, minute: 0 }.resolve(now);
        assert_eq!((span.from, span.to, span.point), (at("2026-10-04 14:40"), at("2026-10-04 15:20"), Some(at("2026-10-04 15:00"))));
        // Later than now today: yesterday.
        let span = Moment::At { days_ago: 0, hour: 23, minute: 0 }.resolve(now);
        assert_eq!(span.moment, Moment::At { days_ago: 1, hour: 23, minute: 0 });
        assert_eq!(span.moment.describe(Lang::Fr), "hier vers 23 h");
        // Up to now at most.
        let span = Moment::Part { days_ago: 0, part: DayPart::Afternoon }.resolve(now);
        assert_eq!((span.from, span.to), (at("2026-10-05 12:00"), now));
        let span = Moment::Ago { secs: 3600 }.resolve(now);
        assert_eq!(span.point, Some(at("2026-10-05 13:00")));
        assert_eq!(Moment::Day { days_ago: 1 }.resolve(now).to, at("2026-10-05 00:00"));
    }

    #[test]
    fn describes_moments() {
        assert_eq!(Moment::At { days_ago: 0, hour: 10, minute: 30 }.describe(Lang::Fr), "vers 10 h 30");
        assert_eq!(Moment::At { days_ago: 1, hour: 15, minute: 0 }.describe(Lang::En), "yesterday around 3 pm");
        assert_eq!(Moment::Ago { secs: 7200 }.describe(Lang::Fr), "il y a 2 heures");
        assert_eq!(Moment::Ago { secs: 1800 }.describe(Lang::Fr), "il y a 30 min");
        assert_eq!(Moment::Ago { secs: 3600 }.describe(Lang::En), "an hour ago");
        assert_eq!(Moment::Part { days_ago: 0, part: DayPart::Afternoon }.describe(Lang::Fr), "cet après-midi");
        assert_eq!(Moment::Part { days_ago: 1, part: DayPart::Evening }.describe(Lang::En), "last night");
        assert_eq!(Moment::Day { days_ago: 2 }.describe(Lang::Fr), "avant-hier");
    }

    #[test]
    fn answers_about_a_moment() {
        let sessions = [
            session("code", "main.rs — Mimo", 0, 1800),
            session("discord", "général", 1800, 2400),
            session("chrome", "", 2400, 2460),
        ];
        let moment = Moment::Ago { secs: 3600 };
        assert_eq!(
            answer(&sessions, 0, 2400, Some(1000), moment, Lang::Fr),
            "Il y a 1 heure, tu étais sur VS Code (« main.rs — Mimo »), et un peu sur Discord."
        );
        assert_eq!(
            answer(&sessions, 0, 3000, None, Moment::Day { days_ago: 1 }, Lang::En),
            "Yesterday: mostly VS Code (30 min), Discord (10 min) and Chrome (1 min)."
        );
        assert_eq!(answer(&[], 0, 100, None, Moment::Earlier, Lang::Fr), "Je ne t'ai pas vu sur le PC tout à l'heure.");
    }

    #[test]
    fn finds_the_last_stretch_before_a_break() {
        let h = 3600;
        let sessions = [session("code", "", 0, 2 * h), session("discord", "", 2 * h, 3 * h), session("chrome", "", 12 * h, 12 * h + 300)];
        assert_eq!(last_stretch(&sessions, 12 * h + 600), (3 * h - 4 * h, 3 * h));
        // No break: the last few hours.
        assert_eq!(last_stretch(&sessions[..2], 3 * h), (-h, 3 * h));
    }

    #[test]
    fn plans_a_reopen() {
        let apps = AppCatalog::new(
            [("Visual Studio Code", "Code"), ("Discord", "Discord"), ("Spotify", "Spotify!App"), ("Google Chrome", "Chrome")]
                .into_iter()
                .filter_map(|(name, id)| InstalledApp::new(name, id))
                .collect(),
        );
        let sessions = [
            Session { label: "Visual Studio Code".into(), ..session("code", "", 0, 3000) },
            session("discord", "", 3000, 4000),
            session("spotify", "", 4000, 4060),
            Session { label: "Google Chrome".into(), ..session("chrome", "", 4060, 5000) },
            session("mimo", "", 5000, 6000),
        ];
        let running: HashSet<String> = ["chrome".to_string()].into();
        let plan = plan_reopen(&sessions, 0, 6000, &running, &apps);
        assert_eq!(plan.launch.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Visual Studio Code", "Discord"]);
        assert_eq!(plan.already_open, ["Google Chrome"]);
        assert_eq!(format_reopen(&plan, "hier", Lang::Fr), "Je rouvre Visual Studio Code et Discord. Google Chrome est déjà ouvert.");
        assert_eq!(
            format_reopen(&ReopenPlan { launch: vec![], already_open: vec!["X".into()] }, "yesterday", Lang::En),
            "Everything you were using yesterday is already open."
        );
    }
}
