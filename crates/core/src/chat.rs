//! Small talk ("salut", "comment ça va ?", "merci", "qui es-tu ?") so Mimo
//! feels like a companion rather than a command line, and "j'ai besoin
//! d'aide". A request is small talk only when *all* of it is: "salut, ouvre
//! youtube" still opens YouTube.

use crate::info::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Greeting,
    HowAreYou,
    /// "ça va, et toi ?": an answer and the question back.
    FineAndYou,
    /// "bien", "super": the user answering "et toi ?".
    UserFine,
    /// "bof", "pas trop".
    UserDown,
    Thanks,
    WhoAreYou,
    Goodbye,
    Compliment,
}

const GREETINGS: &[&str] = &[
    "bonjour", "salut", "coucou", "bonsoir", "hello", "hi", "hey", "he", "yo", "wesh", "slt", "cc", "hola",
];
/// Mimo's name as typed or as the speech models hear it, and words that
/// don't change what was said ("ok merci", "dis, t'es qui ?").
const IGNORED: &[&str] = &["mimo", "memo", "immo", "ok", "okay", "dis", "eh", "bon", "alors", "oh"];
const GREETING_PHRASES: &[&str] = &["good morning", "good evening", "good afternoon", "bonne apres midi"];

const HOW_ARE_YOU: &[&str] = &[
    "comment ca va", "ca va", "comment vas tu", "comment tu vas", "tu vas bien", "vas tu bien", "comment allez vous",
    "ca roule", "quoi de neuf", "la forme", "en forme", "how are you", "how are you doing", "how is it going",
    "how s it going", "what s up", "whats up", "sup", "you good", "how do you do", "how have you been",
];
const USER_FINE: &[&str] = &[
    "bien", "tres bien", "ca va bien", "ca va tres bien", "je vais bien", "je vais tres bien", "super", "nickel",
    "tranquille", "pas mal", "ca va super", "good", "great", "fine", "i m good", "i m fine", "i m great",
    "i am good", "i am fine", "very good", "pretty good", "not bad", "doing good", "doing great",
];
const USER_DOWN: &[&str] = &[
    "bof", "pas trop", "pas bien", "ca va pas", "ca ne va pas", "pas terrible", "pas top", "mal", "je suis fatigue",
    "je suis triste", "not great", "not good", "not so good", "bad", "not really", "i m tired", "i m sad",
    "i m not good", "i m not great",
];
const THANKS: &[&str] = &[
    "merci", "merci beaucoup", "merci bien", "merci infiniment", "super merci", "parfait merci", "top merci",
    "cool merci", "thanks", "thank you", "thanks a lot", "thank you so much", "thank you very much", "thx",
    "great thanks", "perfect thanks",
];
const WHO_ARE_YOU: &[&str] = &[
    "qui es tu", "qui est tu", "t es qui", "tu es qui", "c est quoi ton nom", "comment tu t appelles",
    "comment t appelles tu", "tu sers a quoi", "tu fais quoi", "qu est ce que tu sais faire", "who are you",
    "what s your name", "what is your name", "what are you", "what can you do",
];
const GOODBYE: &[&str] = &[
    "au revoir", "bye", "bye bye", "a plus", "a plus tard", "a tout a l heure", "a demain", "bonne nuit",
    "bonne journee", "bonne soiree", "ciao", "good night", "goodbye", "see you", "see you later",
    "see you tomorrow", "have a nice day", "have a good day", "later",
];
const COMPLIMENTS: &[&str] = &[
    "je t aime", "t es genial", "tu es genial", "t es le meilleur", "tu es le meilleur", "t es trop fort",
    "tu es trop fort", "bravo", "bien joue", "trop bien", "t es super", "tu es super", "i love you",
    "you re awesome", "you are awesome", "you re the best", "you are the best", "good job", "well done",
    "nice job", "you re great", "you are great",
];

