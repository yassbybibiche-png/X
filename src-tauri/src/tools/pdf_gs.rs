//! Outils PDF avec Ghostscript (et Tesseract pour l'OCR) : compresser, niveaux
//! de gris, réparer, PDF → images, OCR.
//!
//! Sortie toujours réservée (`create_new`) avant l'appel ; Ghostscript n'écrit
//! que dans cette réservation ou dans un dossier temporaire privé.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{gs, ocr, per_file, percent};
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};

/// Réglage `-dPDFSETTINGS` du niveau choisi.
pub fn pdf_settings(level: &str) -> Result<&'static str> {
    Ok(match level {
        "screen" => "-dPDFSETTINGS=/screen",
        "ebook" => "-dPDFSETTINGS=/ebook",
        "printer" => "-dPDFSETTINGS=/printer",
        other => bail!("niveau de compression inconnu : {other}"),
    })
}

/// `pdf-compresser` : réécriture `pdfwrite` (images rééchantillonnées).
pub fn compress(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let settings = pdf_settings(ctx.input.option("niveau"))?;
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let out = output::beside(path, Some("compresse"), "pdf")?;
        let extra = [
            settings,
            "-dCompatibilityLevel=1.5",
            "-dDetectDuplicateImages=true",
        ];
        gs::pdfwrite(ctx, path, out.path(), &extra)?;
        report_sizes(ctx, path, out.path())?;
        Ok(out.commit())
    })
}

fn report_sizes(ctx: &ToolContext, before: &Path, after: &Path) -> Result<()> {
    let (a, b) = (
        std::fs::metadata(before)?.len(),
        std::fs::metadata(after)?.len(),
    );
    if b >= a {
        ctx.reporter.warn(&format!(
            "{} : le résultat n'est pas plus léger ({a} → {b} octets) ; \
             le fichier est probablement déjà optimisé.",
            output::file_name(before)
        ));
    } else {
        let gain = 100.0 - b as f64 * 100.0 / a as f64;
        ctx.reporter
            .log(&format!("{a} → {b} octets (−{gain:.0} %)"));
    }
    Ok(())
}

/// `pdf-niveaux-de-gris`.
pub fn grayscale(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let out = output::beside(path, Some("gris"), "pdf")?;
        let extra = [
            "-sColorConversionStrategy=Gray",
            "-dProcessColorModel=/DeviceGray",
            "-dCompatibilityLevel=1.5",
        ];
        gs::pdfwrite(ctx, path, out.path(), &extra)?;
        Ok(out.commit())
    })
}

/// `pdf-reparer` : Ghostscript reconstruit la table des objets à la lecture,
/// puis réécrit un PDF propre.
pub fn repair(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let out = output::beside(path, Some("repare"), "pdf")?;
        gs::pdfwrite(ctx, path, out.path(), &["-dCompatibilityLevel=1.7"])?;
        if lopdf::Document::load(out.path()).is_err() {
            bail!("le PDF n'a pas pu être reconstruit (trop endommagé)");
        }
        Ok(out.commit())
    })
}

fn parse_dpi(value: &str, (min, max): (u32, u32)) -> Result<u32> {
    value
        .trim()
        .parse()
        .ok()
        .filter(|d| (min..=max).contains(d))
        .with_context(|| format!("résolution invalide : « {value} » ({min} à {max} DPI)"))
}

/// Périphérique Ghostscript et extension d'un format d'image.
pub fn image_device(format: &str) -> Result<(&'static str, &'static str)> {
    Ok(match format {
        "png" => ("png16m", "png"),
        "jpg" => ("jpeg", "jpg"),
        other => bail!("format d'image inconnu : {other}"),
    })
}

