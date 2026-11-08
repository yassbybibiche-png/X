//! Appels Ghostscript communs (toujours `-dSAFER`, jamais via un shell).
//!
//! Pièges gérés ici : `-sOutputFile` interprète `%d` (les `%` du chemin sont
//! doublés) et un chemin d'entrée commençant par `-` serait pris pour une option
//! (les chemins sont rendus absolus).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};

use crate::jobs::ToolContext;
use crate::output;
use crate::sidecar::{Binary, Stream};

/// Arguments de base : mode batch, sûr, périphérique `device`.
pub fn base_args(device: &str) -> Vec<OsString> {
    [
        "-dSAFER",
        "-dBATCH",
        "-dNOPAUSE",
        &format!("-sDEVICE={device}"),
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

/// `-sOutputFile=<chemin>` avec les `%` échappés (`%%`), sauf le motif final
/// `pattern` (ex. `%d`) ajouté tel quel.
pub fn output_arg(path: &Path, pattern: &str) -> OsString {
    let escaped = path.to_string_lossy().replace('%', "%%");
    OsString::from(format!("-sOutputFile={escaped}{pattern}"))
}

/// Chemin d'entrée absolu (ne peut pas commencer par `-`).
pub fn input_arg(path: &Path) -> Result<OsString> {
    let abs = std::path::absolute(path)
        .with_context(|| format!("chemin invalide : {}", path.display()))?;
    Ok(abs.into_os_string())
}

/// « Processing pages 1 through 12. » → 12.
pub fn parse_total(line: &str) -> Option<u32> {
    let rest = line.trim().strip_prefix("Processing pages ")?;
    rest.split_whitespace()
        .nth(2)?
        .trim_end_matches('.')
        .parse()
        .ok()
}

/// « Page 3 » → 3.
pub fn parse_page(line: &str) -> Option<u32> {
    line.trim().strip_prefix("Page ")?.trim().parse().ok()
}

/// Lance Ghostscript avec progression par page ; les erreurs récupérées
/// (`****`) sont relayées en avertissements.
pub fn run(ctx: &ToolContext, args: &[OsString], name: &str) -> Result<()> {
    let gs = ctx.binary(Binary::Ghostscript)?;
    let total = AtomicU32::new(0);
    let reporter = ctx.reporter;
    let on_line = |_: Stream, line: &str| {
        if let Some(t) = parse_total(line) {
            total.store(t, Ordering::Relaxed);
        } else if let Some(p) = parse_page(line) {
            let t = total.load(Ordering::Relaxed).max(p);
            let pct = super::percent(p.saturating_sub(1) as usize, t as usize);
            reporter.progress(pct, &format!("{name} : page {p}/{t}…"));
        } else if line.trim_start().starts_with("****") {
            reporter.warn(&format!("Ghostscript : {}", line.trim()));
        }
    };
    ctx.run_sidecar(&gs, "Ghostscript", args, Some(&on_line))?;
    Ok(())
}

/// Réécrit un PDF avec `pdfwrite` et des options supplémentaires.
pub fn pdfwrite(ctx: &ToolContext, input: &Path, out: &Path, extra: &[&str]) -> Result<()> {
    let mut args = base_args("pdfwrite");
    args.extend(extra.iter().map(OsString::from));
    args.push(output_arg(out, ""));
    args.push(input_arg(input)?);
    run(ctx, &args, &output::file_name(input))
}

/// Rastérise toutes les pages dans `dir` (`p1.<ext>`, `p2.<ext>`…) ; renvoie
/// les fichiers dans l'ordre des pages.
pub fn rasterize(
    ctx: &ToolContext,
    input: &Path,
    dir: &Path,
    (device, ext, dpi): (&str, &str, u32),
) -> Result<Vec<PathBuf>> {
    let mut args = base_args(device);
    args.push(format!("-r{dpi}").into());
    args.extend(["-dTextAlphaBits=4", "-dGraphicsAlphaBits=4"].map(OsString::from));
    if ext == "jpg" {
        args.push("-dJPEGQ=90".into());
    }
    args.push(output_arg(&dir.join("p"), &format!("%d.{ext}")));
    args.push(input_arg(input)?);
    run(ctx, &args, &output::file_name(input))?;
    let pages = numbered_files(dir, ext);
    if pages.is_empty() {
        anyhow::bail!("Ghostscript n'a produit aucune page");
    }
    Ok(pages)
}

/// `p1.ext`, `p2.ext`… présents dans `dir`, dans l'ordre.
fn numbered_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    (1..)
        .map(|n| dir.join(format!("p{n}.{ext}")))
        .take_while(|p| p.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse_des_lignes() {
        assert_eq!(parse_total("Processing pages 1 through 12."), Some(12));
        assert_eq!(parse_page("Page 3"), Some(3));
        assert_eq!(parse_page("Pages"), None);
    }

    #[test]
    fn pourcent_echappe_dans_la_sortie() {
        let arg = output_arg(Path::new("dossier 100%/p"), "%d.png");
        assert_eq!(arg, OsString::from("-sOutputFile=dossier 100%%/p%d.png"));
    }

    #[test]
    fn entree_jamais_prise_pour_une_option() {
        let arg = input_arg(Path::new("-dangereux.pdf")).unwrap();
        assert!(!arg.to_string_lossy().starts_with('-'));
        assert!(base_args("pdfwrite").contains(&OsString::from("-dSAFER")));
    }
}