/// Recognizes small talk; `tokens` as from `intent::tokenize`.
pub fn detect(tokens: &[String]) -> Option<Topic> {
    let mut greeted = false;
    let mut words: Vec<&str> = Vec::new();
    for token in tokens {
        let t = token.as_str();
        if GREETINGS.contains(&t) {
            greeted = true;
        } else if !IGNORED.contains(&t) {
            words.push(t);
        }
    }
    // "…, et toi ?" / "…, and you?"
    let asks_back = matches!(words.as_slice(), [.., "et", "toi"] | [.., "and", "you"] | [.., "et", "vous"]);
    if asks_back {
        words.truncate(words.len() - 2);
    }
    let said = words.join(" ");
    let is = |list: &[&str]| list.contains(&said.as_str());

    if said.is_empty() {
        return (greeted || asks_back).then_some(if asks_back { Topic::HowAreYou } else { Topic::Greeting });
    }
    if asks_back && (is(USER_FINE) || is(HOW_ARE_YOU)) {
        return Some(Topic::FineAndYou);
    }
    [
        (GREETING_PHRASES, Topic::Greeting),
        (HOW_ARE_YOU, Topic::HowAreYou),
        (USER_FINE, Topic::UserFine),
        (USER_DOWN, Topic::UserDown),
        (THANKS, Topic::Thanks),
        (WHO_ARE_YOU, Topic::WhoAreYou),
        (GOODBYE, Topic::Goodbye),
        (COMPLIMENTS, Topic::Compliment),
    ]
    .into_iter()
    .find_map(|(list, topic)| is(list).then_some(topic))
}

/// What Mimo says back. `variant` picks among a few wordings (any number,
/// e.g. the current second) so it doesn't always say the same thing;
/// `hour` (0–23, local) lets it say "bonsoir" or "bonne nuit".
pub fn reply(topic: Topic, lang: Lang, variant: usize, hour: u32) -> String {
    let evening = !(5..18).contains(&hour);
    let night = !(5..21).contains(&hour);
    let pick = |options: &[&str]| options[variant % options.len()].to_string();
    match (lang, topic) {
        (Lang::Fr, Topic::Greeting) => pick(&[
            if evening { "Bonsoir ! Qu'est-ce que je peux faire pour toi ?" } else { "Bonjour ! Qu'est-ce que je peux faire pour toi ?" },
            "Salut ! Content de te voir. Je t'écoute.",
            "Coucou ! Besoin d'un coup de main ?",
        ]),
        (Lang::Fr, Topic::HowAreYou) => pick(&[
            "Ça va très bien, merci ! Et toi ?",
            "Toujours en forme, prêt à t'aider ! Et toi, ça va ?",
            "Super bien ! Ton PC tourne, donc moi aussi. Et toi ?",
        ]),
        (Lang::Fr, Topic::FineAndYou) => pick(&[
            "Ça va très bien aussi, merci de demander !",
            "Moi aussi, ça roule ! Je suis là si tu as besoin.",
        ]),
        (Lang::Fr, Topic::UserFine) => pick(&[
            "Tant mieux ! Dis-moi si je peux t'aider.",
            "Ça fait plaisir à entendre !",
        ]),
        (Lang::Fr, Topic::UserDown) => pick(&[
            "Oh, désolé… Si je peux faire quelque chose, dis-le-moi. Une petite pause te ferait peut-être du bien ?",
            "Courage, ça va aller. Je suis là si tu as besoin de quoi que ce soit.",
        ]),
        (Lang::Fr, Topic::Thanks) => pick(&["Avec plaisir !", "De rien, c'est normal !", "Toujours là pour toi."]),
        (Lang::Fr, Topic::WhoAreYou) => pick(&[
            "Je suis Mimo, ton compagnon de bureau : j'ouvre tes apps, je te rappelle des choses et je garde un œil sur ton PC.",
            "Moi, c'est Mimo ! Demande-moi d'ouvrir une app, de noter une tâche, de faire une capture… je m'occupe du reste.",
        ]),
        (Lang::Fr, Topic::Goodbye) => pick(&[
            if night { "Bonne nuit, repose-toi bien !" } else if evening { "Bonne soirée !" } else { "Bonne journée !" },
            "À plus tard !",
        ]),
        (Lang::Fr, Topic::Compliment) => pick(&[
            "Oh, merci, ça me touche !",
            "Tu vas me faire rougir !",
            "Merci ! Toi aussi, tu es génial.",
        ]),
        (Lang::En, Topic::Greeting) => pick(&[
            if evening { "Good evening! What can I do for you?" } else { "Hello! What can I do for you?" },
            "Hi! Good to see you. I'm listening.",
            "Hey there! Need a hand?",
        ]),
        (Lang::En, Topic::HowAreYou) => pick(&[
            "I'm doing great, thanks! How about you?",
            "Always ready to help! How are you doing?",
            "Great! Your PC is running, so I am too. And you?",
        ]),
        (Lang::En, Topic::FineAndYou) => pick(&[
            "I'm doing great too, thanks for asking!",
            "All good here! I'm around if you need me.",
        ]),
        (Lang::En, Topic::UserFine) => pick(&["Glad to hear it! Let me know if I can help.", "That's great to hear!"]),
        (Lang::En, Topic::UserDown) => pick(&[
            "Oh, I'm sorry… If I can do anything, just ask. Maybe a short break would help?",
            "Hang in there. I'm here if you need anything.",
        ]),
        (Lang::En, Topic::Thanks) => pick(&["You're welcome!", "Anytime!", "Always happy to help."]),
        (Lang::En, Topic::WhoAreYou) => pick(&[
            "I'm Mimo, your desktop companion: I open your apps, remind you of things and keep an eye on your PC.",
            "I'm Mimo! Ask me to open an app, add a task, take a screenshot… I'll handle the rest.",
        ]),
        (Lang::En, Topic::Goodbye) => pick(&[
            if night { "Good night, sleep well!" } else if evening { "Have a nice evening!" } else { "Have a great day!" },
            "See you later!",
        ]),
        (Lang::En, Topic::Compliment) => pick(&[
            "Aw, thank you!",
            "You're making me blush!",
            "Thanks! You're pretty great yourself.",
        ]),
    }
}

