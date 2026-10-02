//! Reads the notifications currently in Windows' notification center.
//!
//! The supported API for this (`UserNotificationListener`) is only open to
//! packaged (MSIX) apps, so this reads the per-user notification database
//! Windows keeps (`wpndatabase.db`), read-only. It's undocumented and may
//! change between Windows versions — failures are reported, never fatal.

use std::path::PathBuf;

use mimo_core::notifications::NotificationItem;
use mimo_core::AppCatalog;
use rusqlite::{Connection, OpenFlags};

const LIMIT: usize = 20;

/// Windows FILETIME (100 ns ticks since 1601) → Unix seconds.
fn filetime_to_unix(ticks: i64) -> i64 {
    ticks / 10_000_000 - 11_644_473_600
}

fn database_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join(r"Microsoft\Windows\Notifications\wpndatabase.db"))
}

/// Most recent first. `apps` turns sender ids into app names.
pub fn read(apps: &AppCatalog) -> Result<Vec<NotificationItem>, String> {
    let path = database_path().ok_or("LOCALAPPDATA is not set")?;
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|err| format!("can't open the notification database: {err}"))?;

    let mut statement = connection
        .prepare(
            "SELECT h.PrimaryId, n.ArrivalTime, n.Payload
             FROM Notification n JOIN NotificationHandler h ON h.RecordId = n.HandlerId
             WHERE n.Type = 'toast'
             ORDER BY n.ArrivalTime DESC
             LIMIT ?1",
        )
        .map_err(|err| format!("unexpected notification database layout: {err}"))?;

    let rows = statement
        .query_map([LIMIT as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Vec<u8>>(2)?))
        })
        .map_err(|err| err.to_string())?;

    let mut items = Vec::new();
    for row in rows.flatten() {
        let (sender, arrival, payload) = row;
        let texts = toast_texts(&String::from_utf8_lossy(&payload));
        let Some((title, body)) = texts.split_first() else {
            continue;
        };
        items.push(NotificationItem {
            app: app_name(&sender, apps),
            title: title.clone(),
            body: body.join(" — "),
            time: filetime_to_unix(arrival),
        });
    }
    Ok(items)
}

/// The `<text>` elements of a toast's XML, unescaped.
fn toast_texts(xml: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<text") {
        rest = &rest[start..];
        let Some(open_end) = rest.find('>') else { break };
        // Self-closing <text/> carries nothing.
        if rest[..open_end].ends_with('/') {
            rest = &rest[open_end + 1..];
            continue;
        }
        let content_start = open_end + 1;
        let Some(close) = rest[content_start..].find("</text>") else { break };
        let text = unescape(rest[content_start..content_start + close].trim());
        if !text.is_empty() {
            texts.push(text);
        }
        rest = &rest[content_start + close..];
    }
    texts
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|i| *i <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// "com.squirrel.Discord.Discord" → "Discord": the installed app with that
/// id if known, otherwise a readable piece of the id.
fn app_name(sender: &str, apps: &AppCatalog) -> String {
    if let Some(app) = apps.apps().iter().find(|a| a.id.eq_ignore_ascii_case(sender)) {
        return app.name.clone();
    }
    // Package ids look like "Publisher.App_hash!Entry": keep the app part.
    let base = sender.split(['!', '_']).next().unwrap_or(sender);
    let part = base
        .rsplit(['.', '\\'])
        .find(|p| !p.is_empty() && !p.eq_ignore_ascii_case("exe"))
        .unwrap_or(base);
    part.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_toast_texts() {
        let xml = r#"<toast><visual><binding template="ToastGeneric"><text id="1">L&#233;o</text><text>t&apos;es l&#xE0; ?</text><text/></binding></visual></toast>"#;
        assert_eq!(toast_texts(xml), vec!["Léo".to_string(), "t'es là ?".to_string()]);
    }

    #[test]
    fn readable_app_names() {
        let apps = AppCatalog::default();
        assert_eq!(app_name("com.squirrel.Discord.Discord", &apps), "Discord");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", &apps), "SpotifyMusic");
        assert_eq!(app_name("MSEdge", &apps), "MSEdge");
    }

    #[test]
    fn converts_filetime() {
        // 2024-01-01T00:00:00Z
        assert_eq!(filetime_to_unix(133_485_408_000_000_000), 1_704_067_200);
    }
}
