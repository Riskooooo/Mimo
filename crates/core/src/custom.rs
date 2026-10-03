//! The user's own commands ("Mes commandes" in the settings): when a
//! request is exactly a trigger phrase ("ma chaîne"), Mimo replies something,
//! opens a site, opens an installed app, or treats it as another request
//! ("mets ma chaîne sur youtube"). Never a command line: sites are http(s) URLs,
//! apps come from Windows' own Start menu list.

use serde::{Deserialize, Serialize};

use crate::info::Lang;
use crate::intent::tokenize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomCommand {
    pub id: u64,
    /// As the user wrote it ("Ma chaîne").
    pub trigger: String,
    pub action: CustomAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CustomAction {
    /// Say this in the pill.
    Reply { text: String },
    /// Open this http(s) URL.
    OpenUrl { url: String },
    /// Launch an installed app, by its Start menu id.
    LaunchApp { name: String, app_id: String },
    /// Handle it as if this had been said ("mets ma chaîne sur youtube").
    Request { text: String },
}

/// Words that don't change which command is meant: "hey Mimo, ma chaîne stp".
const IGNORED: &[&str] = &[
    "hey", "he", "eh", "ok", "okay", "salut", "dis", "mimo", "memo", "immo", "stp", "svp", "please", "s", "il",
    "te", "vous", "plait",
];

/// How a request and a trigger are compared: lowercase, no accents or
/// punctuation, without greetings and "please".
pub fn key(text: &str) -> String {
    tokenize(text).into_iter().filter(|t| !IGNORED.contains(&t.as_str())).collect::<Vec<_>>().join(" ")
}

/// The command `request` triggers, if any.
pub fn find<'a>(commands: &'a [CustomCommand], request: &str) -> Option<&'a CustomCommand> {
    let said = key(request);
    if said.is_empty() {
        return None;
    }
    commands.iter().find(|c| key(&c.trigger) == said)
}

/// The trigger as the speech models would write it, for the voice grammar.
pub fn voice_phrase(command: &CustomCommand) -> String {
    command
        .trigger
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Checks and tidies a command before it's saved (trimmed, URL with a
/// scheme); the error is worded for the user.
pub fn validate(mut command: CustomCommand, others: &[CustomCommand], lang: Lang) -> Result<CustomCommand, String> {
    let fr = lang == Lang::Fr;
    command.trigger = command.trigger.trim().to_string();
    let trigger = key(&command.trigger);
    if trigger.is_empty() {
        return Err(if fr { "Écris la phrase qui déclenche la commande." } else { "Write the phrase that triggers it." }.into());
    }
    if others.iter().any(|o| o.id != command.id && key(&o.trigger) == trigger) {
        return Err(if fr { "Tu as déjà une commande pour cette phrase." } else { "You already have a command for that phrase." }.into());
    }
    let empty = |text: &str| text.trim().is_empty();
    match &mut command.action {
        CustomAction::Reply { text } | CustomAction::Request { text } if empty(text) => {
            return Err(if fr { "Écris ce que Mimo doit faire." } else { "Write what Mimo should do." }.into());
        }
        CustomAction::Request { text } if key(text) == trigger => {
            return Err(if fr { "La commande ne peut pas se déclencher elle-même." } else { "A command can't trigger itself." }.into());
        }
        CustomAction::Reply { text } | CustomAction::Request { text } => *text = text.trim().to_string(),
        CustomAction::OpenUrl { url } => {
            let trimmed = url.trim();
            let full = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
                trimmed.to_string()
            } else {
                format!("https://{trimmed}")
            };
            let host = full.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or_default();
            if !host.contains('.') || host.contains(char::is_whitespace) {
                return Err(if fr { "Cette adresse de site n'est pas valide." } else { "That web address isn't valid." }.into());
            }
            *url = full;
        }
        CustomAction::LaunchApp { app_id, .. } if app_id.is_empty() => {
            return Err(if fr { "Choisis une application." } else { "Pick an app." }.into());
        }
        CustomAction::LaunchApp { .. } => {}
    }
    Ok(command)
}

/// The saved commands, persisted by the shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomStore {
    pub commands: Vec<CustomCommand>,
    next_id: u64,
}

impl CustomStore {
    /// Adds the command (id 0) or replaces the one with its id.
    pub fn save(&mut self, mut command: CustomCommand) -> u64 {
        if let Some(existing) = self.commands.iter_mut().find(|c| c.id == command.id && command.id != 0) {
            *existing = command.clone();
            return command.id;
        }
        self.next_id += 1;
        command.id = self.next_id;
        self.commands.push(command);
        self.next_id
    }

    pub fn remove(&mut self, id: u64) {
        self.commands.retain(|c| c.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(trigger: &str, action: CustomAction) -> CustomCommand {
        CustomCommand { id: 0, trigger: trigger.into(), action }
    }

    #[test]
    fn requests_match_triggers_loosely() {
        let commands = vec![CustomCommand { id: 1, ..command("Ma chaîne", CustomAction::Reply { text: "Salut".into() }) }];
        assert_eq!(find(&commands, "hey Mimo, ma chaîne !").map(|c| c.id), Some(1));
        assert_eq!(find(&commands, "ma chaîne stp").map(|c| c.id), Some(1));
        assert_eq!(find(&commands, "ma chaîne youtube"), None);
        assert_eq!(find(&commands, "hey mimo"), None);
    }

    #[test]
    fn validation() {
        let url = validate(command("ma chaîne", CustomAction::OpenUrl { url: " youtube.com/@moi ".into() }), &[], Lang::Fr).unwrap();
        assert_eq!(url.action, CustomAction::OpenUrl { url: "https://youtube.com/@moi".into() });
        assert!(validate(command("x", CustomAction::OpenUrl { url: "pas une url".into() }), &[], Lang::Fr).is_err());
        assert!(validate(command("  ", CustomAction::Reply { text: "a".into() }), &[], Lang::Fr).is_err());
        assert!(validate(command("ma chaîne", CustomAction::Request { text: "Ma chaîne !".into() }), &[], Lang::Fr).is_err());
        let existing = vec![CustomCommand { id: 4, ..command("Ma chaîne", CustomAction::Reply { text: "a".into() }) }];
        assert!(validate(command("ma chaîne", CustomAction::Reply { text: "b".into() }), &existing, Lang::Fr).is_err());
        // Editing that same command is fine.
        assert!(validate(CustomCommand { id: 4, ..command("ma chaîne", CustomAction::Reply { text: "b".into() }) }, &existing, Lang::Fr).is_ok());
    }

    #[test]
    fn store_adds_and_replaces() {
        let mut store = CustomStore::default();
        let id = store.save(command("a", CustomAction::Reply { text: "1".into() }));
        store.save(CustomCommand { id, ..command("a", CustomAction::Reply { text: "2".into() }) });
        assert_eq!(store.commands.len(), 1);
        assert_eq!(store.commands[0].action, CustomAction::Reply { text: "2".into() });
        store.remove(id);
        assert!(store.commands.is_empty());
    }

    #[test]
    fn voice_phrases_are_lowercase_words() {
        assert_eq!(voice_phrase(&command("Ma Chaîne YouTube !", CustomAction::Reply { text: "a".into() })), "ma chaîne youtube");
    }
}