/// "J'ai besoin d'aide", "aide-moi", "help", "au secours".
pub fn is_help_request(tokens: &[String]) -> bool {
    const HELP: &[&str] = &["aide", "aider", "aidez", "help", "secours", "sos"];
    const OPEN: &[&str] = &["ouvre", "lance", "open", "launch", "start", "demarre"];
    tokens.len() <= 7
        && tokens.iter().any(|t| HELP.contains(&t.as_str()))
        // "ouvre l'aide de Windows" opens the Get Help app.
        && !tokens.first().is_some_and(|t| OPEN.contains(&t.as_str()))
}

pub fn help_question(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Tu as besoin des secours, ou c'est une simple question ou un problème me concernant ?",
        Lang::En => "Do you need emergency services, or is it just a question or a problem about me?",
    }
}

/// As the speech models write them.
pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "hello", "hi", "good evening", "how are you", "how are you doing", "what's up", "thank you", "thanks",
            "who are you", "what's your name", "what can you do", "good night", "goodbye", "see you later",
            "i'm good", "i'm fine", "i love you", "good job", "i need help", "help", "help me",
        ],
        _ => &[
            "bonjour", "salut", "coucou", "bonsoir", "comment ça va", "ça va", "ça va et toi", "comment vas tu",
            "tu vas bien", "merci", "merci beaucoup", "qui es tu", "comment tu t'appelles", "bonne nuit",
            "au revoir", "à plus", "bonne journée", "très bien", "je vais bien", "je t'aime", "bravo",
            "j'ai besoin d'aide", "aide moi", "au secours",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn topic(text: &str) -> Option<Topic> {
        detect(&tokenize(text))
    }

    #[test]
    fn small_talk() {
        assert_eq!(topic("Salut Mimo !"), Some(Topic::Greeting));
        assert_eq!(topic("bonjour, comment ça va ?"), Some(Topic::HowAreYou));
        assert_eq!(topic("hey how's it going"), Some(Topic::HowAreYou));
        assert_eq!(topic("ça va et toi ?"), Some(Topic::FineAndYou));
        assert_eq!(topic("très bien"), Some(Topic::UserFine));
        assert_eq!(topic("bof"), Some(Topic::UserDown));
        assert_eq!(topic("merci beaucoup"), Some(Topic::Thanks));
        assert_eq!(topic("t'es qui ?"), Some(Topic::WhoAreYou));
        assert_eq!(topic("what's your name"), Some(Topic::WhoAreYou));
        assert_eq!(topic("bonne nuit Mimo"), Some(Topic::Goodbye));
        assert_eq!(topic("t'es génial"), Some(Topic::Compliment));
    }

    #[test]
    fn requests_are_not_small_talk() {
        for text in ["salut ouvre youtube", "bonjour quelle heure est-il", "merci de m'ouvrir spotify", "ouvre la météo"] {
            assert_eq!(topic(text), None, "{text}");
        }
    }

    #[test]
    fn replies_vary_and_follow_the_time() {
        assert_ne!(reply(Topic::Thanks, Lang::Fr, 0, 12), reply(Topic::Thanks, Lang::Fr, 1, 12));
        assert!(reply(Topic::Greeting, Lang::Fr, 0, 20).starts_with("Bonsoir"));
        assert!(reply(Topic::Goodbye, Lang::En, 0, 23).contains("night"));
    }

    #[test]
    fn help_requests() {
        assert!(is_help_request(&tokenize("j'ai besoin d'aide")));
        assert!(is_help_request(&tokenize("au secours")));
        assert!(is_help_request(&tokenize("I need help")));
        assert!(!is_help_request(&tokenize("ouvre l'aide de windows")));
    }
}
