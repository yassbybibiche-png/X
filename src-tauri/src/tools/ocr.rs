//! Tesseract (OCR français + anglais) : choix des langues, dossier `tessdata`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};

use crate::jobs::ToolContext;
use crate::sidecar::{self, Binary};

/// Tesseract résolu, avec son dossier `tessdata` éventuel (installation de
/// l'application : `<dossier de tesseract>/tessdata`).
pub struct Tesseract {
    pub exe: PathBuf,
    tessdata: Option<PathBuf>,
}

impl Tesseract {
    pub fn new(ctx: &ToolContext) -> Result<Self> {
        let exe = ctx.binary(Binary::Tesseract)?;
        let tessdata = exe
            .parent()
            .map(|d| d.join("tessdata"))
            .filter(|d| d.is_dir());
        Ok(Self { exe, tessdata })
    }

    /// Arguments communs (avant les arguments propres à l'appel).
    pub fn args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if let Some(dir) = &self.tessdata {
            args.push("--tessdata-dir".into());
            args.push(dir.into());
        }
        args
    }

    /// Langues installées (`tesseract --list-langs`).
    pub fn languages(&self) -> Result<Vec<String>> {
        let mut args = self.args();
        args.push("--list-langs".into());
        let out = sidecar::run(&self.exe, &args, Duration::from_secs(30))?.check("Tesseract")?;
        Ok(parse_languages(&out.stdout))
    }
}

/// Lignes après « List of available languages… ».
pub fn parse_languages(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .skip_while(|l| !l.starts_with("List of available"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Langues demandées (`fra+eng`, `fra`, `eng`) restreintes à celles installées.
/// Erreur claire si aucune n'est disponible ; avertissement si une manque.
pub fn pick_languages(ctx: &ToolContext, wanted: &str, installed: &[String]) -> Result<String> {
    let wanted: Vec<&str> = match wanted.trim() {
        "" => vec!["fra", "eng"],
        w => w.split('+').map(str::trim).collect(),
    };
    let (ok, missing): (Vec<&str>, Vec<&str>) = wanted
        .iter()
        .partition(|l| installed.iter().any(|i| i == *l));
    if ok.is_empty() {
        bail!(
            "langue(s) OCR non installée(s) : {} (installées : {}). \
             Ajoutez les fichiers <langue>.traineddata dans le dossier tessdata.",
            missing.join(", "),
            installed.join(", ")
        );
    }
    if !missing.is_empty() {
        ctx.reporter.warn(&format!(
            "Langue(s) OCR absente(s), ignorée(s) : {}",
            missing.join(", ")
        ));
    }
    Ok(ok.join("+"))
}

/// Tesseract prêt à l'emploi + langues retenues.
pub fn prepare(ctx: &ToolContext) -> Result<(Tesseract, String)> {
    let tess = Tesseract::new(ctx)?;
    let langs = pick_languages(ctx, ctx.input.option("langue"), &tess.languages()?)?;
    ctx.reporter.log(&format!("OCR : langue(s) {langs}"));
    Ok((tess, langs))
}

/// `tesseract <image> <base> -l <langs> [config]` ; Tesseract ajoute
/// l'extension (`.pdf`, `.txt`) à `base`, ou écrit sur stdout si `base` = `stdout`.
pub fn recognize(
    ctx: &ToolContext,
    (tess, langs): (&Tesseract, &str),
    image: &Path,
    base: &Path,
    config: Option<&str>,
) -> Result<String> {
    let mut args = tess.args();
    args.push(image.into());
    args.push(base.into());
    args.extend(["-l", langs].map(OsString::from));
    if let Some(c) = config {
        args.push(c.into());
    }
    Ok(ctx.run_sidecar(&tess.exe, "Tesseract", &args, None)?.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::CancelToken;
    use crate::jobs::{JobInput, MemoryReporter};
    use crate::sidecar::Resolver;

    #[test]
    fn langues_installees() {
        let out = "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nfra\nosd\n";
        assert_eq!(parse_languages(out), ["eng", "fra", "osd"]);
    }

    #[test]
    fn choix_des_langues() {
        let input = JobInput::default();
        let reporter = MemoryReporter::default();
        let ctx = ToolContext {
            input: &input,
            reporter: &reporter,
            resolver: &Resolver::default(),
            cancel: &CancelToken::default(),
        };
        let both = ["eng".to_string(), "fra".to_string()];
        assert_eq!(pick_languages(&ctx, "", &both).unwrap(), "fra+eng");
        let only_eng = ["eng".to_string()];
        assert_eq!(pick_languages(&ctx, "fra+eng", &only_eng).unwrap(), "eng");
        assert_eq!(reporter.count(crate::jobs::EventKind::Warning), 1);
        let err = pick_languages(&ctx, "fra", &only_eng).unwrap_err();
        assert!(err.to_string().contains("non installée"), "{err}");
    }
}
