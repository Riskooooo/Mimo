//! "Lis mes notifications": recognizing the request and wording what Mimo
//! shows and says. Reading Windows' notifications is the shell's job.

use serde::Serialize;

use crate::info::Lang;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotificationItem {
    /// Sending app, as best known ("Discord").
    pub app: String,
    pub title: String,
    pub body: String,
    /// Unix seconds.
    pub time: i64,
}

/// How many notifications get read aloud; the rest are only listed.
const SPOKEN_LIMIT: usize = 5;

pub fn is_notifications_request(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|t| matches!(t.as_str(), "notification" | "notifications" | "notif" | "notifs"))
}

/// One line for the pill.
pub fn summary(items: &[NotificationItem], lang: Lang) -> String {
    let mut apps: Vec<&str> = Vec::new();
    for item in items {
        if !apps.contains(&item.app.as_str()) {
            apps.push(&item.app);
        }
    }
    let from = apps.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    match (lang, items.len()) {
        (Lang::Fr, 0) => "Aucune notification.".to_string(),
        (Lang::Fr, 1) => format!("1 notification ({from})."),
        (Lang::Fr, n) => format!("{n} notifications ({from})."),
        (Lang::En, 0) => "No notifications.".to_string(),
        (Lang::En, 1) => format!("1 notification ({from})."),
        (Lang::En, n) => format!("{n} notifications ({from})."),
    }
}

/// What to say aloud: a count, then the most recent ones.
pub fn speech(items: &[NotificationItem], lang: Lang) -> String {
    let mut text = match (lang, items.len()) {
        (Lang::Fr, 0) => return "Tu n'as aucune notification.".to_string(),
        (Lang::Fr, 1) => "Tu as une notification.".to_string(),
        (Lang::Fr, n) => format!("Tu as {n} notifications."),
        (Lang::En, 0) => return "You have no notifications.".to_string(),
        (Lang::En, 1) => "You have one notification.".to_string(),
        (Lang::En, n) => format!("You have {n} notifications."),
    };
    for item in items.iter().take(SPOKEN_LIMIT) {
        let content = [item.title.as_str(), item.body.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(". ");
        let from = match lang {
            Lang::Fr => format!(" De {} : ", item.app),
            Lang::En => format!(" From {}: ", item.app),
        };
        text.push_str(&from);
        text.push_str(&content);
        if !content.ends_with(['.', '!', '?']) {
            text.push('.');
        }
    }
    text
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "read my notifications", "read me my notifications", "my notifications",
            "what are my notifications", "any notifications", "do i have notifications",
        ],
        _ => &[
            "lis mes notifications", "lis moi mes notifications", "mes notifications",
            "quelles sont mes notifications", "est-ce que j'ai des notifications", "j'ai des notifications",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn item(app: &str, title: &str, body: &str) -> NotificationItem {
        NotificationItem { app: app.into(), title: title.into(), body: body.into(), time: 0 }
    }

    #[test]
    fn recognizes_requests() {
        assert!(is_notifications_request(&tokenize("lis-moi mes notifications")));
        assert!(is_notifications_request(&tokenize("read my notifications")));
        assert!(!is_notifications_request(&tokenize("ouvre youtube")));
    }

    #[test]
    fn summary_and_speech() {
        let items = vec![item("Discord", "Léo", "t'es là ?"), item("Gmail", "Facture", ""), item("Discord", "Sam", "go")];
        assert_eq!(summary(&items, Lang::Fr), "3 notifications (Discord, Gmail).");
        assert_eq!(
            speech(&items[..2], Lang::Fr),
            "Tu as 2 notifications. De Discord : Léo. t'es là ? De Gmail : Facture."
        );
        assert_eq!(speech(&[], Lang::En), "You have no notifications.");
    }
}
