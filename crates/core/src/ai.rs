//! The optional local AI (setting `ai_enabled`, off by default): a small
//! language model running on the PC, never online. This module is the
//! pure part — the prompts, reading the model's answers back, and what it
//! may trigger. The shell downloads and runs the model.
//!
//! The model never executes anything. To understand a request the rules
//! didn't, it rewrites it as one of Mimo's own phrasings ("j'entends rien"
//! → "monte le son"), which then goes through the usual rule-based parser
//! and its whitelists — or it answers in words.

use serde::{Deserialize, Serialize};

use crate::control::ControlCommand;
use crate::info::Lang;
use crate::intent::Intent;
use crate::reminders::ReminderCommand;
use crate::tasks::TaskCommand;

/// A chat message for the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

fn system(content: String) -> Message {
    Message { role: "system", content }
}

fn user(content: String) -> Message {
    Message { role: "user", content }
}

/// What the model made of a request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum Understanding {
    /// One of Mimo's own phrasings, to parse with the rules.
    Command(String),
    /// A reply to show.
    Answer(String),
}

/// The JSON the model is constrained to when understanding a request.
pub fn understanding_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": ["command", "answer"] },
            "text": { "type": "string" }
        },
        "required": ["kind", "text"]
    })
}

/// What Mimo can do, as the phrasings its parser knows (the model picks
/// one and fills in the blanks).
const COMMANDS: &[&str] = &[
    "ouvre <app ou site>",
    "cherche <recherche> sur google",
    "cherche <recherche> sur youtube",
    "mets <vidéo ou chaîne> sur youtube",
    "<artiste ou chanson> sur spotify",
    "quelle heure est-il",
    "quelle date sommes-nous",
    "quel temps fait-il",
    "quel temps fait-il à <ville>",
    "monte le son",
    "baisse le son",
    "mets le son à <0-100>",
    "coupe le son",
    "remets le son",
    "augmente la luminosité",
    "baisse la luminosité",
    "pause",
    "chanson suivante",
    "chanson précédente",
    "fais un bilan du pc",
    "lis mes notifications",
    "rappelle-moi dans <durée> de <quoi>",
    "rappelle-moi à <heure> de <quoi>",
    "mets un réveil à <heure>",
    "mes rappels",
    "ajoute une tâche <titre> <quand>",
    "qu'est-ce que j'ai à faire aujourd'hui",
    "traduis: <texte>",
    "résumé de ma journée",
    "temps d'écran cette semaine",
    "combien de temps j'ai passé sur <app>",
    "qu'est-ce que je faisais <quand>",
    "rouvre ce que j'avais ouvert",
    "prends une capture d'écran",
    "enregistre l'écran",
    "arrête l'enregistrement",
];

/// The prompt that turns a request the rules didn't get into a command or
/// an answer. `now`: e.g. "lundi 5 octobre 2026, 14:05".
pub fn understanding_prompt(request: &str, lang: Lang, now: &str) -> Vec<Message> {
    let commands = COMMANDS.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n");
    let language = match lang {
        Lang::Fr => "français",
        Lang::En => "English",
    };
    vec![
        system(format!(
            "Tu es Mimo, un petit assistant sur un PC Windows. On est le {now}.\n\
             Si la demande correspond à une de ces actions, réponds {{\"kind\": \"command\", \"text\": \"…\"}} \
             avec l'action écrite exactement sur ce modèle (remplis les <…>) :\n{commands}\n\
             Pense à ce que la personne veut obtenir. Exemples : « j'entends rien » → monte le son ; \
             « c'est trop fort » → baisse le son ; « on voit rien sur l'écran » → augmente la luminosité ; \
             « je veux écouter de la musique » → ouvre spotify ; « il est quelle heure » → quelle heure est-il.\n\
             Sinon (question, conseil, idée, conversation), réponds {{\"kind\": \"answer\", \"text\": \"…\"}} : \
             une réponse utile et exacte, en {language}, en 1 à 3 phrases, sans markdown. \
             Dans une réponse, ne propose jamais de faire une action toi-même (rappel, ouverture…). \
             Si tu ne sais pas, dis-le simplement."
        )),
        user(request.to_string()),
    ]
}

/// Reads the model's output (JSON, possibly wrapped in stray text).
pub fn parse_understanding(output: &str) -> Option<Understanding> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    let understanding: Understanding = serde_json::from_str(output.get(start..=end)?).ok()?;
    match &understanding {
        Understanding::Command(text) | Understanding::Answer(text) if text.trim().is_empty() => None,
        _ => Some(understanding),
    }
}

