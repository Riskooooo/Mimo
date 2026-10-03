//! Reminders and alarms: understanding "rappelle-moi dans 10 minutes de
//! sortir le linge" / "wake me up at 7:30", and keeping the list of pending
//! ones. Scheduling and ringing are the shell's job.

use serde::{Deserialize, Serialize};

use crate::info::Lang;
use crate::tasks::{capitalize, find_due, remainder, Due, Words};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderKind {
    /// A message at a time; rings once.
    Reminder,
    /// Rings until dismissed.
    Alarm,
}

/// When, as said: relative ("dans 10 minutes") or a time of day ("à 7 h"),
/// which the shell turns into the next such moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    In { seconds: u64 },
    At { hour: u32, minute: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderRequest {
    pub kind: ReminderKind,
    pub when: When,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderCommand {
    Create(ReminderRequest),
    List,
    /// Cancel all pending ones (of one kind, if said: "annule mes réveils").
    Cancel(Option<ReminderKind>),
}

/// When a reminder typed in the panel is due: relative, or a day and/or
/// time ("demain à 9 h", "vendredi") that the shell resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderTime {
    In { seconds: u64 },
    On(Due),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReminder {
    pub kind: ReminderKind,
    pub time: ReminderTime,
    pub message: Option<String>,
}

/// Parses what's typed in the panel's "new reminder" field: a message with
/// a moment ("appeler maman demain à 18h", "dans 10 min sortir le linge"),
/// no command words needed. `None` without a moment.
pub fn parse_new_reminder(text: &str) -> Option<NewReminder> {
    let words = Words::new(text);
    let folded: Vec<String> = words.tokens.iter().map(|t| t.folded.clone()).collect();
    let kind = if folded.iter().any(|t| ALARM_WORDS.contains(&t.as_str())) {
        ReminderKind::Alarm
    } else {
        ReminderKind::Reminder
    };
    let relative = (0..folded.len())
        .filter(|&i| matches!(folded[i].as_str(), "dans" | "in"))
        .find_map(|i| duration(&folded, i + 1).map(|(seconds, end)| (seconds, i..end)));
    let (time, span) = match relative {
        Some((seconds, range)) => (ReminderTime::In { seconds }, range.collect::<Vec<_>>()),
        None => {
            let (due, used) = find_due(&words);
            if due.is_empty() {
                return None;
            }
            (ReminderTime::On(due), used)
        }
    };
    let message = remainder(&words, &[REMIND_WORDS, ALARM_WORDS], Some(&span)).map(|m| capitalize(&m));
    Some(NewReminder { kind, time, message })
}

/// A scheduled reminder, as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: u64,
    pub kind: ReminderKind,
    /// Unix time (seconds) it's due.
    pub due: i64,
    pub message: Option<String>,
}

const REMIND_WORDS: &[&str] = &["rappelle", "rappeler", "rappel", "rappels", "remind", "reminder", "reminders"];
const ALARM_WORDS: &[&str] = &["reveille", "reveiller", "reveil", "reveils", "alarme", "alarmes", "alarm", "alarms", "wake"];
const LIST_WORDS: &[&str] = &["mes", "my", "liste", "list", "quels", "quelles", "show", "what"];
const CANCEL_WORDS: &[&str] = &["annule", "annuler", "supprime", "supprimer", "efface", "enleve", "cancel", "delete", "remove", "clear"];

/// Words between the trigger and the message: "rappelle-moi *de* …",
/// "remind me *to* …", "mets *un* réveil".
const FILLERS: &[&str] = &[
    "moi", "me", "nous", "de", "d", "que", "qu", "to", "that", "about", "pour", "un", "une", "an", "a",
    "set", "mets", "met", "programme", "cree", "creer", "fais", "up", "please", "stp", "svp", "il", "faut",
];

pub fn detect(tokens: &[String]) -> Option<ReminderCommand> {
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    let is_alarm = has_any(ALARM_WORDS);
    if !is_alarm && !has_any(REMIND_WORDS) {
        return None;
    }
    let kind = if is_alarm { ReminderKind::Alarm } else { ReminderKind::Reminder };

    if has_any(CANCEL_WORDS) {
        return Some(ReminderCommand::Cancel(Some(kind)));
    }

    match find_when(tokens) {
        Some((when, span)) => {
            let message = message(tokens, span);
            Some(ReminderCommand::Create(ReminderRequest { kind, when, message }))
        }
        // "mes rappels", "what are my alarms"
        None if has_any(LIST_WORDS) => Some(ReminderCommand::List),
        None => None,
    }
}

/// Finds "dans/in N unit" or "à/at H[:M]" and the token range it spans.
fn find_when(tokens: &[String]) -> Option<(When, std::ops::Range<usize>)> {
    for (i, token) in tokens.iter().enumerate() {
        match token.as_str() {
            "dans" | "in" => {
                if let Some((seconds, end)) = duration(tokens, i + 1) {
                    return Some((When::In { seconds }, i..end));
                }
            }
            "a" | "at" | "pour" | "for" | "vers" => {
                if let Some((hour, minute, end)) = time_of_day(tokens, i + 1) {
                    return Some((When::At { hour, minute }, i..end));
                }
            }
            _ => {}
        }
    }
    // A bare time ("réveil 7h30", "alarm 7 am").
    (0..tokens.len()).find_map(|i| {
        let (hour, minute, end) = time_of_day(tokens, i)?;
        // Plain numbers only count with an explicit marker (h, am, pm, :).
        let explicit = compact_hm(&tokens[i]).is_some()
            || end - i > 1
            || matches!(tokens[i].as_str(), "midi" | "minuit" | "noon" | "midnight");
        explicit.then_some((When::At { hour, minute }, i..end))
    })
}

/// "10 minutes", "dix minutes", "une heure", "1 h 30", "half an hour",
/// "une demi heure", "un quart d'heure".
fn duration(tokens: &[String], start: usize) -> Option<(u64, usize)> {
    let word = |i: usize| tokens.get(i).map(String::as_str);
    match (word(start), word(start + 1), word(start + 2)) {
        (Some("une" | "un"), Some("demi"), Some("heure")) => return Some((30 * 60, start + 3)),
        (Some("un"), Some("quart"), Some("d")) if word(start + 3) == Some("heure") => return Some((15 * 60, start + 4)),
        (Some("half"), Some("an"), Some("hour")) => return Some((30 * 60, start + 3)),
        _ => {}
    }

    let mut total = 0u64;
    let mut i = start;
    while let Some((n, after)) = number(tokens, i) {
        let unit = match word(after) {
            Some("seconde" | "secondes" | "second" | "seconds" | "sec" | "s") => 1,
            Some("minute" | "minutes" | "min" | "mn") => 60,
            Some("heure" | "heures" | "hour" | "hours" | "h") => 3600,
            _ => break,
        };
        total += u64::from(n) * unit;
        i = after + 1;
        if word(i) == Some("et") || word(i) == Some("and") {
            i += 1;
        }
    }
    // "1h30" as one token
    if total == 0 {
        let (h, m) = compact_hm(word(start)?)?;
        return Some((u64::from(h) * 3600 + u64::from(m) * 60, start + 1));
    }
    Some((total, i))
}

/// "7h", "7h30", "7 h 30", "7 heures trente", "sept heures et demie",
/// "19:45" (tokenized "19 45"), "7 pm", "7:30 am", "midi", "noon".
pub(crate) fn time_of_day(tokens: &[String], start: usize) -> Option<(u32, u32, usize)> {
    let word = |i: usize| tokens.get(i).map(String::as_str);
    match word(start)? {
        "midi" | "noon" => return Some((12, 0, start + 1)),
        "minuit" | "midnight" => return Some((0, 0, start + 1)),
        compact => {
            if let Some((h, m)) = compact_hm(compact) {
                return Some(with_meridiem(h, m, tokens, start + 1));
            }
        }
    }

    let (hour, mut i) = number(tokens, start)?;
    if hour > 23 {
        return None;
    }
    let mut minute = 0;
    if matches!(word(i), Some("h" | "heure" | "heures" | "o")) {
        i += 1;
        if word(i) == Some("clock") {
            i += 1;
        }
    }
    match (word(i), word(i + 1)) {
        (Some("et"), Some("demie" | "demi")) => {
            minute = 30;
            i += 2;
        }
        (Some("et"), Some("quart")) => {
            minute = 15;
            i += 2;
        }
        (Some("moins"), Some("le")) if word(i + 2) == Some("quart") => {
            return Some(((hour + 23) % 24, 45, i + 3));
        }
        _ => {
            if let Some((m, after)) = number(tokens, i) {
                if m < 60 {
                    minute = m;
                    i = after;
                }
            }
        }
    }
    Some(with_meridiem(hour, minute, tokens, i))
}

fn with_meridiem(hour: u32, minute: u32, tokens: &[String], i: usize) -> (u32, u32, usize) {
    match tokens.get(i).map(String::as_str) {
        Some("pm" | "p") if hour < 12 => (hour + 12, minute, i + 1),
        Some("am" | "a") if hour == 12 => (0, minute, i + 1),
        Some("am" | "pm") => (hour, minute, i + 1),
        // "7 heures du soir"
        Some("du") if tokens.get(i + 1).map(String::as_str) == Some("soir") && hour < 12 => (hour + 12, minute, i + 2),
        _ => (hour, minute, i),
    }
}

/// "7h", "7h30", "19h05".
fn compact_hm(token: &str) -> Option<(u32, u32)> {
    let (h, m) = token.split_once('h')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = if m.is_empty() { 0 } else { m.parse().ok()? };
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// A number written with digits or words ("25", "vingt cinq", "twenty
/// five"), and the index right after it.
pub(crate) fn number(tokens: &[String], start: usize) -> Option<(u32, usize)> {
    let first = tokens.get(start)?;
    if let Ok(n) = first.parse::<u32>() {
        return Some((n, start + 1));
    }
    let mut total = word_value(first)?;
    let mut i = start + 1;
    // Compose tens + units: "dix sept" (tokenized from "dix-sept"),
    // "vingt cinq", "vingt et un", "twenty five".
    if total >= 10 && total % 10 == 0 {
        let mut j = i;
        if tokens.get(j).map(String::as_str) == Some("et") {
            j += 1;
        }
        let min_unit = if total == 10 { 7 } else { 1 };
        if let Some(unit) = tokens.get(j).and_then(|t| word_value(t)).filter(|u| (min_unit..10).contains(u)) {
            total += unit;
            i = j + 1;
        }
    }
    Some((total, i))
}

fn word_value(word: &str) -> Option<u32> {
    Some(match word {
        "zero" => 0,
        "un" | "une" | "one" => 1,
        "deux" | "two" => 2,
        "trois" | "three" => 3,
        "quatre" | "four" => 4,
        "cinq" | "five" => 5,
        "six" => 6,
        "sept" | "seven" => 7,
        "huit" | "eight" => 8,
        "neuf" | "nine" => 9,
        "dix" | "ten" => 10,
        "onze" | "eleven" => 11,
        "douze" | "twelve" => 12,
        "treize" | "thirteen" => 13,
        "quatorze" | "fourteen" => 14,
        "quinze" | "fifteen" => 15,
        "seize" | "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "vingt" | "twenty" => 20,
        "trente" | "thirty" => 30,
        "quarante" | "forty" => 40,
        "cinquante" | "fifty" => 50,
        "soixante" | "sixty" => 60,
        _ => return None,
    })
}

/// Whatever remains once the trigger words, fillers and the time clause are
/// removed: "sortir le linge".
fn message(tokens: &[String], when: std::ops::Range<usize>) -> Option<String> {
    let words: Vec<&str> = tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| !when.contains(i))
        .map(|(_, t)| t.as_str())
        .filter(|t| !REMIND_WORDS.contains(t) && !ALARM_WORDS.contains(t))
        .collect();
    let trimmed: Vec<&str> = words
        .iter()
        .copied()
        .skip_while(|w| FILLERS.contains(w))
        .collect();
    (!trimmed.is_empty()).then(|| trimmed.join(" "))
}

/// The pending list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderStore {
    pub reminders: Vec<Reminder>,
    next_id: u64,
}

