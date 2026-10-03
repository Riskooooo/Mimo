//! Something to find or watch *on* a platform: "mets squeezie sur youtube",
//! "recherche la dernière vidéo d'inoxtag sur youtube", "gotaga sur
//! twitch", "damso feu de bois sur spotify", "ouvre spotify et affiche
//! damso", "cherche une pizzeria sur google maps". Only ever builds a URL
//! on the platform's own domain (or Spotify's `spotify:` link) with the
//! words as an encoded query.

use crate::apps::AppCatalog;
use crate::info::Lang;

/// What to open: a search on `platform`, or straight to a channel/profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRequest {
    pub platform: &'static str,
    pub query: String,
    pub url: String,
    /// The URL is a channel ("twitch.tv/gotaga"), not a search.
    pub channel: bool,
}

struct Platform {
    /// Accent-folded, as `intent::tokenize` writes them.
    aliases: &'static [&'static str],
    label: &'static str,
}

const PLATFORMS: &[Platform] = &[
    Platform { aliases: &["youtube", "you tube", "yt"], label: "YouTube" },
    Platform { aliases: &["twitch"], label: "Twitch" },
    Platform { aliases: &["spotify"], label: "Spotify" },
    Platform { aliases: &["deezer"], label: "Deezer" },
    Platform { aliases: &["soundcloud", "sound cloud"], label: "SoundCloud" },
    Platform { aliases: &["netflix"], label: "Netflix" },
    Platform { aliases: &["google maps", "maps", "plan", "plans"], label: "Google Maps" },
    Platform { aliases: &["google"], label: "Google" },
    Platform { aliases: &["wikipedia", "wiki"], label: "Wikipedia" },
    Platform { aliases: &["amazon"], label: "Amazon" },
    Platform { aliases: &["tiktok", "tik tok"], label: "TikTok" },
    Platform { aliases: &["twitter", "x"], label: "X" },
    Platform { aliases: &["reddit"], label: "Reddit" },
    Platform { aliases: &["github", "git hub"], label: "GitHub" },
    Platform { aliases: &["leboncoin", "le bon coin", "bon coin"], label: "Leboncoin" },
];

/// Verbs that may lead the request ("mets", "cherche", "regarde"…).
const VERBS: &[&str] = &[
    "ouvre", "ouvrir", "lance", "lancer", "mets", "met", "mettre", "affiche", "afficher", "montre", "montrer",
    "cherche", "chercher", "recherche", "rechercher", "trouve", "trouver", "regarde", "regarder", "ecoute",
    "ecouter", "joue", "jouer", "va", "open", "launch", "start", "put", "show", "search", "find", "look",
    "watch", "listen", "play", "go",
];
const SEARCH_VERBS: &[&str] = &[
    "cherche", "chercher", "recherche", "rechercher", "trouve", "trouver", "search", "find", "look",
];
const WATCH_VERBS: &[&str] = &["regarde", "regarder", "watch"];
const LISTEN_VERBS: &[&str] = &["ecoute", "ecouter", "listen"];
/// "… *sur* youtube", "… *on* spotify".
const LINKS: &[&str] = &["sur", "on", "dans", "in", "via"];
/// "ouvre spotify *et* mets damso".
const JOINERS: &[&str] = &["et", "and", "puis", "then"];

/// Words around what's actually searched: "*la dernière vidéo de*
/// inoxtag", "*le live de* gotaga", "*du* damso", "*for* lofi".
const QUERY_FILLERS: &[&str] = &[
    "moi", "me", "m", "la", "le", "les", "l", "un", "une", "des", "du", "de", "d", "the", "a", "an", "some",
    "derniere", "dernieres", "dernier", "derniers", "latest", "last", "new", "nouvelle", "nouvelles", "recente",
    "recentes", "recent", "video", "videos", "clip", "clips", "chaine", "channel", "live", "stream", "son", "sons",
    "chanson", "chansons", "musique", "musiques", "song", "songs", "music", "titre", "titres", "album", "playlist",
    "of", "from", "by", "pour", "for", "par", "to", "with", "avec", "quelque", "chose", "something",
];
const LATEST_WORDS: &[&str] = &[
    "derniere", "dernieres", "dernier", "derniers", "latest", "last", "new", "nouvelle", "nouvelles", "recente",
    "recentes", "recent",
];
const VIDEO_WORDS: &[&str] = &["video", "videos", "clip", "clips"];

