//! Rules for the global "summon Mimo" keyboard shortcut.
//!
//! A global shortcut is taken away from every other app while Mimo runs, so
//! a bare letter (or Shift+letter) would make that key impossible to type
//! anywhere else. Only function keys may be bound on their own; anything else
//! needs Ctrl, Alt or the Windows key.

use crate::error::CoreError;

pub const DEFAULT_SUMMON_SHORTCUT: &str = "F9";

const MODIFIERS: &[&str] = &[
    "ctrl", "control", "alt", "option", "shift", "super", "cmd", "command", "win", "meta",
    "commandorcontrol", "cmdorctrl",
];

/// Modifiers that make any key safe to grab globally (Shift alone does not).
const STRONG_MODIFIERS: &[&str] = &[
    "ctrl", "control", "alt", "option", "super", "cmd", "command", "win", "meta",
    "commandorcontrol", "cmdorctrl",
];

/// Validates an accelerator string such as `"F9"` or `"Ctrl+Shift+M"`.
pub fn validate_shortcut(shortcut: &str) -> Result<(), CoreError> {
    let invalid = |reason: &str| Err(CoreError::InvalidShortcut(reason.to_string()));

    let parts: Vec<String> = shortcut.split('+').map(|p| p.trim().to_lowercase()).collect();
    if parts.iter().any(String::is_empty) {
        return invalid("empty shortcut");
    }
    let (key, modifiers) = parts.split_last().expect("split always yields one part");

    if MODIFIERS.contains(&key.as_str()) {
        return invalid("a shortcut needs a key besides modifiers");
    }
    if let Some(stray) = modifiers.iter().find(|m| !MODIFIERS.contains(&m.as_str())) {
        return Err(CoreError::InvalidShortcut(format!("“{stray}” is not a modifier")));
    }

    let has_strong_modifier = modifiers.iter().any(|m| STRONG_MODIFIERS.contains(&m.as_str()));
    if !has_strong_modifier && !is_function_key(key) {
        return invalid("add Ctrl, Alt or Win, or use a function key (F1–F24)");
    }
    Ok(())
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_keys_alone_are_allowed() {
        assert!(validate_shortcut("F5").is_ok());
        assert!(validate_shortcut("f12").is_ok());
        assert!(validate_shortcut("Shift+F9").is_ok());
    }

    #[test]
    fn modified_keys_are_allowed() {
        assert!(validate_shortcut("Ctrl+Shift+M").is_ok());
        assert!(validate_shortcut("Alt+Space").is_ok());
        assert!(validate_shortcut("Super+KeyK").is_ok());
    }

    #[test]
    fn plain_typing_keys_are_rejected() {
        assert!(validate_shortcut("M").is_err());
        assert!(validate_shortcut("Shift+M").is_err());
        assert!(validate_shortcut("Space").is_err());
        assert!(validate_shortcut("F25").is_err());
    }

    #[test]
    fn malformed_shortcuts_are_rejected() {
        assert!(validate_shortcut("").is_err());
        assert!(validate_shortcut("Ctrl+").is_err());
        assert!(validate_shortcut("Ctrl+Shift").is_err());
        assert!(validate_shortcut("Foo+M").is_err());
    }
}
