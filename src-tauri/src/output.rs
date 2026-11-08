//! Noms de sortie : à côté de l'original, suffixe, jamais d'écrasement.
//!
//! La réservation utilise `create_new` (atomique) : un fichier existant n'est
//! jamais remplacé, même en cas de course entre deux tâches. Si l'outil échoue,
//! le fichier réservé est supprimé à la destruction de [`OutputFile`].

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const MAX_ATTEMPTS: u32 = 10_000;

/// Fichier de sortie réservé (créé vide).
#[derive(Debug)]
pub struct OutputFile {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl OutputFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Prend le descripteur ouvert en écriture (une seule fois).
    pub fn take_file(&mut self) -> Result<File> {
        self.file.take().context("fichier de sortie déjà utilisé")
    }

    /// Conserve le fichier (succès) et renvoie son chemin.
    pub fn commit(mut self) -> PathBuf {
        self.committed = true;
        self.path.clone()
    }
}

impl Drop for OutputFile {
    fn drop(&mut self) {
        if !self.committed {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Nom candidat n° `n` : `base.ext`, puis `base-2.ext`, `base-3.ext`…
pub fn candidate(base: &str, ext: &str, n: u32) -> String {
    let name = if n <= 1 {
        base.to_string()
    } else {
        format!("{base}-{n}")
    };
    if ext.is_empty() {
        name
    } else {
        format!("{name}.{ext}")
    }
}

/// Base de nom avec suffixe facultatif : `rapport` + `compresse` → `rapport-compresse`.
pub fn with_suffix(base: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(s) if !s.is_empty() => format!("{base}-{s}"),
        _ => base.to_string(),
    }
}

/// Nom sans extension (ou nom complet d'un dossier).
pub fn stem(path: &Path) -> String {
    let os = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    os.map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sortie".to_string())
}

/// Nom complet du fichier (avec extension).
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sortie".to_string())
}

/// Dossier contenant `input` (le dossier courant si aucun parent).
pub fn parent_dir(input: &Path) -> PathBuf {
    match input.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Réserve le premier nom libre `base[-n].ext` dans `dir`.
pub fn reserve(dir: &Path, base: &str, ext: &str) -> Result<OutputFile> {
    if base.contains(['/', '\\']) || base == ".." {
        bail!("nom de sortie invalide : {base}");
    }
    for n in 1..=MAX_ATTEMPTS {
        let path = dir.join(candidate(base, ext, n));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return Ok(OutputFile {
                    path,
                    file: Some(file),
                    committed: false,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("impossible de créer {}", path.display()));
            }
        }
    }
    bail!(
        "aucun nom de sortie libre pour {base}.{ext} dans {}",
        dir.display()
    )
}

/// Dossier de sortie réservé (créé vide par `create_dir`, atomique). En cas
/// d'échec, il est supprimé avec son contenu (uniquement ce que l'outil y a écrit).
#[derive(Debug)]
pub struct OutputDir {
    path: PathBuf,
    committed: bool,
}

impl OutputDir {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn commit(mut self) -> PathBuf {
        self.committed = true;
        self.path.clone()
    }
}

impl Drop for OutputDir {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Réserve le premier dossier libre `base[-n]` dans `dir` (jamais un dossier existant).
pub fn reserve_dir(dir: &Path, base: &str) -> Result<OutputDir> {
    if base.contains(['/', '\\']) || base == ".." || base.is_empty() {
        bail!("nom de dossier de sortie invalide : {base}");
    }
    for n in 1..=MAX_ATTEMPTS {
        let path = dir.join(candidate(base, "", n));
        match std::fs::create_dir(&path) {
            Ok(()) => {
                return Ok(OutputDir {
                    path,
                    committed: false,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("impossible de créer {}", path.display()));
            }
        }
    }
    bail!(
        "aucun nom de dossier libre pour {base} dans {}",
        dir.display()
    )
}

/// Remplit la réservation `out` avec le fichier `src` produit par un sidecar
/// dans un dossier de travail (renommage sur la même partition, sinon copie).
/// Seule notre propre réservation vide est remplacée.
pub fn fill_from(mut out: OutputFile, src: &Path) -> Result<PathBuf> {
    drop(out.take_file()?);
    if std::fs::rename(src, out.path()).is_err() {
        std::fs::copy(src, out.path())
            .with_context(|| format!("copie de {} impossible", src.display()))?;
    }
    Ok(out.commit())
}

/// Réserve une sortie à côté de `input` : `<stem>-<suffix>.<ext>`.
pub fn beside(input: &Path, suffix: Option<&str>, ext: &str) -> Result<OutputFile> {
    reserve(&parent_dir(input), &with_suffix(&stem(input), suffix), ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn noms_candidats() {
        assert_eq!(candidate("photo", "jpg", 1), "photo.jpg");
        assert_eq!(candidate("photo", "jpg", 2), "photo-2.jpg");
        assert_eq!(candidate("LISEZMOI", "", 3), "LISEZMOI-3");
        assert_eq!(with_suffix("a", Some("fusionne")), "a-fusionne");
        assert_eq!(with_suffix("a", None), "a");
    }

    #[test]
    fn sortie_a_cote_de_l_original() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("rapport.pdf");
        fs::write(&input, b"x").unwrap();
        let out = beside(&input, Some("compresse"), "pdf").unwrap();
        assert_eq!(out.path(), tmp.path().join("rapport-compresse.pdf"));
    }

    #[test]
    fn jamais_d_ecrasement() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("photo.png");
        fs::write(&input, b"original").unwrap();
        fs::write(tmp.path().join("photo.jpg"), b"existant").unwrap();
        fs::write(tmp.path().join("photo-2.jpg"), b"existant 2").unwrap();
        let out = beside(&input, None, "jpg").unwrap();
        assert_eq!(out.path(), tmp.path().join("photo-3.jpg"));
        out.commit();
        assert_eq!(fs::read(tmp.path().join("photo.jpg")).unwrap(), b"existant");
        assert_eq!(
            fs::read(tmp.path().join("photo-2.jpg")).unwrap(),
            b"existant 2"
        );
        assert_eq!(fs::read(&input).unwrap(), b"original");
    }

    #[test]
    fn meme_extension_ne_remplace_pas_l_entree() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("image.png");
        fs::write(&input, b"original").unwrap();
        let out = beside(&input, None, "png").unwrap();
        assert_eq!(out.path(), tmp.path().join("image-2.png"));
        assert_eq!(fs::read(&input).unwrap(), b"original");
    }