impl ReminderStore {
    pub fn add(&mut self, kind: ReminderKind, due: i64, message: Option<String>) -> &Reminder {
        self.next_id += 1;
        self.reminders.push(Reminder { id: self.next_id, kind, due, message });
        self.reminders.sort_by_key(|r| r.due);
        self.reminders.iter().find(|r| r.id == self.next_id).expect("just added")
    }

    /// Removes and returns everything due at or before `now`.
    pub fn take_due(&mut self, now: i64) -> Vec<Reminder> {
        let (due, pending) = self.reminders.drain(..).partition(|r| r.due <= now);
        self.reminders = pending;
        due
    }

    pub fn next_due(&self) -> Option<i64> {
        self.reminders.first().map(|r| r.due)
    }

    pub fn cancel(&mut self, kind: Option<ReminderKind>) -> usize {
        let before = self.reminders.len();
        self.reminders.retain(|r| kind.is_some_and(|k| r.kind != k));
        before - self.reminders.len()
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.reminders.len();
        self.reminders.retain(|r| r.id != id);
        before != self.reminders.len()
    }
}

/// "Rappel à 18 h 30 : appeler maman", "Alarm set for 7:00 AM".
pub fn format_created(lang: Lang, kind: ReminderKind, hour: u32, minute: u32, message: Option<&str>) -> String {
    let time = match lang {
        Lang::Fr => format!("{hour} h {minute:02}"),
        Lang::En => {
            let (h12, suffix) = match hour {
                0 => (12, "AM"),
                1..=11 => (hour, "AM"),
                12 => (12, "PM"),
                _ => (hour - 12, "PM"),
            };
            format!("{h12}:{minute:02} {suffix}")
        }
    };
    match (lang, kind, message) {
        (Lang::Fr, ReminderKind::Alarm, _) => format!("Réveil réglé à {time}."),
        (Lang::Fr, ReminderKind::Reminder, Some(m)) => format!("Rappel à {time} : {m}"),
        (Lang::Fr, ReminderKind::Reminder, None) => format!("Rappel réglé à {time}."),
        (Lang::En, ReminderKind::Alarm, _) => format!("Alarm set for {time}."),
        (Lang::En, ReminderKind::Reminder, Some(m)) => format!("Reminder at {time}: {m}"),
        (Lang::En, ReminderKind::Reminder, None) => format!("Reminder set for {time}."),
    }
}

