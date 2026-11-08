//! yt-dlp : validation des URL et vérification de mise à jour.
//!
//! yt-dlp n'est jamais figé (leçon 2 du brief) : la version installée est
//! comparée à la version épinglée du manifeste ; « Mettre à jour » réinstalle
//! la release épinglée dans `<données app>/bin/yt-dlp/`, qui passe avant le PATH.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::sidecar::{self, Binary, Resolver, Source};

/// URL acceptée : http(s) uniquement, hôte présent, sans espace ni caractère
/// de contrôle, et ne commençant pas par `-` (jamais prise pour une option).
pub fn validate_url(raw: &str) -> Result<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    let Some(rest) = rest else {
        bail!("adresse invalide : seules les URL http:// ou https:// sont acceptées");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.starts_with('.') || host.ends_with('@') {
        bail!("adresse invalide : nom de site manquant");
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("adresse invalide : espaces ou caractères de contrôle interdits");
    }
    Ok(url.to_string())
}

/// « 2024.04.09 » → [2024, 4, 9] (les suffixes non numériques sont ignorés).
fn version_key(v: &str) -> Vec<u64> {
    v.trim()
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// État de mise à jour affiché dans l'UI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub installed: Option<String>,
    pub source: Option<Source>,
    pub pinned: Option<String>,
    pub update_available: bool,
    pub message: String,
}

/// Compare la version installée à la version épinglée.
pub fn compare(
    installed: Option<&str>,
    source: Option<Source>,
    pinned: Option<&str>,
) -> UpdateStatus {
    let newer = match (installed, pinned) {
        (Some(i), Some(p)) => version_key(p) > version_key(i),
        (None, Some(_)) => true,
        _ => false,
    };
    let message = match (installed, pinned, newer) {
        (_, None, _) => "Aucune version téléchargeable pour cette plateforme.".to_string(),
        (None, Some(p), _) => format!("yt-dlp n'est pas installé : version {p} disponible."),
        (Some(i), Some(p), true) => format!(
            "Mise à jour disponible : {i} → {p}. Les sites changent souvent : une version ancienne échoue vite."
        ),
        (Some(i), Some(_), false) => format!("yt-dlp {i} est à jour."),
    };
    UpdateStatus {
        installed: installed.map(String::from),
        source,
        pinned: pinned.map(String::from),
        update_available: newer,
        message,
    }
}

/// `check_ytdlp_update` : `yt-dlp --version` comparé au manifeste.
pub fn check_update(resolver: &Resolver) -> UpdateStatus {
    let resolved = resolver.resolve(Binary::YtDlp);
    let installed = resolved
        .as_ref()
        .and_then(|r| sidecar::version(Binary::YtDlp, &r.path));
    let pinned = crate::download::offer(Binary::YtDlp)
        .ok()
        .map(|o| o.version);
    compare(
        installed.as_deref(),
        resolved.map(|r| r.source),
        pinned.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert!(validate_url(" https://www.youtube.com/watch?v=abc ").is_ok());
        assert!(validate_url("http://exemple.fr/v").is_ok());
        for bad in [
            "ftp://x.fr/a",
            "file:///etc/passwd",
            "--exec=rm",
            "https://",
            "https://a.fr/x y",
            "javascript:alert(1)",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn comparaison_des_versions() {
        let s = compare(
            Some("2024.04.09"),
            Some(Source::SystemPath),
            Some("2026.10.26"),
        );
        assert!(s.update_available && s.message.contains("2024.04.09 → 2026.10.26"));
        assert!(!compare(Some("2026.10.26"), None, Some("2026.10.26")).update_available);
        assert!(!compare(Some("2026.11.01"), None, Some("2026.10.26")).update_available);
        assert!(compare(None, None, Some("2026.10.26")).update_available);
    }

    /// Vrai yt-dlp de la VM (2024.04.09) : `--version` lu, mise à jour proposée.
    #[test]
    fn verification_avec_le_vrai_yt_dlp() {
        let Some(r) = crate::tools::testutil::real(&[Binary::YtDlp]) else {
            return;
        };
        let s = check_update(&r);
        let installed = s.installed.clone().expect("version lue");
        assert!(installed.starts_with("20"), "{installed}");
        let pinned = s.pinned.clone().unwrap();
        assert_eq!(
            s.update_available,
            version_key(&pinned) > version_key(&installed),
            "{s:?}"
        );
    }
}