    #[test]
    fn reservations_successives_distinctes() {
        let tmp = tempfile::tempdir().unwrap();
        let a = reserve(tmp.path(), "archive", "zip").unwrap();
        let b = reserve(tmp.path(), "archive", "zip").unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn echec_supprime_la_reservation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = {
            let out = reserve(tmp.path(), "temp", "txt").unwrap();
            out.path().to_path_buf()
        };
        assert!(!path.exists());
    }

    #[test]
    fn nom_de_dossier_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Vacances.2024");
        fs::create_dir(&dir).unwrap();
        assert_eq!(stem(&dir), "Vacances.2024");
    }

    #[test]
    fn dossier_reserve_sans_ecrasement_et_nettoye() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("photos")).unwrap();
        fs::write(tmp.path().join("photos").join("garde.txt"), b"x").unwrap();
        let d = reserve_dir(tmp.path(), "photos").unwrap();
        assert_eq!(d.path(), tmp.path().join("photos-2"));
        fs::write(d.path().join("partiel"), b"y").unwrap();
        drop(d);
        assert!(!tmp.path().join("photos-2").exists());
        assert!(tmp.path().join("photos").join("garde.txt").exists());
        assert!(reserve_dir(tmp.path(), "..").is_err());
    }

    #[test]
    fn base_invalide_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(reserve(tmp.path(), "../evasion", "txt").is_err());
    }
}