/// `pdf-vers-images` : une image par page, `<nom>-p<N>.<ext>` à côté du PDF.
pub fn to_images(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let dpi = parse_dpi(ctx.input.option("dpi"), (36, 600))?;
    let (device, ext) = image_device(ctx.input.option("format"))?;
    let mut all = Vec::new();
    for path in &ctx.input.inputs {
        ctx.check_cancel()?;
        let tmp = tempfile::tempdir().context("dossier temporaire impossible")?;
        let pages = gs::rasterize(ctx, path, tmp.path(), (device, ext, dpi))?;
        all.extend(place_pages(ctx, path, &pages, ext)?);
    }
    Ok(all)
}

/// Copie chaque page rendue dans une sortie réservée ; tout ou rien.
fn place_pages(
    ctx: &ToolContext,
    input: &Path,
    pages: &[PathBuf],
    ext: &str,
) -> Result<Vec<PathBuf>> {
    let mut pending: Vec<OutputFile> = Vec::with_capacity(pages.len());
    for (i, page) in pages.iter().enumerate() {
        ctx.check_cancel()?;
        let mut out = output::beside(input, Some(&format!("p{}", i + 1)), ext)?;
        copy_into(page, &mut out)?;
        pending.push(out);
    }
    ctx.reporter.log(&format!(
        "{} : {} image(s)",
        output::file_name(input),
        pending.len()
    ));
    Ok(pending.into_iter().map(OutputFile::commit).collect())
}

fn copy_into(src: &Path, out: &mut OutputFile) -> Result<()> {
    let mut writer = BufWriter::new(out.take_file()?);
    io::copy(&mut File::open(src)?, &mut writer).context("copie de l'image impossible")?;
    writer.flush()?;
    Ok(())
}

/// Résolution de rastérisation pour l'OCR (bon compromis précision/taille).
const OCR_DPI: u32 = 300;

/// `pdf-ocr` : pages rastérisées (Ghostscript), reconnues par Tesseract
/// (image + couche texte invisible), puis réassemblées (lopdf).
pub fn ocr_pdf(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let engine = ocr::prepare(ctx)?;
    per_file(ctx, |path| {
        let mut out = output::beside(path, Some("ocr"), "pdf")?;
        let tmp = tempfile::tempdir().context("dossier temporaire impossible")?;
        ctx.reporter.progress(0.0, "Rastérisation des pages…");
        let images = gs::rasterize(ctx, path, tmp.path(), ("pnggray", "png", OCR_DPI))?;
        let pdfs = recognize_pages(ctx, (&engine.0, &engine.1), &images)?;
        ctx.reporter.progress(95.0, "Assemblage du PDF…");
        let mut doc = super::pdf::merge_paths(&pdfs)?;
        super::pdf::save(&mut doc, out.take_file()?)?;
        Ok(out.commit())
    })
}