/// What the model's rewording may trigger: nothing that can't be undone or
/// that would be a nasty surprise if it misread ("j'ai sommeil" → sleep).
pub fn allowed(intent: &Intent) -> bool {
    !matches!(
        intent,
        Intent::Unknown
            | Intent::Control { command: ControlCommand::Lock | ControlCommand::Sleep, .. }
            | Intent::Reminder { command: ReminderCommand::Cancel(_), .. }
            | Intent::Task { command: TaskCommand::Delete { .. } | TaskCommand::Complete { .. }, .. }
            | Intent::Recall { command: crate::recall::RecallCommand::Reopen(_), .. }
            | Intent::Custom { .. }
    )
}

/// Something to do with the text the user copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTool {
    Summarize,
    Fix,
    Rephrase,
    Explain,
}

const COPIED_WORDS: &[&str] = &[
    "copie", "copier", "copies", "presse", "papier", "papiers", "texte", "copied", "clipboard", "text", "ca", "this",
];

/// "résume ce que j'ai copié", "corrige les fautes", "reformule ce texte",
/// "explique ce que j'ai copié", "summarize what I copied", "fix the
/// spelling", "rephrase this".
pub fn detect_text_tool(tokens: &[String]) -> Option<TextTool> {
    let has_any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    let about_copy = has_any(COPIED_WORDS);
    if has_any(&["corrige", "corriger"]) && (about_copy || has_any(&["faute", "fautes", "orthographe"])) {
        return Some(TextTool::Fix);
    }
    if has_any(&["fix", "correct", "proofread"]) && (about_copy || has_any(&["spelling", "grammar", "typos", "mistakes"])) {
        return Some(TextTool::Fix);
    }
    if has_any(&["reformule", "reformuler", "rephrase", "reword", "rewrite", "reecris"]) {
        return Some(TextTool::Rephrase);
    }
    if !about_copy {
        return None;
    }
    if has_any(&["resume", "resumer", "summarize", "summarise", "summary"]) {
        return Some(TextTool::Summarize);
    }
    if has_any(&["explique", "expliquer", "explain"]) {
        return Some(TextTool::Explain);
    }
    None
}

pub fn text_tool_prompt(tool: TextTool, text: &str, lang: Lang) -> Vec<Message> {
    let language = match lang {
        Lang::Fr => "en français",
        Lang::En => "in English",
    };
    let instruction = match tool {
        TextTool::Summarize => format!("Résume ce texte {language} en 2 à 4 phrases claires, sans markdown."),
        TextTool::Fix => "Corrige les fautes d'orthographe, de grammaire et de ponctuation de ce texte, dans sa \
                          langue d'origine, sans changer le sens ni le ton. Renvoie seulement le texte corrigé."
            .to_string(),
        TextTool::Rephrase => "Reformule ce texte dans sa langue d'origine, plus clair et plus poli, sans changer le \
                               sens. Renvoie seulement le texte reformulé."
            .to_string(),
        TextTool::Explain => format!("Explique simplement ce texte {language} en 2 à 4 phrases, sans markdown."),
    };
    vec![system(format!("Tu es Mimo, un assistant. {instruction}")), user(text.to_string())]
}

/// The heading of the card showing the result.
pub fn text_tool_title(tool: TextTool, lang: Lang) -> &'static str {
    match (tool, lang) {
        (TextTool::Summarize, Lang::Fr) => "Résumé",
        (TextTool::Summarize, Lang::En) => "Summary",
        (TextTool::Fix, Lang::Fr) => "Correction",
        (TextTool::Fix, Lang::En) => "Corrected",
        (TextTool::Rephrase, Lang::Fr) => "Reformulation",
        (TextTool::Rephrase, Lang::En) => "Rephrased",
        (TextTool::Explain, Lang::Fr) => "Explication",
        (TextTool::Explain, Lang::En) => "Explanation",
    }
}

/// Rewords a factual, template-made answer (a day's summary, "what was I
/// doing") as a natural sentence — the figures must stay as they are.
pub fn recap_prompt(facts: &str, lang: Lang) -> Vec<Message> {
    let language = match lang {
        Lang::Fr => "en français, en tutoyant",
        Lang::En => "in English",
    };
    vec![
        system(format!(
            "Tu es Mimo, un assistant sympa sur un PC. Reformule ce constat {language}, en une ou deux phrases \
             naturelles et chaleureuses, sans juger. Garde exactement les noms d'applications, les heures et les durées, \
             n'invente rien, pas de markdown, pas d'emoji. Renvoie seulement la phrase."
        )),
        user(facts.to_string()),
    ]
}