/// "2 rappels en attente." / "No reminders."
pub fn format_pending(lang: Lang, count: usize) -> String {
    match (lang, count) {
        (Lang::Fr, 0) => "Aucun rappel ni réveil en attente.".to_string(),
        (Lang::Fr, 1) => "1 rappel en attente.".to_string(),
        (Lang::Fr, n) => format!("{n} rappels en attente."),
        (Lang::En, 0) => "No pending reminders or alarms.".to_string(),
        (Lang::En, 1) => "1 pending reminder.".to_string(),
        (Lang::En, n) => format!("{n} pending reminders."),
    }
}

pub fn format_cancelled(lang: Lang, kind: Option<ReminderKind>, count: usize) -> String {
    let alarm = kind == Some(ReminderKind::Alarm);
    match (lang, count, alarm) {
        (Lang::Fr, 0, true) => "Aucun réveil à annuler.".to_string(),
        (Lang::Fr, 0, false) => "Aucun rappel à annuler.".to_string(),
        (Lang::Fr, 1, true) => "Réveil annulé.".to_string(),
        (Lang::Fr, 1, false) => "Rappel annulé.".to_string(),
        (Lang::Fr, n, true) => format!("{n} réveils annulés."),
        (Lang::Fr, n, false) => format!("{n} rappels annulés."),
        (Lang::En, 0, true) => "No alarms to cancel.".to_string(),
        (Lang::En, 0, false) => "No reminders to cancel.".to_string(),
        (Lang::En, 1, true) => "Alarm cancelled.".to_string(),
        (Lang::En, 1, false) => "Reminder cancelled.".to_string(),
        (Lang::En, n, true) => format!("{n} alarms cancelled."),
        (Lang::En, n, false) => format!("{n} reminders cancelled."),
    }
}

