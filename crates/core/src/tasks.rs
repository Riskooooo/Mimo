//! The to-do list: understanding "ajoute une tâche : appeler le plombier
//! demain à 14 h", "j'ai fini d'appeler le plombier", "qu'est-ce que j'ai à
//! faire aujourd'hui", and keeping the list. Dates are resolved against a
//! "now" the caller passes in, so everything here is testable.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

use crate::info::Lang;
use crate::intent::{tokenize, tokenize_spans, Token};
use crate::reminders::{number, time_of_day};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub due_date: Option<NaiveDate>,
    pub due_time: Option<NaiveTime>,
}

impl Task {
    /// Past its due moment: the time if there is one, otherwise the end of
    /// the due day.
    pub fn is_overdue(&self, now: NaiveDateTime) -> bool {
        match (self.due_date, self.due_time) {
            (Some(date), Some(time)) => date.and_time(time) < now,
            (Some(date), None) => date < now.date(),
            _ => false,
        }
    }

    pub fn is_due_today(&self, now: NaiveDateTime) -> bool {
        self.due_date == Some(now.date())
    }
}

/// A day as said, before resolving against today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayRef {
    Today,
    Tomorrow,
    AfterTomorrow,
    Weekday(Weekday),
    /// "le 15", "le 15 octobre"
    Date { day: u32, month: Option<u32> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Due {
    pub day: Option<DayRef>,
    pub time: Option<(u32, u32)>,
}

impl Due {
    pub fn is_empty(&self) -> bool {
        self.day.is_none() && self.time.is_none()
    }

    /// Concrete date/time. A time alone means today, or tomorrow if that
    /// time has already passed.
    pub fn resolve(&self, now: NaiveDateTime) -> (Option<NaiveDate>, Option<NaiveTime>) {
        let today = now.date();
        let time = self.time.and_then(|(h, m)| NaiveTime::from_hms_opt(h, m, 0));
        let date = match self.day {
            Some(DayRef::Today) => Some(today),
            Some(DayRef::Tomorrow) => Some(today + Duration::days(1)),
            Some(DayRef::AfterTomorrow) => Some(today + Duration::days(2)),
            Some(DayRef::Weekday(weekday)) => {
                // The next such day, never today ("vendredi" said on a Friday = next week).
                let ahead = (7 + weekday.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64) % 7;
                Some(today + Duration::days(if ahead == 0 { 7 } else { ahead }))
            }
            Some(DayRef::Date { day, month }) => {
                let month = month.unwrap_or(today.month());
                let this_year = NaiveDate::from_ymd_opt(today.year(), month, day);
                match this_year {
                    Some(d) if d >= today => Some(d),
                    // Already past: next month (day only) or next year.
                    _ if self.day.is_some() && matches!(self.day, Some(DayRef::Date { month: None, .. })) => {
                        let (y, m) = if today.month() == 12 { (today.year() + 1, 1) } else { (today.year(), today.month() + 1) };
                        NaiveDate::from_ymd_opt(y, m, day)
                    }
                    _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day),
                }
            }
            None => time.map(|t| if t > now.time() { today } else { today + Duration::days(1) }),
        };
        (date, time)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskCommand {
    Add { title: String, due: Due },
    Complete { query: String },
    Delete { query: String },
    List { today_only: bool },
}

const TASK_WORDS: &[&str] = &["tache", "taches", "task", "tasks", "todo", "to-do"];
const LIST_WORDS: &[&str] = &["liste", "list"];
const ADD_VERBS: &[&str] = &["ajoute", "ajouter", "rajoute", "note", "noter", "add", "nouvelle", "new", "cree", "creer", "create"];
const DELETE_VERBS: &[&str] = &["supprime", "supprimer", "enleve", "enlever", "retire", "retirer", "efface", "effacer", "delete", "remove"];
const DONE_WORDS: &[&str] = &[
    "fini", "finie", "fait", "faite", "termine", "terminee", "coche", "cocher", "done", "finished",
    "completed", "complete", "did",
];
const SHOW_WORDS: &[&str] = &["affiche", "afficher", "montre", "montrer", "mes", "quelles", "liste", "show", "my", "list", "what", "see", "voir"];
const GREETINGS: &[&str] = &["hey", "he", "eh", "et", "ok", "okay", "salut", "dis", "mimo", "memo"];

/// Words trimmed from the edges of a task title / query.
const EDGE_FILLERS: &[&str] = &[
    "une", "un", "la", "le", "les", "ma", "mes", "a", "dans", "de", "d", "du", "que", "qu", "je", "dois",
    "to", "my", "the", "an", "on", "in", "as", "comme", "est", "is", "liste", "list", "tache", "task",
    "pour", "for", "moi", "me", "please", "stp", "svp", "il", "faut", "que", "i", "need", "have", ":",
];

/// Recognizes a to-do request. `request` is the raw text (titles keep their
/// original spelling); greetings in front are ignored.
pub fn detect(request: &str) -> Option<TaskCommand> {
    let mut spans = tokenize_spans(request);
    while spans.first().is_some_and(|t| GREETINGS.contains(&t.folded.as_str())) {
        spans.remove(0);
    }
    let pairs = Words { input: request, tokens: spans };
    let tokens: Vec<&str> = pairs.tokens.iter().map(|t| t.folded.as_str()).collect();
    let has = |w: &str| tokens.contains(&w);
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(t));
    let about_tasks = has_any(TASK_WORDS) || (has_any(LIST_WORDS) && (has("ma") || has("my")));

    // "qu'est-ce que j'ai à faire (aujourd'hui)", "what do I have to do (today)"
    let has_seq = |seq: &[&str]| tokens.windows(seq.len()).any(|w| w == seq);
    let what_to_do = has_seq(&["a", "faire"]) && (has("qu") || has("quoi") || has("j"))
        || has_seq(&["to", "do"]) && (has("what") || has("have"));
    let today_only = has("aujourd") || has("today") || has("ajd");
    if what_to_do && !has_any(ADD_VERBS) {
        return Some(TaskCommand::List { today_only });
    }

    if about_tasks && has_any(DELETE_VERBS) {
        let query = remainder(&pairs, &[DELETE_VERBS, TASK_WORDS], None);
        return query.map(|query| TaskCommand::Delete { query });
    }

    if has_any(ADD_VERBS) && (about_tasks || has_any(LIST_WORDS)) {
        let due = find_due(&pairs);
        let title = remainder(&pairs, &[ADD_VERBS, TASK_WORDS, LIST_WORDS], Some(&due.1))?;
        return Some(TaskCommand::Add { title: capitalize(&title), due: due.0 });
    }

    // "j'ai fini X", "j'ai fait X", "X est fait", "marque X comme fait",
    // "I finished X", "mark X as done", "X is done"
    let finished = (has("j") && has("ai") && has_any(DONE_WORDS))
        || (has("i") && has_any(&["finished", "did", "completed", "done"]))
        || has_any(&["marque", "marquer", "mark", "check", "coche", "cocher"]) && has_any(DONE_WORDS)
        || (about_tasks && has_any(DONE_WORDS))
        || has_seq(&["is", "done"])
        || has_seq(&["est", "fait"])
        || has_seq(&["est", "faite"])
        || has_seq(&["est", "fini"])
        || has_seq(&["est", "finie"]);
    if finished {
        let query = remainder(
            &pairs,
            &[DONE_WORDS, TASK_WORDS, &["j", "ai", "i", "marque", "marquer", "mark", "check", "off", "coche", "cocher"]],
            None,
        )?;
        return Some(TaskCommand::Complete { query });
    }

    if about_tasks && has_any(SHOW_WORDS) {
        return Some(TaskCommand::List { today_only });
    }
    None
}

/// Parses what's typed in the panel's "new task" field: a title with an
/// optional day/time, no command words needed.
pub fn parse_new_task(text: &str) -> Option<(String, Due)> {
    let pairs = Words { input: text, tokens: tokenize_spans(text) };
    let (due, span) = find_due(&pairs);
    let title = remainder(&pairs, &[], Some(&span))?;
    Some((capitalize(&title), due))
}

/// The tokens of a request together with its original text.
pub(crate) struct Words<'a> {
    pub input: &'a str,
    pub tokens: Vec<Token>,
}

