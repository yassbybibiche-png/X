//! Protection / déprotection de PDF par mot de passe avec qpdf (AES-256).
//!
//! Le mot de passe n'apparaît **jamais** sur la ligne de commande (visible dans
//! la liste des processus) : tous les arguments sont passés à qpdf dans un
//! fichier temporaire privé (`qpdf @fichier`, un argument par ligne).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::jobs::ToolContext;
use crate::output;
use crate::sidecar::{self, Binary, SidecarError};

/// Code de sortie qpdf « succès avec avertissements ».
const QPDF_WARNINGS: i32 = 3;

fn password(ctx: &ToolContext) -> Result<String> {
    let pw = ctx.input.option("mot_de_passe").to_string();
    if pw.is_empty() {
        bail!("indiquez un mot de passe");
    }
    if pw.contains(['\n', '\r']) {
        bail!("le mot de passe ne peut pas contenir de retour à la ligne");
    }
    Ok(pw)
}

fn path_line(path: &Path) -> Result<String> {
    let abs = std::path::absolute(path)?;
    let s = abs
        .to_str()
        .context("chemin non UTF-8 non pris en charge par qpdf")?;
    if s.contains(['\n', '\r']) {
        bail!("chemin invalide (retour à la ligne)");
    }
    Ok(s.to_string())
}

/// Arguments de chiffrement AES-256 (mot de passe d'ouverture = propriétaire).
pub fn encrypt_lines(input: &Path, out: &Path, pw: &str) -> Result<Vec<String>> {
    Ok(vec![
        path_line(input)?,
        "--encrypt".into(),
        pw.into(),
        pw.into(),
        "256".into(),
        "--".into(),
        path_line(out)?,
    ])
}

pub fn decrypt_lines(input: &Path, out: &Path, pw: &str) -> Result<Vec<String>> {
    Ok(vec![
        format!("--password={pw}"),
        "--decrypt".into(),
        path_line(input)?,
        path_line(out)?,
    ])
}

/// Lance `qpdf @<fichier d'arguments>`.
fn run_qpdf(ctx: &ToolContext, lines: &[String]) -> Result<()> {
    let qpdf = ctx.binary(Binary::Qpdf)?;
    let mut args_file = tempfile::NamedTempFile::new().context("fichier temporaire impossible")?;
    args_file.write_all(lines.join("\n").as_bytes())?;
    args_file.flush()?;
    let mut arg = std::ffi::OsString::from("@");
    arg.push(args_file.path());
    let run = sidecar::run_with(&qpdf, &[arg], ctx.run_options(None))?;
    match run.status.code() {
        Some(0) => Ok(()),
        Some(QPDF_WARNINGS) => {
            ctx.reporter
                .warn(&format!("qpdf : {}", sidecar::truncate_stderr(&run.stderr)));
            Ok(())
        }
        _ if run.stderr.contains("invalid password") => bail!("mot de passe incorrect"),
        _ => Err(SidecarError::Failed {
            label: "qpdf".into(),
            status: run.status.to_string(),
            stderr: sidecar::truncate_stderr(&run.stderr),
        }
        .into()),
    }
}

/// `qpdf --is-encrypted` : code 0 = chiffré, 2 = non chiffré.
fn is_encrypted(ctx: &ToolContext, path: &Path) -> Result<bool> {
    let qpdf = ctx.binary(Binary::Qpdf)?;
    let args = [
        "--is-encrypted".into(),
        std::path::absolute(path)?.into_os_string(),
    ];
    let run = sidecar::run_with(&qpdf, &args, ctx.run_options(None))?;
    match run.status.code() {
        Some(0) => Ok(true),
        Some(2) => Ok(false),
        _ => bail!("PDF illisible : {}", sidecar::truncate_stderr(&run.stderr)),
    }
}

/// `pdf-proteger`.
pub fn protect(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let pw = password(ctx)?;
    let input = &ctx.input.inputs[0];
    if is_encrypted(ctx, input)? {
        bail!("ce PDF est déjà protégé : retirez d'abord sa protection");
    }
    let out = output::beside(input, Some("protege"), "pdf")?;
    ctx.reporter.progress(10.0, "Chiffrement AES-256…");
    run_qpdf(ctx, &encrypt_lines(input, out.path(), &pw)?)?;
    ctx.reporter.progress(100.0, "PDF protégé");
    Ok(vec![out.commit()])
}

/// `pdf-deproteger`.
pub fn unprotect(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let pw = password(ctx)?;
    let input = &ctx.input.inputs[0];
    if !is_encrypted(ctx, input)? {
        bail!("ce PDF n'est pas protégé par un mot de passe");
    }
    let out = output::beside(input, Some("deprotege"), "pdf")?;
    ctx.reporter.progress(10.0, "Déchiffrement…");
    run_qpdf(ctx, &decrypt_lines(input, out.path(), &pw)?)?;
    ctx.reporter.progress(100.0, "Protection retirée");
    Ok(vec![out.commit()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::tools::pdf::tests::make_pdf;
    use crate::tools::testutil::real;

    #[test]
    fn mot_de_passe_jamais_dans_les_arguments_du_processus() {
        let lines = encrypt_lines(Path::new("a.pdf"), Path::new("b.pdf"), "s3cret").unwrap();
        assert_eq!(lines[1..6], ["--encrypt", "s3cret", "s3cret", "256", "--"]);
        // Seul `@fichier` est passé à qpdf : voir run_qpdf.
        let d = decrypt_lines(Path::new("a.pdf"), Path::new("b.pdf"), "x").unwrap();
        assert_eq!(d[0], "--password=x");
    }

    #[test]
    fn proteger_puis_deproteger_avec_qpdf() {
        let Some(r) = real(&[Binary::Qpdf]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("doc.pdf");
        make_pdf(&src, "P", 2);
        let run = |id: &str, file: &Path, pw: &str| {
            let job = JobInput::new(vec![file.to_path_buf()]).with("mot_de_passe", pw);
            execute(id, job, &MemoryReporter::default(), &r)
        };
        let prot = run("pdf-proteger", &src, "s3cret").unwrap();
        assert!(prot[0].ends_with("doc-protege.pdf"));
        let raw = std::fs::read(&prot[0]).unwrap();
        assert!(raw.windows(8).any(|w| w == b"/Encrypt"));
        assert!(
            run("pdf-proteger", &prot[0], "x").is_err(),
            "double protection"
        );

        let err = run("pdf-deproteger", &prot[0], "faux").unwrap_err();
        assert!(
            err.to_string().contains("mot de passe incorrect"),
            "{err:#}"
        );
        assert!(!tmp.path().join("doc-protege-deprotege.pdf").exists());
        let clear = run("pdf-deproteger", &prot[0], "s3cret").unwrap();
        let doc = lopdf::Document::load(&clear[0]).unwrap();
        assert!(!doc.is_encrypted());
        assert!(doc.extract_text(&[2]).unwrap().contains("P-2"));
        assert!(run("pdf-deproteger", &src, "x").is_err(), "PDF non protégé");
        assert!(run("pdf-proteger", &src, "").is_err(), "mot de passe vide");
    }
}