/// Spoken phrasings for the voice grammar. Messages are free text, so
/// those phrasings end in "[unk]" (handing over to dictation); alarm times
/// are enumerated in words.
pub fn voice_phrases(lang: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    match lang {
        "en" => {
            let hours = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
            for h in hours {
                for tail in ["", " thirty", " fifteen", " forty five"] {
                    for meridiem in ["", " am", " pm"] {
                        for verb in ["wake me up at", "set an alarm for", "alarm at"] {
                            phrases.push(format!("{verb} {h}{tail}{meridiem}"));
                        }
                    }
                }
            }
            phrases.extend(
                ["remind me [unk]", "remind me to [unk]", "remind me in [unk]", "my reminders", "my alarms",
                 "what are my reminders", "cancel my reminders", "cancel my alarms", "cancel the alarm"]
                    .map(String::from),
            );
        }
        _ => {
            let hours = [
                "une", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix", "onze", "douze",
                "treize", "quatorze", "quinze", "seize", "dix-sept", "dix-huit", "dix-neuf", "vingt",
                "vingt et une", "vingt-deux", "vingt-trois",
            ];
            for h in hours {
                let unit = if h == "une" || h == "vingt et une" { "heure" } else { "heures" };
                for tail in ["", " trente", " quinze", " quarante-cinq", " et demie", " et quart"] {
                    for verb in ["réveille moi à", "mets un réveil à", "un réveil à", "alarme à"] {
                        phrases.push(format!("{verb} {h} {unit}{tail}"));
                    }
                }
            }
            for v in ["réveille moi à midi", "mets un réveil à midi", "réveille moi à minuit"] {
                phrases.push(v.to_string());
            }
            phrases.extend(
                ["rappelle moi [unk]", "rappelle moi de [unk]", "rappelle moi dans [unk]", "mes rappels",
                 "mes réveils", "quels sont mes rappels", "annule mes rappels", "annule mes réveils",
                 "annule le réveil", "supprime mes rappels"]
                    .map(String::from),
            );
        }
    }
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn ask(q: &str) -> Option<ReminderCommand> {
        detect(&tokenize(q))
    }

    fn create(kind: ReminderKind, when: When, message: Option<&str>) -> Option<ReminderCommand> {
        Some(ReminderCommand::Create(ReminderRequest { kind, when, message: message.map(String::from) }))
    }

    #[test]
    fn relative_reminders() {
        assert_eq!(
            ask("rappelle-moi dans 10 minutes de sortir le linge"),
            create(ReminderKind::Reminder, When::In { seconds: 600 }, Some("sortir le linge"))
        );
        assert_eq!(
            ask("rappelle moi de sortir le linge dans dix minutes"),
            create(ReminderKind::Reminder, When::In { seconds: 600 }, Some("sortir le linge"))
        );
        assert_eq!(
            ask("rappelle-moi dans une heure et 15 minutes d'appeler maman"),
            create(ReminderKind::Reminder, When::In { seconds: 4500 }, Some("appeler maman"))
        );
        assert_eq!(
            ask("remind me in half an hour to check the oven"),
            create(ReminderKind::Reminder, When::In { seconds: 1800 }, Some("check the oven"))
        );
        assert_eq!(
            ask("rappelle moi dans vingt cinq minutes"),
            create(ReminderKind::Reminder, When::In { seconds: 1500 }, None)
        );
    }

    #[test]
    fn reminders_at_a_time() {
        assert_eq!(
            ask("rappelle-moi à 18h30 d'appeler maman"),
            create(ReminderKind::Reminder, When::At { hour: 18, minute: 30 }, Some("appeler maman"))
        );
        assert_eq!(
            ask("remind me at 6 pm to call mom"),
            create(ReminderKind::Reminder, When::At { hour: 18, minute: 0 }, Some("call mom"))
        );
    }

    #[test]
    fn alarms() {
        assert_eq!(ask("réveille-moi à 7h"), create(ReminderKind::Alarm, When::At { hour: 7, minute: 0 }, None));
        assert_eq!(
            ask("mets un réveil à sept heures et demie"),
            create(ReminderKind::Alarm, When::At { hour: 7, minute: 30 }, None)
        );
        assert_eq!(
            ask("réveille moi à huit heures quarante-cinq"),
            create(ReminderKind::Alarm, When::At { hour: 8, minute: 45 }, None)
        );
        assert_eq!(ask("set an alarm for 7:30 am"), create(ReminderKind::Alarm, When::At { hour: 7, minute: 30 }, None));
        assert_eq!(ask("wake me up at seven thirty"), create(ReminderKind::Alarm, When::At { hour: 7, minute: 30 }, None));
        assert_eq!(ask("réveille-moi à midi"), create(ReminderKind::Alarm, When::At { hour: 12, minute: 0 }, None));
    }

    #[test]
    fn listing_and_cancelling() {
        assert_eq!(ask("mes rappels"), Some(ReminderCommand::List));
        assert_eq!(ask("what are my alarms"), Some(ReminderCommand::List));
        assert_eq!(ask("annule mes réveils"), Some(ReminderCommand::Cancel(Some(ReminderKind::Alarm))));
        assert_eq!(ask("cancel my reminders"), Some(ReminderCommand::Cancel(Some(ReminderKind::Reminder))));
    }

    #[test]
    fn unrelated_requests_are_ignored() {
        assert_eq!(ask("ouvre youtube"), None);
        assert_eq!(ask("quelle heure est-il"), None);
    }

    #[test]
    fn store_keeps_order_and_hands_out_due_ones() {
        let mut store = ReminderStore::default();
        store.add(ReminderKind::Alarm, 300, None);
        store.add(ReminderKind::Reminder, 100, Some("tea".into()));
        assert_eq!(store.next_due(), Some(100));
        let due = store.take_due(150);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message.as_deref(), Some("tea"));
        assert_eq!(store.next_due(), Some(300));
        assert_eq!(store.cancel(Some(ReminderKind::Reminder)), 0);
        assert_eq!(store.cancel(Some(ReminderKind::Alarm)), 1);
    }

    #[test]
    fn confirmations_are_worded_per_language() {
        assert_eq!(
            format_created(Lang::Fr, ReminderKind::Reminder, 18, 30, Some("appeler maman")),
            "Rappel à 18 h 30 : appeler maman"
        );
        assert_eq!(format_created(Lang::En, ReminderKind::Alarm, 7, 0, None), "Alarm set for 7:00 AM.");
    }

    #[test]
    fn reminders_typed_in_the_panel() {
        use crate::tasks::{DayRef, Due};
        let typed = parse_new_reminder("Appeler maman demain à 18h").unwrap();
        assert_eq!(typed.kind, ReminderKind::Reminder);
        assert_eq!(typed.time, ReminderTime::On(Due { day: Some(DayRef::Tomorrow), time: Some((18, 0)) }));
        assert_eq!(typed.message.as_deref(), Some("Appeler maman"));

        let typed = parse_new_reminder("dans 10 minutes sortir le linge").unwrap();
        assert_eq!(typed.time, ReminderTime::In { seconds: 600 });
        assert_eq!(typed.message.as_deref(), Some("Sortir le linge"));

        let typed = parse_new_reminder("réveil à 7h30").unwrap();
        assert_eq!(typed.kind, ReminderKind::Alarm);
        assert_eq!(typed.time, ReminderTime::On(Due { day: None, time: Some((7, 30)) }));
        assert_eq!(typed.message, None);

        assert_eq!(parse_new_reminder("appeler maman"), None);
    }
}