fn recognize_pages(
    ctx: &ToolContext,
    engine: (&ocr::Tesseract, &str),
    images: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut pdfs = Vec::with_capacity(images.len());
    for (i, image) in images.iter().enumerate() {
        ctx.check_cancel()?;
        let msg = format!("Reconnaissance du texte, page {}/{}…", i + 1, images.len());
        ctx.reporter.progress(percent(i, images.len()) * 0.9, &msg);
        let base = image.with_extension("");
        ocr::recognize(ctx, engine, image, &base, Some("pdf"))?;
        let pdf = base.with_extension("pdf");
        if !pdf.is_file() {
            bail!("Tesseract n'a pas produit la page {}", i + 1);
        }
        pdfs.push(pdf);
    }
    Ok(pdfs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::{Binary, Resolver};
    use crate::tools::pdf::tests::make_pdf;
    use crate::tools::testutil::{real, scratch_pdf};
    use lopdf::Document;

    #[test]
    fn reglages() {
        assert_eq!(pdf_settings("screen").unwrap(), "-dPDFSETTINGS=/screen");
        assert!(pdf_settings("max").is_err());
        assert_eq!(image_device("jpg").unwrap(), ("jpeg", "jpg"));
        assert!(parse_dpi("20", (36, 600)).is_err() && parse_dpi("150", (36, 600)).is_ok());
    }

    fn run(id: &str, job: JobInput, r: &Resolver) -> Vec<PathBuf> {
        execute(id, job, &MemoryReporter::default(), r).unwrap_or_else(|e| panic!("{id} : {e:#}"))
    }

    #[test]
    fn compresser_gris_reparer_avec_ghostscript() {
        let Some(r) = real(&[Binary::Ghostscript]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("doc.pdf");
        scratch_pdf(&src);
        let job = JobInput::new(vec![src.clone()]).with("niveau", "screen");
        let small = run("pdf-compresser", job, &r);
        assert_eq!(small, vec![tmp.path().join("doc-compresse.pdf")]);
        assert!(
            std::fs::metadata(&small[0]).unwrap().len() < std::fs::metadata(&src).unwrap().len()
        );

        let gray = run("pdf-niveaux-de-gris", JobInput::new(vec![src.clone()]), &r);
        let pngs = run("pdf-vers-images", JobInput::new(gray).with("dpi", "36"), &r);
        let img = image::open(&pngs[0]).unwrap().to_rgb8();
        assert!(
            img.pixels().all(|p| p[0] == p[1] && p[1] == p[2]),
            "pas en gris"
        );

        // PDF abîmé : table xref décalée → reconstruit par Ghostscript.
        let bad = tmp.path().join("abime.pdf");
        let mut bytes = std::fs::read(tmp.path().join("doc-compresse.pdf")).unwrap();
        let pos = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
        bytes.truncate(pos);
        bytes.extend_from_slice(b"startxref\n999999\n%%EOF\n");
        std::fs::write(&bad, bytes).unwrap();
        let fixed = run("pdf-reparer", JobInput::new(vec![bad]), &r);
        assert_eq!(Document::load(&fixed[0]).unwrap().get_pages().len(), 2);
    }

    #[test]
    fn pdf_vers_images_avec_ghostscript() {
        let Some(r) = real(&[Binary::Ghostscript]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("doc 100%.pdf");
        make_pdf(&src, "P", 2);
        std::fs::write(tmp.path().join("doc 100%-p1.jpg"), b"existant").unwrap();
        let job = JobInput::new(vec![src])
            .with("dpi", "72")
            .with("format", "jpg");
        let outs = run("pdf-vers-images", job, &r);
        let names: Vec<_> = outs.iter().map(|p| output::file_name(p)).collect();
        assert_eq!(names, ["doc 100%-p1-2.jpg", "doc 100%-p2.jpg"]);
        let img = image::open(&outs[1]).unwrap();
        assert_eq!((img.width(), img.height()), (595, 842));
        assert_eq!(
            std::fs::read(tmp.path().join("doc 100%-p1.jpg")).unwrap(),
            b"existant"
        );
    }

    #[test]
    fn ocr_avec_tesseract() {
        let Some(r) = real(&[Binary::Ghostscript, Binary::Tesseract]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        // Page « scannée » : le texte n'existe que sous forme d'image.
        let text_pdf = tmp.path().join("texte.pdf");
        crate::tools::testutil::big_text_pdf(&text_pdf, "BONJOUR LE MONDE");
        let pngs = run(
            "pdf-vers-images",
            JobInput::new(vec![text_pdf]).with("dpi", "150"),
            &r,
        );
        let scan = run("pdf-images-vers-pdf", JobInput::new(pngs), &r);
        assert!(!crate::tools::testutil::gs_text(&r, &scan[0]).contains("BONJOUR"));
        let outs = run("pdf-ocr", JobInput::new(scan), &r);
        assert!(outs[0].ends_with("texte-p1-images-ocr.pdf"), "{outs:?}");
        let text = crate::tools::testutil::gs_text(&r, &outs[0]);
        assert!(text.contains("BONJOUR"), "texte reconnu : {text}");
    }
}