/// Recognizes a request about something on a platform. `tokens` as from
/// `intent::tokenize`, politeness and preambles already removed.
pub fn detect(tokens: &[String], apps: &AppCatalog, lang: Lang) -> Option<PlatformRequest> {
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let verb = words.first().copied().filter(|w| VERBS.contains(w));
    let rest = if verb.is_some() { &words[1..] } else { &words[..] };

    let (platform, query) = platform_at_end(rest)
        .or_else(|| platform_at_start(rest))
        .or_else(|| open_then_ask(rest, verb))
        .or_else(|| implied_platform(rest, verb?, apps))?;

    let latest = platform.label == "YouTube"
        && query.iter().any(|w| LATEST_WORDS.contains(w))
        && query.iter().any(|w| VIDEO_WORDS.contains(w));
    let query = clean_query(query)?;
    // "ouvre youtube et twitch" isn't a search for "twitch".
    if PLATFORMS.iter().any(|p| p.aliases.contains(&query.as_str())) {
        return None;
    }
    Some(build(platform, query, latest, apps, lang))
}

/// "squeezie sur youtube", "damso feu de bois on spotify".
fn platform_at_end<'a>(words: &'a [&'a str]) -> Option<(&'static Platform, &'a [&'a str])> {
    PLATFORMS.iter().find_map(|platform| {
        platform.aliases.iter().find_map(|alias| {
            let alias: Vec<&str> = alias.split(' ').collect();
            let at = words.len().checked_sub(alias.len() + 1)?;
            (words[at + 1..] == alias[..] && LINKS.contains(&words[at])).then(|| (platform, &words[..at]))
        })
    })
}

/// "sur youtube squeezie", "on twitch gotaga".
fn platform_at_start<'a>(words: &'a [&'a str]) -> Option<(&'static Platform, &'a [&'a str])> {
    let (link, after) = words.split_first()?;
    if !LINKS.contains(link) {
        return None;
    }
    let (platform, len) = platform_prefix(after)?;
    Some((platform, &after[len..]))
}

/// "ouvre spotify et affiche damso", "open youtube and search lofi".
fn open_then_ask<'a>(words: &'a [&'a str], verb: Option<&str>) -> Option<(&'static Platform, &'a [&'a str])> {
    verb?;
    let (platform, len) = platform_prefix(words)?;
    let (joiner, asked) = words[len..].split_first()?;
    if !JOINERS.contains(joiner) {
        return None;
    }
    let asked = match asked.split_first() {
        Some((first, after)) if VERBS.contains(first) => after,
        _ => asked,
    };
    Some((platform, asked))
}