impl<'a> Words<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input, tokens: tokenize_spans(input) }
    }
}

/// The words left once `drop` lists and the due-date tokens are removed,
/// trimmed of fillers at both ends, in their original spelling: words that
/// were next to each other keep what separated them ("l'examen").
pub(crate) fn remainder(words: &Words, drop: &[&[&str]], due_tokens: Option<&[usize]>) -> Option<String> {
    let kept: Vec<usize> = (0..words.tokens.len())
        .filter(|i| {
            let t = words.tokens[*i].folded.as_str();
            !drop.iter().any(|list| list.contains(&t)) && !due_tokens.is_some_and(|d| d.contains(i))
        })
        .collect();
    let meaningful = |i: &&usize| !EDGE_FILLERS.contains(&words.tokens[**i].folded.as_str());
    let first = kept.iter().position(|i| meaningful(&i))?;
    let last = kept.iter().rposition(|i| meaningful(&i))?;

    let mut out = String::new();
    let mut previous: Option<usize> = None;
    for &i in &kept[first..=last] {
        let token = &words.tokens[i];
        if let Some(p) = previous {
            let gap = &words.input[words.tokens[p].end..token.start];
            let tight = p + 1 == i && gap.chars().count() <= 2 && !gap.chars().any(char::is_alphanumeric) && gap.trim() != ":";
            out.push_str(if tight && !gap.is_empty() { gap } else { " " });
        }
        out.push_str(&words.input[token.start..token.end]);
        previous = Some(i);
    }
    Some(out)
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

const WEEKDAYS: &[(&str, Weekday)] = &[
    ("lundi", Weekday::Mon), ("mardi", Weekday::Tue), ("mercredi", Weekday::Wed), ("jeudi", Weekday::Thu),
    ("vendredi", Weekday::Fri), ("samedi", Weekday::Sat), ("dimanche", Weekday::Sun),
    ("monday", Weekday::Mon), ("tuesday", Weekday::Tue), ("wednesday", Weekday::Wed), ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri), ("saturday", Weekday::Sat), ("sunday", Weekday::Sun),
];

