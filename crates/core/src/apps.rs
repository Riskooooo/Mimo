//! The apps installed on this PC (as listed in the Start menu), and matching
//! a spoken/typed name against them. Discovering the list is the shell's
//! job; this module only decides which entry a request means.

use crate::intent::tokenize;

/// Start menu entries that aren't apps anyone would ask to open.
const NOISE: &[&str] = &[
    "uninstall", "desinstaller", "readme", "lisezmoi", "documentation", "help", "aide",
    "manual", "manuel", "website", "web site", "release notes", "faq", "samples", "legal",
    "registration", "more",
];

/// Longest name (in words) offered to voice recognition; longer ones are
/// rarely said in full and only bloat the grammar.
const MAX_SPOKEN_WORDS: usize = 4;

/// Shortest request that may match an app by part of its name ("code"
/// for "Visual Studio Code"), so a stray "x" doesn't open "XBOX".
const MIN_PARTIAL_MATCH_LEN: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// Display name, e.g. "Explorateur de fichiers".
    pub name: String,
    /// Windows AppUserModelID / Start menu app id, launchable through
    /// `shell:AppsFolder\<id>`. Comes from the OS, never from user text.
    pub id: String,
    /// Name as parser tokens ("explorateur de fichiers").
    key: Vec<String>,
}

impl InstalledApp {
    /// `None` for entries that aren't real apps (uninstallers, help files,
    /// web links…).
    pub fn new(name: &str, id: &str) -> Option<Self> {
        let key = tokenize(name);
        let joined = key.join(" ");
        let is_noise = NOISE.iter().any(|noise| {
            let noise: Vec<&str> = noise.split(' ').collect();
            key.windows(noise.len()).any(|w| w.iter().zip(&noise).all(|(a, b)| a == b))
        });
        if key.is_empty() || is_noise || id.contains("://") || joined.len() < 2 {
            return None;
        }
        Some(Self {
            name: name.trim().to_string(),
            id: id.trim().to_string(),
            key,
        })
    }

    /// The name as voice recognition should expect it: lowercase, accents
    /// kept (the speech models spell words with them), no punctuation or
    /// version numbers. `None` if it's too long to be worth listening for.
    pub fn spoken_name(&self) -> Option<String> {
        let lowered = self.name.to_lowercase();
        let words: Vec<&str> = lowered
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
            .collect();
        (!words.is_empty() && words.len() <= MAX_SPOKEN_WORDS).then(|| words.join(" "))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppCatalog {
    apps: Vec<InstalledApp>,
}

impl AppCatalog {
    pub fn new(apps: Vec<InstalledApp>) -> Self {
        Self { apps }
    }

    pub fn apps(&self) -> &[InstalledApp] {
        &self.apps
    }

    /// An app whose whole name is exactly `target` (already tokenized).
    pub fn exact(&self, target: &[String]) -> Option<&InstalledApp> {
        self.apps.iter().find(|app| app.key == target)
    }

    /// Best app whose name contains every word of `target`, preferring
    /// names that start with it, then the shortest (most specific) name.
    pub fn partial(&self, target: &[String]) -> Option<&InstalledApp> {
        if target.join(" ").len() < MIN_PARTIAL_MATCH_LEN {
            return None;
        }
        self.apps
            .iter()
            .filter(|app| target.iter().all(|word| app.key.contains(word)))
            .min_by_key(|app| (!app.key.starts_with(target), app.key.len(), app.name.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> AppCatalog {
        AppCatalog::new(
            [
                ("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
                ("Explorateur de fichiers", "Microsoft.Windows.Explorer"),
                ("Google Chrome", "Chrome"),
                ("Visual Studio 2022", "VisualStudio.17"),
                ("Visual Studio Code", "Microsoft.VisualStudioCode"),
                ("Visual Studio Installer", "VSInstaller"),
                ("Uninstall Node.js", "NodeUninstall"),
                ("Steam Support Center", "http://support.steampowered.com/"),
                ("XBOX", "Microsoft.GamingApp"),
            ]
            .into_iter()
            .filter_map(|(name, id)| InstalledApp::new(name, id))
            .collect(),
        )
    }

    fn words(s: &str) -> Vec<String> {
        tokenize(s)
    }

    #[test]
    fn noise_entries_are_dropped() {
        let names: Vec<_> = catalog().apps().iter().map(|a| a.name.clone()).collect();
        assert!(!names.contains(&"Uninstall Node.js".to_string()));
        assert!(!names.contains(&"Steam Support Center".to_string()));
        assert!(names.contains(&"Spotify".to_string()));
    }

    #[test]
    fn exact_match_ignores_case_and_accents() {
        let catalog = catalog();
        assert_eq!(catalog.exact(&words("spotify")).map(|a| a.id.as_str()), Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"));
        assert_eq!(catalog.exact(&words("explorateur de fichiers")).map(|a| a.name.as_str()), Some("Explorateur de fichiers"));
    }

    #[test]
    fn partial_match_prefers_most_specific_name() {
        let catalog = catalog();
        assert_eq!(catalog.partial(&words("chrome")).map(|a| a.name.as_str()), Some("Google Chrome"));
        assert_eq!(catalog.partial(&words("visual studio code")).map(|a| a.name.as_str()), Some("Visual Studio Code"));
        assert_eq!(catalog.partial(&words("code")).map(|a| a.name.as_str()), Some("Visual Studio Code"));
        assert_eq!(catalog.partial(&words("x")), None);
    }

    #[test]
    fn spoken_names_keep_accents_and_drop_versions() {
        let app = InstalledApp::new("Visual Studio 2022", "id").unwrap();
        assert_eq!(app.spoken_name().as_deref(), Some("visual studio"));
        let app = InstalledApp::new("Paramètres", "id").unwrap();
        assert_eq!(app.spoken_name().as_deref(), Some("paramètres"));
        let app = InstalledApp::new("Pare-feu Windows Defender avec fonctions avancées de sécurité", "id").unwrap();
        assert_eq!(app.spoken_name(), None);
    }
}
