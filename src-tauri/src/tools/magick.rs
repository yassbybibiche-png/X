//! ImageMagick, v7 (`magick`) ou v6 (`convert`, Ubuntu/Debian) : même syntaxe
//! `<exe> entrée … sortie` pour les deux ; `montage` = `magick montage` (v7)
//! ou le binaire voisin `montage` (v6).
//!
//! Sécurité / robustesse :
//! - l'entrée est passée en chemin absolu suivi de `[0]` (première image) ;
//! - ImageMagick écrit dans un dossier temporaire privé (`sortie.<ext>`, nom
//!   sans caractère spécial), copiée ensuite dans la sortie réservée : aucun
//!   motif `%d` ni préfixe `format:` d'un nom d'utilisateur n'est interprété ;
//! - les textes (`-annotate`) sont échappés : `%` → `%%`, `@` initial → `\@`
//!   (sinon `@fichier` ferait lire un fichier arbitraire).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::jobs::ToolContext;
use crate::output::OutputFile;
use crate::sidecar::{self, Binary};

pub struct Magick {
    exe: PathBuf,
    v7: bool,
}

/// Mode d'un format (`-list format`) : lecture, écriture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Support {
    pub read: bool,
    pub write: bool,
}

impl Magick {
    pub fn new(ctx: &ToolContext) -> Result<Self> {
        Self::from_path(ctx.binary(Binary::ImageMagick)?)
    }

    pub fn from_path(exe: PathBuf) -> Result<Self> {
        let stem = exe
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        Ok(Self {
            v7: stem == "magick",
            exe,
        })
    }

    /// Programme + premiers arguments pour un sous-outil (`montage`…).
    fn tool(&self, name: &str) -> Result<(PathBuf, Vec<OsString>)> {
        if self.v7 {
            return Ok((self.exe.clone(), vec![name.into()]));
        }
        let exe = sidecar::sibling(&self.exe, name)
            .with_context(|| format!("« {name} » (ImageMagick 6) introuvable à côté de convert"))?;
        Ok((exe, Vec::new()))
    }

    /// Support d'un format par cette build (`AVIF`, `HEIC`, `WEBP`…).
    pub fn support(&self, format: &str) -> Result<Support> {
        let run = sidecar::run(&self.exe, &["-list", "format"], Duration::from_secs(30))?
            .check("ImageMagick")?;
        Ok(parse_support(&run.stdout, format))
    }

    /// Lance `<exe> args… <tmp>/sortie.<ext>` puis copie le résultat dans `out`.
    pub fn convert_into(
        &self,
        ctx: &ToolContext,
        args: Vec<OsString>,
        out: &mut OutputFile,
        ext: &str,
    ) -> Result<()> {
        self.run_into(ctx, (self.exe.clone(), Vec::new()), args, out, ext)
    }

    /// Idem avec `montage`.
    pub fn montage_into(
        &self,
        ctx: &ToolContext,
        args: Vec<OsString>,
        out: &mut OutputFile,
        ext: &str,
    ) -> Result<()> {
        self.run_into(ctx, self.tool("montage")?, args, out, ext)
    }

    fn run_into(
        &self,
        ctx: &ToolContext,
        (program, mut all): (PathBuf, Vec<OsString>),
        args: Vec<OsString>,
        out: &mut OutputFile,
        ext: &str,
    ) -> Result<()> {
        let tmp = tempfile::tempdir().context("dossier temporaire impossible")?;
        let result = tmp.path().join(format!("sortie.{ext}"));
        all.extend(args);
        all.push(result.clone().into());
        ctx.run_sidecar(&program, "ImageMagick", &all, None)?;
        if !result.is_file() {
            bail!("ImageMagick n'a produit aucun fichier");
        }
        let mut w = std::io::BufWriter::new(out.take_file()?);
        std::io::copy(&mut std::fs::File::open(&result)?, &mut w)?;
        std::io::Write::flush(&mut w)?;
        Ok(())
    }
}

/// Lignes « `   AVIF  rw+   AV1 Image File Format` » de `-list format`.
pub fn parse_support(list: &str, format: &str) -> Support {
    let mode = list.lines().find_map(|line| {
        // « Format [Module] Mode Description » : le module n'est affiché que s'il diffère.
        let mut it = line.split_whitespace();
        let name = it.next()?.trim_end_matches('*');
        let mode = it.take(2).find(|t| is_mode(t))?;
        name.eq_ignore_ascii_case(format).then(|| mode.to_string())
    });
    match mode {
        Some(m) => Support {
            read: m.starts_with('r'),
            write: m.as_bytes()[1] == b'w',
        },
        None => Support {
            read: false,
            write: false,
        },
    }
}

/// « rw+ », « r-- », « -w- »…
fn is_mode(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() == 3
        && matches!(b[0], b'r' | b'-')
        && matches!(b[1], b'w' | b'-')
        && matches!(b[2], b'+' | b'-')
}

/// Entrée : chemin absolu + `[0]` (première image d'un fichier multi-images).
pub fn input(path: &Path) -> Result<OsString> {
    let mut arg = std::path::absolute(path)?.into_os_string();
    arg.push("[0]");
    Ok(arg)
}

/// Texte littéral pour `-annotate` / `label:` (pas d'échappement `%`, pas de `@fichier`).
pub fn literal_text(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('%', "%%");
    match escaped.strip_prefix('@') {
        Some(rest) => format!("\\@{rest}"),
        None => escaped,
    }
}

/// Couleur `#rrggbb` validée (jamais une chaîne libre passée à ImageMagick).
pub fn hex_color(value: &str) -> Result<String> {
    let v = value.trim().trim_start_matches('#');
    if v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(format!("#{}", v.to_ascii_lowercase()));
    }
    bail!("couleur invalide : « {value} » (format #rrggbb, ex. #ffffff)")
}

pub fn strings(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "   Format  Mode  Description\n------\n     AVIF  r--   AV1 Image File Format (1.17.6)\n      ICO* ICON      rw+   Microsoft icon\n     WEBP* WEBP      rw+   WebP\n";

    #[test]
    fn formats_pris_en_charge() {
        assert_eq!(
            parse_support(LIST, "avif"),
            Support {
                read: true,
                write: false
            }
        );
        assert_eq!(
            parse_support(LIST, "WEBP"),
            Support {
                read: true,
                write: true
            }
        );
        assert_eq!(
            parse_support(LIST, "heic"),
            Support {
                read: false,
                write: false
            }
        );
    }

    #[test]
    fn v6_et_v7() {
        assert!(Magick::from_path("/usr/bin/magick".into()).unwrap().v7);
        assert!(!Magick::from_path("/usr/bin/convert".into()).unwrap().v7);
    }

    #[test]
    fn textes_et_couleurs_surs() {
        assert_eq!(literal_text("@/etc/passwd"), "\\@/etc/passwd");
        assert_eq!(literal_text("100 % sûr"), "100 %% sûr");
        assert_eq!(hex_color("FFAA00").unwrap(), "#ffaa00");
        assert!(hex_color("red; rm").is_err());
        assert!(
            input(Path::new("a.png"))
                .unwrap()
                .to_string_lossy()
                .ends_with("a.png[0]")
        );
    }
}