/// No platform said: "recherche la dernière vidéo d'inoxtag" is for
/// YouTube, "regarde …" too; "écoute damso" is for Spotify when it's
/// installed.
fn implied_platform<'a>(
    words: &'a [&'a str],
    verb: &str,
    apps: &AppCatalog,
) -> Option<(&'static Platform, &'a [&'a str])> {
    let label = if (SEARCH_VERBS.contains(&verb) || WATCH_VERBS.contains(&verb) || verb == "mets" || verb == "put")
        && words.iter().any(|w| VIDEO_WORDS.contains(w))
        || WATCH_VERBS.contains(&verb)
    {
        "YouTube"
    } else if LISTEN_VERBS.contains(&verb) {
        if spotify_app(apps) { "Spotify" } else { "YouTube" }
    } else {
        return None;
    };
    Some((PLATFORMS.iter().find(|p| p.label == label)?, words))
}

fn platform_prefix(words: &[&str]) -> Option<(&'static Platform, usize)> {
    PLATFORMS.iter().find_map(|platform| {
        platform.aliases.iter().find_map(|alias| {
            let alias: Vec<&str> = alias.split(' ').collect();
            words.starts_with(&alias).then_some((platform, alias.len()))
        })
    })
}

/// The searched words, without the fillers at either end.
fn clean_query(words: &[&str]) -> Option<String> {
    let first = words.iter().position(|w| !QUERY_FILLERS.contains(w))?;
    let last = words.iter().rposition(|w| !QUERY_FILLERS.contains(w))?;
    Some(words[first..=last].join(" "))
}

fn spotify_app(apps: &AppCatalog) -> bool {
    apps.exact(&["spotify".to_string()]).is_some()
}

fn build(platform: &'static Platform, query: String, latest: bool, apps: &AppCatalog, lang: Lang) -> PlatformRequest {
    let q = encode(&query, "+");
    let path = encode(&query, "%20");
    // A single word on Twitch is a channel name.
    let channel = platform.label == "Twitch" && !query.contains(' ');
    let url = match platform.label {
        "YouTube" if latest => format!("https://www.youtube.com/results?search_query={q}&sp=CAI%253D"),
        "YouTube" => format!("https://www.youtube.com/results?search_query={q}"),
        "Twitch" if channel => format!("https://www.twitch.tv/{query}"),
        "Twitch" => format!("https://www.twitch.tv/search?term={path}"),
        // The desktop app opens on its search page; the web player otherwise.
        "Spotify" if spotify_app(apps) => format!("spotify:search:{path}"),
        "Spotify" => format!("https://open.spotify.com/search/{path}"),
        "Deezer" => format!("https://www.deezer.com/search/{path}"),
        "SoundCloud" => format!("https://soundcloud.com/search?q={path}"),
        "Netflix" => format!("https://www.netflix.com/search?q={path}"),
        "Google Maps" => format!("https://www.google.com/maps/search/{q}"),
        "Wikipedia" if lang == Lang::En => format!("https://en.wikipedia.org/w/index.php?search={q}"),
        "Wikipedia" => format!("https://fr.wikipedia.org/w/index.php?search={q}"),
        "Amazon" if lang == Lang::En => format!("https://www.amazon.com/s?k={q}"),
        "Amazon" => format!("https://www.amazon.fr/s?k={q}"),
        "TikTok" => format!("https://www.tiktok.com/search?q={path}"),
        "X" => format!("https://x.com/search?q={path}"),
        "Reddit" => format!("https://www.reddit.com/search/?q={q}"),
        "GitHub" => format!("https://github.com/search?q={q}"),
        "Leboncoin" => format!("https://www.leboncoin.fr/recherche?text={q}"),
        _ => format!("https://www.google.com/search?q={q}"),
    };
    PlatformRequest { platform: platform.label, query, url, channel }
}

/// Percent-encodes `text` for a URL; spaces become `space`.
fn encode(text: &str, space: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push_str(space),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// As the speech models write them; "[unk]" is what was asked for.
pub fn voice_phrases(lang: &str) -> Vec<String> {
    let (verbs, link, platforms, extra): (&[&str], &str, &[&str], &[&str]) = match lang {
        "en" => (
            &["play", "put", "search", "search for", "find", "watch", "listen to", "open"],
            "on",
            &["youtube", "twitch", "spotify", "deezer", "netflix", "tiktok", "google maps", "amazon"],
            &[
                "[unk] on youtube", "[unk] on twitch", "[unk] on spotify", "open spotify and play [unk]",
                "open youtube and search [unk]", "open twitch and watch [unk]", "find the latest video from [unk]",
                "search the latest video from [unk] on youtube", "watch [unk]", "listen to [unk]",
            ],
        ),
        _ => (
            &["mets", "mets moi", "cherche", "recherche", "trouve", "regarde", "écoute", "lance", "ouvre", "affiche"],
            "sur",
            &["youtube", "twitch", "spotify", "deezer", "netflix", "tiktok", "google maps", "amazon"],
            &[
                "[unk] sur youtube", "[unk] sur twitch", "[unk] sur spotify", "ouvre spotify et mets [unk]",
                "ouvre spotify et affiche [unk]", "ouvre youtube et cherche [unk]", "ouvre youtube et mets [unk]",
                "ouvre twitch et regarde [unk]", "cherche la dernière vidéo de [unk]",
                "recherche la dernière vidéo de [unk]", "recherche la dernière vidéo de [unk] sur youtube",
                "mets la dernière vidéo de [unk]", "regarde [unk]", "écoute [unk]",
            ],
        ),
    };
    let mut phrases: Vec<String> = extra.iter().map(|p| p.to_string()).collect();
    for verb in verbs {
        for platform in platforms {
            phrases.push(format!("{verb} [unk] {link} {platform}"));
        }
    }
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::InstalledApp;
    use crate::intent::tokenize;

    fn ask(text: &str, apps: &AppCatalog) -> Option<PlatformRequest> {
        detect(&tokenize(text), apps, Lang::Fr)
    }

    fn url(text: &str) -> Option<String> {
        ask(text, &AppCatalog::default()).map(|r| r.url)
    }

    fn with_spotify() -> AppCatalog {
        AppCatalog::new(InstalledApp::new("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify").into_iter().collect())
    }

    #[test]
    fn youtube() {
        assert_eq!(url("mets squeezie sur youtube").as_deref(), Some("https://www.youtube.com/results?search_query=squeezie"));
        assert_eq!(
            url("recherche la dernière vidéo de inoxtag sur youtube").as_deref(),
            Some("https://www.youtube.com/results?search_query=inoxtag&sp=CAI%253D")
        );
        // No platform said, but it's about a video.
        assert_eq!(
            url("cherche la dernière vidéo d'inoxtag").as_deref(),
            Some("https://www.youtube.com/results?search_query=inoxtag&sp=CAI%253D")
        );
        assert_eq!(url("ouvre youtube et cherche lofi hip hop").as_deref(), Some("https://www.youtube.com/results?search_query=lofi+hip+hop"));
        assert_eq!(url("sur youtube les vidéos de squeezie").as_deref(), Some("https://www.youtube.com/results?search_query=squeezie"));
        assert_eq!(url("watch mrbeast on youtube").as_deref(), Some("https://www.youtube.com/results?search_query=mrbeast"));
    }

    #[test]
    fn twitch_channels_and_searches() {
        let gotaga = ask("gotaga sur twitch", &AppCatalog::default()).unwrap();
        assert!(gotaga.channel);
        assert_eq!(gotaga.url, "https://www.twitch.tv/gotaga");
        assert_eq!(url("mets le live de gotaga sur twitch").as_deref(), Some("https://www.twitch.tv/gotaga"));
        assert_eq!(url("cherche minecraft speedrun sur twitch").as_deref(), Some("https://www.twitch.tv/search?term=minecraft%20speedrun"));
    }

    #[test]
    fn spotify_uses_the_app_when_installed() {
        assert_eq!(url("damso feu de bois sur spotify").as_deref(), Some("https://open.spotify.com/search/damso%20feu%20de%20bois"));
        let apps = with_spotify();
        assert_eq!(ask("ouvre spotify et affiche damso", &apps).unwrap().url, "spotify:search:damso");
        assert_eq!(ask("mets du damso sur spotify", &apps).unwrap().url, "spotify:search:damso");
        assert_eq!(ask("écoute ninho", &apps).unwrap().url, "spotify:search:ninho");
    }

    #[test]
    fn other_platforms() {
        assert_eq!(url("cherche une pizzeria sur google maps").as_deref(), Some("https://www.google.com/maps/search/pizzeria"));
        assert_eq!(url("cherche un casque gamer sur amazon").as_deref(), Some("https://www.amazon.fr/s?k=casque+gamer"));
        assert_eq!(url("stranger things sur netflix").as_deref(), Some("https://www.netflix.com/search?q=stranger%20things"));
    }

    #[test]
    fn plain_opening_is_left_alone() {
        for text in ["ouvre youtube", "va sur youtube", "ouvre spotify", "ouvre youtube et twitch", "lance le site de twitch", "cherche des recettes"] {
            assert_eq!(url(text), None, "{text}");
        }
    }
}