const MONTHS: &[(&str, u32)] = &[
    ("janvier", 1), ("fevrier", 2), ("mars", 3), ("avril", 4), ("mai", 5), ("juin", 6), ("juillet", 7),
    ("aout", 8), ("septembre", 9), ("octobre", 10), ("novembre", 11), ("decembre", 12),
    ("january", 1), ("february", 2), ("march", 3), ("april", 4), ("may", 5), ("june", 6), ("july", 7),
    ("august", 8), ("september", 9), ("october", 10), ("november", 11), ("december", 12),
];

/// French month names as written (MONTHS holds them accent-folded for matching).
const FR_MONTH_NAMES: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre",
    "novembre", "décembre",
];

/// Connectors that belong to the date phrase when right before it.
const DUE_LEAD: &[&str] = &["pour", "a", "at", "on", "by", "le", "ce", "cette", "this", "next", "prochain", "avant", "before", "d", "ici", "vers", "around", "for"];

/// Finds the day and time in a request, and which tokens they span.
pub(crate) fn find_due(words: &Words) -> (Due, Vec<usize>) {
    let tokens: Vec<String> = words.tokens.iter().map(|t| t.folded.clone()).collect();
    let word = |i: usize| tokens.get(i).map(String::as_str);
    let mut due = Due::default();
    let mut used: Vec<usize> = Vec::new();
    let mark = |range: std::ops::Range<usize>, used: &mut Vec<usize>| {
        // Pull in connectors just before the phrase ("pour demain", "à 14h").
        let mut start = range.start;
        while start > 0 && DUE_LEAD.contains(&tokens[start - 1].as_str()) && !used.contains(&(start - 1)) {
            start -= 1;
        }
        used.extend(start..range.end);
    };

    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        let day = match (t, word(i + 1)) {
            ("aujourd", Some("hui")) => Some((DayRef::Today, 2)),
            ("today" | "ajd", _) => Some((DayRef::Today, 1)),
            ("apres", Some("demain")) => Some((DayRef::AfterTomorrow, 2)),
            ("demain" | "tomorrow", _) => Some((DayRef::Tomorrow, 1)),
            _ => WEEKDAYS.iter().find(|(name, _)| *name == t).map(|(_, wd)| (DayRef::Weekday(*wd), 1)),
        };
        if let Some((day_ref, len)) = day.filter(|_| due.day.is_none()) {
            due.day = Some(day_ref);
            mark(i..i + len, &mut used);
            i += len;
            continue;
        }
        // "ce soir", "tonight", "ce matin", "this morning"
        let part_of_day = match (t, word(i + 1)) {
            ("ce", Some("soir")) | ("this", Some("evening")) => Some(((20, 0), 2)),
            ("tonight", _) => Some(((20, 0), 1)),
            ("ce", Some("matin")) | ("this", Some("morning")) => Some(((9, 0), 2)),
            ("cet", Some("aprem")) | ("cet", Some("apres")) | ("this", Some("afternoon")) => Some(((15, 0), 2)),
            _ => None,
        };
        if let Some((time, len)) = part_of_day.filter(|_| due.time.is_none()) {
            due.time = Some(time);
            due.day.get_or_insert(DayRef::Today);
            mark(i..i + len, &mut used);
            i += len;
            continue;
        }
        // "le 15 (octobre)"
        if t == "le" || t == "the" {
            if let Some((day, after)) = number(&tokens, i + 1).filter(|(d, _)| (1..=31).contains(d)) {
                // Not a time: "le 15 h" isn't a date.
                if !matches!(word(after), Some("h" | "heure" | "heures")) && due.day.is_none() {
                    let month = word(after).and_then(|m| MONTHS.iter().find(|(name, _)| *name == m)).map(|(_, n)| *n);
                    due.day = Some(DayRef::Date { day, month });
                    let end = if month.is_some() { after + 1 } else { after };
                    mark(i..end, &mut used);
                    i = end;
                    continue;
                }
            }
        }
        // Times: "à 14h", "at 5 pm", "18h30"
        let time_start = matches!(t, "a" | "at" | "vers" | "around").then_some(i + 1).or_else(|| {
            (t.contains('h') && t.split_once('h').is_some_and(|(h, _)| !h.is_empty() && h.chars().all(|c| c.is_ascii_digit())))
                .then_some(i)
        });
        if let Some(start) = time_start.filter(|_| due.time.is_none()) {
            if let Some((h, m, end)) = time_of_day(&tokens, start) {
                // A bare number after "à" is only a time with an hour marker
                // or am/pm ("à 3 personnes" isn't 3:00).
                let explicit = end - start > 1 || tokens[start].contains('h') || matches!(tokens[start].as_str(), "midi" | "minuit" | "noon" | "midnight");
                if explicit {
                    due.time = Some((h, m));
                    mark(i..end, &mut used);
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    (due, used)
}

/// The list itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStore {
    pub tasks: Vec<Task>,
    next_id: u64,
}

impl TaskStore {
    pub fn add(&mut self, title: String, due: Due, now: NaiveDateTime) -> Task {
        self.next_id += 1;
        let (due_date, due_time) = due.resolve(now);
        let task = Task { id: self.next_id, title, due_date, due_time };
        self.tasks.push(task.clone());
        self.sort();
        task
    }

    /// Overdue and dated first (soonest first), undated last.
    fn sort(&mut self) {
        self.tasks.sort_by_key(|t| (t.due_date.is_none(), t.due_date, t.due_time.is_none(), t.due_time, t.id));
    }

    /// Replaces a task's title and due date/time (edited in the panel).
    pub fn update(&mut self, id: u64, title: String, due_date: Option<NaiveDate>, due_time: Option<NaiveTime>) -> Option<Task> {
        let task = self.tasks.iter_mut().find(|t| t.id == id)?;
        task.title = title;
        task.due_date = due_date;
        // A time alone has no day to belong to.
        task.due_time = due_date.and(due_time);
        let task = task.clone();
        self.sort();
        Some(task)
    }

    pub fn remove(&mut self, id: u64) -> Option<Task> {
        let index = self.tasks.iter().position(|t| t.id == id)?;
        Some(self.tasks.remove(index))
    }

    /// The task whose title best matches `query` (most shared words), if
    /// at least half of the query's meaningful words are in it.
    pub fn find(&self, query: &str) -> Option<&Task> {
        let words: Vec<String> = tokenize(query).into_iter().filter(|w| w.len() > 2 || w.chars().all(|c| c.is_ascii_digit())).collect();
        if words.is_empty() {
            return None;
        }
        self.tasks
            .iter()
            .map(|task| {
                let title = tokenize(&task.title);
                let shared = words.iter().filter(|w| title.iter().any(|t| t == *w || (t.len() > 3 && w.len() > 3 && (t.starts_with(w.as_str()) || w.starts_with(t.as_str()))))).count();
                (task, shared)
            })
            .filter(|(_, shared)| *shared * 2 >= words.len() && *shared > 0)
            .max_by_key(|(task, shared)| (*shared, std::cmp::Reverse(task.id)))
            .map(|(task, _)| task)
    }

    /// (to do today — overdue ones included —, overdue)
    pub fn counts(&self, now: NaiveDateTime) -> (usize, usize) {
        let today = self.tasks.iter().filter(|t| t.is_due_today(now) || t.is_overdue(now)).count();
        let overdue = self.tasks.iter().filter(|t| t.is_overdue(now)).count();
        (today, overdue)
    }
}

/// "demain 14:00", "vendredi", "tomorrow 2:00 PM".
pub fn format_due(task: &Task, now: NaiveDateTime, lang: Lang) -> Option<String> {
    let date = task.due_date?;
    let today = now.date();
    let day = if date == today {
        match lang { Lang::Fr => "aujourd'hui".to_string(), Lang::En => "today".to_string() }
    } else if date == today + Duration::days(1) {
        match lang { Lang::Fr => "demain".to_string(), Lang::En => "tomorrow".to_string() }
    } else if date > today && date < today + Duration::days(7) {
        let i = date.weekday().num_days_from_monday() as usize;
        match lang { Lang::Fr => WEEKDAYS[i].0.to_string(), Lang::En => capitalize(WEEKDAYS[7 + i].0) }
    } else {
        let month = date.month0() as usize;
        match lang {
            Lang::Fr => format!("{} {}", date.day(), FR_MONTH_NAMES[month]),
            Lang::En => format!("{} {}", capitalize(MONTHS[12 + month].0), date.day()),
        }
    };
    Some(match task.due_time {
        Some(time) => match lang {
            Lang::Fr => format!("{day} à {}", time.format("%H:%M")),
            Lang::En => format!("{day} at {}", time.format("%-I:%M %p")),
        },
        None => day,
    })
}

pub fn format_added(task: &Task, now: NaiveDateTime, lang: Lang) -> String {
    match (format_due(task, now, lang), lang) {
        (Some(due), Lang::Fr) => format!("Tâche ajoutée : {} ({due})", task.title),
        (None, Lang::Fr) => format!("Tâche ajoutée : {}", task.title),
        (Some(due), Lang::En) => format!("Task added: {} ({due})", task.title),
        (None, Lang::En) => format!("Task added: {}", task.title),
    }
}

pub fn format_completed(task: &Task, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Bien joué, « {} » est faite.", task.title),
        Lang::En => format!("Nice, “{}” is done.", task.title),
    }
}

pub fn format_deleted(task: &Task, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Tâche supprimée : {}", task.title),
        Lang::En => format!("Task deleted: {}", task.title),
    }
}