/// The model's free text, tidied: no surrounding quotes or markdown bold.
pub fn clean_text(output: &str) -> String {
    let trimmed = output.trim().trim_matches(|c| c == '"' || c == '«' || c == '»' || c == '“' || c == '”').trim();
    trimmed.replace("**", "")
}

/// When the AI is needed but off or not downloaded yet.
pub fn unavailable_message(lang: Lang, downloading: bool) -> &'static str {
    match (lang, downloading) {
        (Lang::Fr, false) => "Pour ça, active l'IA locale dans les réglages de Mimo.",
        (Lang::En, false) => "For that, turn on the local AI in Mimo's settings.",
        (Lang::Fr, true) => "L'IA locale est encore en cours de téléchargement.",
        (Lang::En, true) => "The local AI is still downloading.",
    }
}

pub fn failed_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "L'IA locale n'a pas réussi à répondre.",
        Lang::En => "The local AI couldn't answer.",
    }
}

pub fn nothing_copied_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Copie d'abord un texte (Ctrl+C), puis redemande-moi.",
        Lang::En => "Copy some text first (Ctrl+C), then ask me again.",
    }
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "summarize what i copied", "summarize the text", "fix the spelling", "fix the mistakes",
            "correct what i copied", "rephrase this", "rephrase what i copied", "explain what i copied",
        ],
        _ => &[
            "résume ce que j'ai copié", "résume le texte", "corrige les fautes", "corrige ce que j'ai copié",
            "corrige le texte", "reformule ça", "reformule ce que j'ai copié", "explique ce que j'ai copié",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{parse, tokenize};

    #[test]
    fn reads_the_models_answers() {
        assert_eq!(
            parse_understanding(r#"{"kind": "command", "text": "monte le son"}"#),
            Some(Understanding::Command("monte le son".into()))
        );
        assert_eq!(
            parse_understanding("Voici : {\"kind\":\"answer\",\"text\":\"La photosynthèse…\"} "),
            Some(Understanding::Answer("La photosynthèse…".into()))
        );
        assert_eq!(parse_understanding(r#"{"kind": "answer", "text": "  "}"#), None);
        assert_eq!(parse_understanding("pas du json"), None);
    }

    #[test]
    fn every_listed_command_parses() {
        // The placeholders filled in as the model would.
        for command in COMMANDS {
            let filled = command
                .replace("<app ou site>", "spotify")
                .replace("<recherche>", "recette de crêpes")
                .replace("<vidéo ou chaîne>", "squeezie")
                .replace("<artiste ou chanson>", "damso")
                .replace("<ville>", "Lyon")
                .replace("<0-100>", "30")
                .replace("<durée>", "10 minutes")
                .replace("<heure>", "7h30")
                .replace("<quoi>", "sortir le linge")
                .replace("<titre>", "appeler le plombier")
                .replace("<quand>", "demain")
                .replace("<texte>", "hello")
                .replace("<app>", "discord");
            assert_ne!(parse(&filled), Intent::Unknown, "{filled}");
        }
    }

    #[test]
    fn risky_rewordings_are_refused() {
        assert!(!allowed(&parse("mets le pc en veille")));
        assert!(!allowed(&parse("verrouille le pc")));
        assert!(!allowed(&parse("annule mes rappels")));
        assert!(!allowed(&parse("rouvre ce que j'avais ouvert")));
        assert!(allowed(&parse("monte le son")));
        assert!(allowed(&parse("ouvre youtube")));
    }

    #[test]
    fn understands_text_tools() {
        let q = |text: &str| detect_text_tool(&tokenize(text));
        assert_eq!(q("résume ce que j'ai copié"), Some(TextTool::Summarize));
        assert_eq!(q("corrige les fautes"), Some(TextTool::Fix));
        assert_eq!(q("fix the spelling"), Some(TextTool::Fix));
        assert_eq!(q("reformule ça plus poliment"), Some(TextTool::Rephrase));
        assert_eq!(q("explain what I copied"), Some(TextTool::Explain));
        for text in ["résumé de ma journée", "ouvre youtube", "explique-moi la photosynthèse", "corrige", "copie"] {
            assert_eq!(q(text), None, "{text}");
        }
    }

    #[test]
    fn tidies_free_text() {
        assert_eq!(clean_text("  \"Bonjour **toi**\"\n"), "Bonjour toi");
    }
}
