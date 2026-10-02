//! Questions Mimo answers instead of opening something: the time, the date,
//! the weather. This module recognizes them and words the answers, in the
//! language the question was asked in; fetching the actual clock/weather is
//! the shell's job.

/// The app's language (setting): what Mimo writes and listens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Fr,
    #[default]
    En,
}

impl Lang {
    /// "fr" / "en", as stored in settings. Anything else is English, the
    /// default.
    pub fn from_code(code: &str) -> Self {
        if code == "fr" {
            Lang::Fr
        } else {
            Lang::En
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Today,
    Tomorrow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    Time,
    Date,
    /// `place` is what followed "à"/"in" ("météo à Lyon"), if anything;
    /// otherwise the shell uses the user's approximate location.
    Weather { day: Day, place: Option<String> },
}

const FR_MARKERS: &[&str] = &[
    "quelle", "quel", "heure", "est", "il", "meteo", "temps", "fait", "fera", "jour", "sommes",
    "demain", "aujourd", "hui", "donne", "moi", "la", "le", "on", "date", "mon", "mes", "lis",
    "fais", "rappelle", "reveille", "reveil", "rappels", "dans", "un", "une", "comment", "va", "du",
    "etat", "annule", "de", "j", "ai", "des",
];
const EN_MARKERS: &[&str] = &[
    "what", "time", "is", "it", "weather", "tomorrow", "today", "day", "the", "tell", "me", "s",
    "my", "run", "check", "read", "remind", "set", "an", "alarm", "for", "at", "in", "how", "wake",
    "up", "cancel", "reminders", "alarms", "any", "do", "i", "to", "are",
];

/// Recognizes a question in already-tokenized, accent-folded words.
pub fn detect(tokens: &[String]) -> Option<(Question, Lang)> {
    let has = |word: &str| tokens.iter().any(|t| t == word);
    let lang = detect_lang(tokens);

    let question = if has("meteo") || has("weather") || (has("temps") && (has("fait") || has("fera") || has("quel"))) {
        Question::Weather {
            day: if has("demain") || has("tomorrow") { Day::Tomorrow } else { Day::Today },
            place: place(tokens),
        }
    } else if has("heure") || (has("time") && (has("what") || has("tell") || tokens.len() <= 2)) {
        Question::Time
    } else if has("date")
        || (has("jour") && (has("quel") || has("sommes") || has("on")))
        || (has("day") && has("what"))
    {
        Question::Date
    } else {
        return None;
    };
    Some((question, lang))
}

pub(crate) fn detect_lang(tokens: &[String]) -> Lang {
    let fr = tokens.iter().filter(|t| FR_MARKERS.contains(&t.as_str())).count();
    let en = tokens.iter().filter(|t| EN_MARKERS.contains(&t.as_str())).count();
    if en > fr {
        Lang::En
    } else {
        Lang::Fr
    }
}

/// "météo à Lyon", "weather in New York" → the words after the preposition,
/// minus trailing day words.
fn place(tokens: &[String]) -> Option<String> {
    let at = tokens.iter().position(|t| matches!(t.as_str(), "a" | "in" | "pour" | "for" | "sur"))?;
    let words: Vec<&str> = tokens[at + 1..]
        .iter()
        .map(String::as_str)
        .take_while(|w| !matches!(*w, "demain" | "tomorrow" | "aujourd" | "today"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

const FR_WEEKDAYS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
const EN_WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const FR_MONTHS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre",
    "novembre", "décembre",
];
const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

/// `hour` 0–23.
pub fn format_time(lang: Lang, hour: u32, minute: u32) -> String {
    match lang {
        Lang::Fr => format!("Il est {hour} h {minute:02}."),
        Lang::En => {
            let (h12, suffix) = match hour {
                0 => (12, "AM"),
                1..=11 => (hour, "AM"),
                12 => (12, "PM"),
                _ => (hour - 12, "PM"),
            };
            format!("It's {h12}:{minute:02} {suffix}.")
        }
    }
}

/// `weekday` 0 = Monday, `month` 1–12.
pub fn format_date(lang: Lang, weekday: usize, day: u32, month: usize) -> String {
    let weekday = weekday.min(6);
    let month = month.clamp(1, 12) - 1;
    match lang {
        Lang::Fr => {
            let day = if day == 1 { "1er".to_string() } else { day.to_string() };
            format!("Nous sommes le {} {day} {}.", FR_WEEKDAYS[weekday], FR_MONTHS[month])
        }
        Lang::En => format!("It's {}, {} {day}.", EN_WEEKDAYS[weekday], EN_MONTHS[month]),
    }
}

/// What the shell fetched for a weather question.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherReport {
    pub place: String,
    pub day: Day,
    /// Current temperature (°C); only meaningful for today.
    pub now: Option<f64>,
    /// WMO weather code for the day (or now, for today).
    pub code: i32,
    pub min: f64,
    pub max: f64,
}

pub fn format_weather(lang: Lang, report: &WeatherReport) -> String {
    let sky = describe_weather(report.code, lang);
    let (min, max) = (report.min.round(), report.max.round());
    match (lang, report.day, report.now) {
        (Lang::Fr, Day::Today, Some(now)) => {
            format!("{} : {}°, {sky} ({min}° / {max}°)", report.place, now.round())
        }
        (Lang::Fr, _, _) => format!("Demain à {} : {sky}, {min}° / {max}°", report.place),
        (Lang::En, Day::Today, Some(now)) => {
            format!("{}: {}°, {sky} ({min}° / {max}°)", report.place, now.round())
        }
        (Lang::En, _, _) => format!("Tomorrow in {}: {sky}, {min}° / {max}°", report.place),
    }
}

/// WMO weather interpretation codes, as used by Open-Meteo.
pub fn describe_weather(code: i32, lang: Lang) -> &'static str {
    let (fr, en) = match code {
        0 => ("ciel dégagé", "clear sky"),
        1 => ("plutôt dégagé", "mostly clear"),
        2 => ("partiellement nuageux", "partly cloudy"),
        3 => ("couvert", "overcast"),
        45 | 48 => ("brouillard", "fog"),
        51 | 53 | 55 | 56 | 57 => ("bruine", "drizzle"),
        61 | 63 | 66 => ("pluie", "rain"),
        65 | 67 => ("forte pluie", "heavy rain"),
        71 | 73 | 77 => ("neige", "snow"),
        75 => ("forte neige", "heavy snow"),
        80 | 81 => ("averses", "showers"),
        82 => ("fortes averses", "heavy showers"),
        85 | 86 => ("averses de neige", "snow showers"),
        95 => ("orages", "thunderstorms"),
        96 | 99 => ("orages et grêle", "thunderstorms with hail"),
        _ => ("temps variable", "changeable"),
    };
    match lang {
        Lang::Fr => fr,
        Lang::En => en,
    }
}

/// Spoken phrasings for voice recognition's grammar (see
/// `intent::voice_phrases`). Variants are generous on purpose: the voice
/// module drops any that use words the speech model doesn't know.
pub fn voice_questions(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "what time is it", "what's the time", "tell me the time", "the time", "what is the time",
            "what's the weather", "what is the weather", "what's the weather today",
            "what's the weather tomorrow", "weather", "the weather", "weather tomorrow",
            "tell me the weather", "what day is it", "what's the date", "what is the date",
            "what's the date today",
        ],
        _ => &[
            "quelle heure est-il", "quelle heure est il", "il est quelle heure", "donne moi l'heure",
            "quelle heure", "l'heure", "quel temps fait-il", "quel temps fait il",
            "quel temps fait-il aujourd'hui", "quel temps fera-t-il demain", "quel temps il fait",
            "la météo", "météo", "donne moi la météo", "quelle est la météo", "la météo de demain",
            "météo demain", "la météo demain", "quel jour sommes-nous", "quel jour sommes nous",
            "on est quel jour", "quelle est la date", "quel jour on est",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn ask(q: &str) -> Option<(Question, Lang)> {
        detect(&tokenize(q))
    }

    #[test]
    fn recognizes_time_questions() {
        assert_eq!(ask("Quelle heure est-il ?"), Some((Question::Time, Lang::Fr)));
        assert_eq!(ask("donne moi l'heure"), Some((Question::Time, Lang::Fr)));
        assert_eq!(ask("what time is it"), Some((Question::Time, Lang::En)));
    }

    #[test]
    fn recognizes_weather_questions() {
        let today = Question::Weather { day: Day::Today, place: None };
        assert_eq!(ask("quel temps fait-il aujourd'hui ?"), Some((today.clone(), Lang::Fr)));
        assert_eq!(ask("donne moi la météo"), Some((today.clone(), Lang::Fr)));
        assert_eq!(ask("what's the weather"), Some((today, Lang::En)));
        assert_eq!(
            ask("quel temps fera-t-il demain à Lyon"),
            Some((Question::Weather { day: Day::Tomorrow, place: Some("lyon".into()) }, Lang::Fr))
        );
        assert_eq!(
            ask("weather in new york tomorrow"),
            Some((Question::Weather { day: Day::Tomorrow, place: Some("new york".into()) }, Lang::En))
        );
    }

    #[test]
    fn recognizes_date_questions() {
        assert_eq!(ask("quel jour sommes-nous"), Some((Question::Date, Lang::Fr)));
        assert_eq!(ask("what day is it"), Some((Question::Date, Lang::En)));
    }

    #[test]
    fn other_requests_are_not_questions() {
        assert_eq!(ask("ouvre youtube"), None);
        assert_eq!(ask("lance spotify"), None);
    }

    #[test]
    fn answers_are_worded_per_language() {
        assert_eq!(format_time(Lang::Fr, 14, 5), "Il est 14 h 05.");
        assert_eq!(format_time(Lang::En, 14, 5), "It's 2:05 PM.");
        assert_eq!(format_time(Lang::En, 0, 30), "It's 12:30 AM.");
        assert_eq!(format_date(Lang::Fr, 3, 2, 10), "Nous sommes le jeudi 2 octobre.");
        assert_eq!(format_date(Lang::Fr, 2, 1, 1), "Nous sommes le mercredi 1er janvier.");
        assert_eq!(format_date(Lang::En, 3, 2, 10), "It's Thursday, October 2.");

        let report = WeatherReport { place: "Paris".into(), day: Day::Today, now: Some(17.4), code: 3, min: 11.6, max: 19.2 };
        assert_eq!(format_weather(Lang::Fr, &report), "Paris : 17°, couvert (12° / 19°)");
        let tomorrow = WeatherReport { day: Day::Tomorrow, now: None, code: 61, ..report };
        assert_eq!(format_weather(Lang::En, &tomorrow), "Tomorrow in Paris: rain, 12° / 19°");
    }
}