pub fn format_not_found(query: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("Je ne trouve pas de tâche « {query} »."),
        Lang::En => format!("I can't find a task “{query}”."),
    }
}

/// One line for the pill when the list is shown.
pub fn format_summary(store: &TaskStore, now: NaiveDateTime, today_only: bool, lang: Lang) -> String {
    let (today, overdue) = store.counts(now);
    let total = store.tasks.len();
    let late = |n: usize| match (lang, n) {
        (_, 0) => String::new(),
        (Lang::Fr, n) => format!(", dont {n} en retard"),
        (Lang::En, n) => format!(", {n} overdue"),
    };
    match (lang, today_only) {
        (Lang::Fr, true) if today == 0 => "Rien à faire aujourd'hui.".to_string(),
        (Lang::En, true) if today == 0 => "Nothing to do today.".to_string(),
        (Lang::Fr, true) => format!("Aujourd'hui : {today} tâche{}{}.", if today > 1 { "s" } else { "" }, late(overdue)),
        (Lang::En, true) => format!("Today: {today} task{}{}.", if today > 1 { "s" } else { "" }, late(overdue)),
        (Lang::Fr, false) if total == 0 => "Aucune tâche.".to_string(),
        (Lang::En, false) if total == 0 => "No tasks.".to_string(),
        (Lang::Fr, false) => format!("{total} tâche{}{}.", if total > 1 { "s" } else { "" }, late(overdue)),
        (Lang::En, false) => format!("{total} task{}{}.", if total > 1 { "s" } else { "" }, late(overdue)),
    }
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "add a task [unk]", "add [unk] to my list", "new task [unk]", "what do i have to do today",
            "what do i have to do", "show my tasks", "my tasks", "show my to do list", "i finished [unk]",
            "i did [unk]", "mark [unk] as done", "delete the task [unk]", "remove the task [unk]",
        ],
        _ => &[
            "ajoute une tâche [unk]", "ajoute [unk] à ma liste", "nouvelle tâche [unk]",
            "qu'est-ce que j'ai à faire aujourd'hui", "qu'est-ce que j'ai à faire", "mes tâches",
            "affiche les tâches", "affiche mes tâches", "montre mes tâches", "j'ai fini [unk]",
            "j'ai fait [unk]", "supprime la tâche [unk]", "enlève la tâche [unk]",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Friday 2 October 2026, 10:00.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap().and_hms_opt(10, 0, 0).unwrap()
    }

    fn add(request: &str) -> (String, Option<NaiveDate>, Option<NaiveTime>) {
        match detect(request) {
            Some(TaskCommand::Add { title, due }) => {
                let (d, t) = due.resolve(now());
                (title, d, t)
            }
            other => panic!("{request:?} → {other:?}"),
        }
    }

    fn date(d: u32, m: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, m, d)
    }

    fn time(h: u32, m: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(h, m, 0)
    }

    #[test]
    fn adding_tasks_with_dates() {
        assert_eq!(add("ajoute une tâche : appeler le plombier demain à 14h"), ("Appeler le plombier".into(), date(3, 10), time(14, 0)));
        assert_eq!(add("Ajoute Acheter du pain à ma liste pour vendredi"), ("Acheter du pain".into(), date(9, 10), None));
        assert_eq!(add("add a task buy milk tomorrow at 5 pm"), ("Buy milk".into(), date(3, 10), time(17, 0)));
        assert_eq!(add("nouvelle tâche réviser l'examen le 15 octobre"), ("Réviser l'examen".into(), date(15, 10), None));
        assert_eq!(add("ajoute une tâche sortir les poubelles ce soir"), ("Sortir les poubelles".into(), date(2, 10), time(20, 0)));
        assert_eq!(add("ajoute une tâche ranger le garage"), ("Ranger le garage".into(), None, None));
        assert_eq!(add("hey mimo add pay rent to my list on monday"), ("Pay rent".into(), date(5, 10), None));
    }

    #[test]
    fn a_time_alone_means_the_next_such_moment() {
        assert_eq!(add("ajoute une tâche appeler Léa à 18h").1, date(2, 10));
        assert_eq!(add("ajoute une tâche appeler Léa à 8h").1, date(3, 10));
    }

    #[test]
    fn completing_and_deleting() {
        assert_eq!(detect("j'ai fini d'appeler le plombier"), Some(TaskCommand::Complete { query: "appeler le plombier".into() }));
        assert_eq!(detect("I finished buy milk"), Some(TaskCommand::Complete { query: "buy milk".into() }));
        assert_eq!(detect("mark pay rent as done"), Some(TaskCommand::Complete { query: "pay rent".into() }));
        assert_eq!(detect("supprime la tâche ranger le garage"), Some(TaskCommand::Delete { query: "ranger le garage".into() }));
        assert_eq!(detect("delete the task buy milk"), Some(TaskCommand::Delete { query: "buy milk".into() }));
    }

    #[test]
    fn listing() {
        assert_eq!(detect("qu'est-ce que j'ai à faire aujourd'hui ?"), Some(TaskCommand::List { today_only: true }));
        assert_eq!(detect("affiche les tâches"), Some(TaskCommand::List { today_only: false }));
        assert_eq!(detect("what do I have to do today"), Some(TaskCommand::List { today_only: true }));
        assert_eq!(detect("show my tasks"), Some(TaskCommand::List { today_only: false }));
    }

    #[test]
    fn unrelated_requests_are_ignored() {
        for q in ["ouvre youtube", "rappelle-moi dans 10 minutes de sortir le linge", "quelle heure est-il", "lance spotify"] {
            assert_eq!(detect(q), None, "{q}");
        }
    }

    #[test]
    fn store_finds_overdue_and_matches_titles() {
        let mut store = TaskStore::default();
        let plumber = store.add("Appeler le plombier".into(), Due { day: Some(DayRef::Today), time: Some((9, 0)) }, now());
        store.add("Acheter du pain".into(), Due { day: Some(DayRef::Tomorrow), time: None }, now());
        store.add("Ranger le garage".into(), Due::default(), now());

        assert!(plumber.is_overdue(now()));
        assert_eq!(store.counts(now()), (1, 1));
        assert_eq!(store.find("appeler le plombier").map(|t| t.id), Some(plumber.id));
        assert_eq!(store.find("plombier").map(|t| t.id), Some(plumber.id));
        assert_eq!(store.find("faire la vaisselle"), None);
        assert_eq!(store.tasks.last().map(|t| t.title.as_str()), Some("Ranger le garage"));
        assert_eq!(format_summary(&store, now(), true, Lang::Fr), "Aujourd'hui : 1 tâche, dont 1 en retard.");
    }

    #[test]
    fn panel_input_needs_no_command_words() {
        let (title, due) = parse_new_task("acheter du pain demain 18h").unwrap();
        assert_eq!(title, "Acheter du pain");
        assert_eq!(due.resolve(now()), (date(3, 10), time(18, 0)));
    }

    #[test]
    fn due_wording() {
        let task = Task { id: 1, title: "x".into(), due_date: date(3, 10), due_time: time(14, 0) };
        assert_eq!(format_due(&task, now(), Lang::Fr).as_deref(), Some("demain à 14:00"));
        assert_eq!(format_due(&task, now(), Lang::En).as_deref(), Some("tomorrow at 2:00 PM"));
    }

    #[test]
    fn editing_a_task_keeps_the_list_sorted() {
        let mut store = TaskStore::default();
        let first = store.add("Courses".into(), Due::default(), now());
        store.add("Plombier".into(), Due { day: Some(DayRef::Tomorrow), time: None }, now());
        let date = NaiveDate::from_ymd_opt(2026, 10, 2);
        let edited = store.update(first.id, "Courses bio".into(), date, NaiveTime::from_hms_opt(9, 0, 0)).unwrap();
        assert_eq!(edited.title, "Courses bio");
        assert_eq!(store.tasks[0].id, first.id, "now dated today, so first");
        // A time without a day is dropped.
        let edited = store.update(first.id, "Courses".into(), None, NaiveTime::from_hms_opt(9, 0, 0)).unwrap();
        assert_eq!((edited.due_date, edited.due_time), (None, None));
        assert!(store.update(999, "x".into(), None, None).is_none());
    }
}
