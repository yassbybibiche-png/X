//! Images : agrandissement par IA (Real-ESRGAN ncnn Vulkan) et OCR (Tesseract).
//!
//! Real-ESRGAN : binaire + modèles (`models/<nom>.param|.bin`, livrés dans le
//! zip officiel) dans `<données>/bin/realesrgan/`. Sans GPU Vulkan, le binaire
//! échoue (« vkCreateInstance failed ») : l'outil se rabat alors sur un
//! agrandissement classique Lanczos, **signalé** (avertissement + nom
//! `-x4-lanczos.png`), jamais présenté comme un résultat IA.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;

use super::images::{OutFormat, encode};
use super::images_edit::open_oriented;
use super::{ocr, per_file};
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};
use crate::sidecar::{Binary, Stream};

/// Côté maximal d'entrée (×4 → 16 000 px).
const MAX_INPUT_SIDE: u32 = 4000;
const SCALE: u32 = 4;

/// Modèles livrés dans le zip officiel.
pub fn model_name(choice: &str) -> Result<&'static str> {
    Ok(match choice {
        "photo" => "realesrgan-x4plus",
        "dessin" => "realesrgan-x4plus-anime",
        other => bail!("modèle inconnu : {other}"),
    })
}

/// Dossier `models` à côté du binaire, avec le modèle demandé.
pub fn models_dir(exe: &Path, model: &str) -> Result<PathBuf> {
    let dir = exe
        .parent()
        .map(|d| d.join("models"))
        .context("dossier de Real-ESRGAN introuvable")?;
    for ext in ["param", "bin"] {
        let file = dir.join(format!("{model}.{ext}"));
        if !file.is_file() {
            bail!(
                "modèle Real-ESRGAN absent : {} — réinstallez Real-ESRGAN depuis l'onglet Diagnostic \
                 (le zip officiel contient les modèles).",
                file.display()
            );
        }
    }
    Ok(dir)
}

pub fn esrgan_args(input: &Path, out: &Path, model: &str, models: &Path) -> Vec<OsString> {
    vec![
        "-i".into(),
        input.into(),
        "-o".into(),
        out.into(),
        "-n".into(),
        model.into(),
        "-s".into(),
        SCALE.to_string().into(),
        "-f".into(),
        "png".into(),
        "-m".into(),
        models.into(),
    ]
}

/// « 12.34% » → 12.
pub fn parse_percent(line: &str) -> Option<u32> {
    let v: f32 = line.trim().strip_suffix('%')?.trim().parse().ok()?;
    Some(v as u32)
}

/// Échec typique sans GPU / pilote Vulkan.
pub fn is_vulkan_failure(stderr: &str) -> bool {
    [
        "vkCreateInstance failed",
        "invalid gpu device",
        "vkEnumeratePhysicalDevices",
    ]
    .iter()
    .any(|m| stderr.contains(m))
}

/// `images-agrandir-ia`.
pub fn upscale(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let model = model_name(ctx.input.option("modele"))?;
    let exe = ctx.binary(Binary::RealEsrgan)?;
    let models = models_dir(&exe, model)?;
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let (w, h) = image::image_dimensions(path).context("image illisible")?;
        if w.max(h) > MAX_INPUT_SIDE {
            bail!(
                "image trop grande ({w} × {h} px) : {MAX_INPUT_SIDE} px maximum avant agrandissement ×4"
            );
        }
        let tmp = tempfile::tempdir()?;
        let png_in = tmp.path().join("entree.png");
        open_oriented(path)?
            .save(&png_in)
            .context("préparation de l'image impossible")?;
        let result = tmp.path().join("sortie.png");
        match run_esrgan(ctx, &exe, (&png_in, &result), (model, &models)) {
            Ok(()) => place(path, "x4", &result),
            Err(e) if is_vulkan_failure(&format!("{e:#}")) => lanczos_fallback(ctx, path),
            Err(e) => Err(e),
        }
    })
}

fn run_esrgan(
    ctx: &ToolContext,
    exe: &Path,
    (input, out): (&Path, &Path),
    (model, models): (&str, &Path),
) -> Result<()> {
    let last = AtomicU32::new(u32::MAX);
    let reporter = ctx.reporter;
    let on_line = |s: Stream, line: &str| {
        if let (Stream::Stderr, Some(p)) = (s, parse_percent(line))
            && last.swap(p, Ordering::Relaxed) != p
        {
            reporter.progress(p.min(99) as f32, "Agrandissement par IA…");
        }
    };
    let run = ctx.run_sidecar(
        exe,
        "Real-ESRGAN",
        &esrgan_args(input, out, model, models),
        Some(&on_line),
    )?;
    // Le binaire peut sortir en code 0 sans rien produire (pas de GPU).
    if !out.is_file() {
        bail!(
            "Real-ESRGAN n'a produit aucune image : {}",
            crate::sidecar::truncate_stderr(&run.stderr)
        );
    }
    Ok(())
}

fn place(input: &Path, suffix: &str, result: &Path) -> Result<PathBuf> {
    let mut out: OutputFile = output::beside(input, Some(suffix), "png")?;
    let mut w = std::io::BufWriter::new(out.take_file()?);
    std::io::copy(&mut std::fs::File::open(result)?, &mut w)?;
    std::io::Write::flush(&mut w)?;
    Ok(out.commit())
}

fn lanczos_fallback(ctx: &ToolContext, path: &Path) -> Result<PathBuf> {
    ctx.reporter.warn(
        "Aucun GPU compatible Vulkan : Real-ESRGAN ne peut pas fonctionner sur cette machine. \
         Agrandissement CLASSIQUE (Lanczos ×4, sans IA) produit à la place : fichier « -x4-lanczos.png ».",
    );
    let img = open_oriented(path)?;
    let big = img.resize_exact(
        img.width() * SCALE,
        img.height() * SCALE,
        FilterType::Lanczos3,
    );
    let mut out = output::beside(path, Some("x4-lanczos"), "png")?;
    encode(&big, OutFormat::Png, 100, out.take_file()?)?;
    Ok(out.commit())
}

/// `images-ocr-texte` : texte reconnu dans `<nom>.txt`.
pub fn ocr_text(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (tess, langs) = ocr::prepare(ctx)?;
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let mut out = output::beside(path, None, "txt")?;
        let tmp = tempfile::tempdir()?;
        // Image normalisée en PNG (orientation EXIF appliquée, formats exotiques acceptés).
        let png = tmp.path().join("page.png");
        open_oriented(path)?
            .save(&png)
            .context("préparation de l'image impossible")?;
        let text = ocr::recognize(ctx, (&tess, &langs), &png, Path::new("stdout"), None)?;
        if text.trim().is_empty() {
            ctx.reporter.warn(&format!(
                "{} : aucun texte reconnu",
                output::file_name(path)
            ));
        }
        std::io::Write::write_all(&mut out.take_file()?, text.as_bytes())?;
        Ok(out.commit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{EventKind, JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use crate::tools::testutil::real;
    use image::{Rgba, RgbaImage};

    #[test]
    fn analyse_et_modeles() {
        assert_eq!(parse_percent("45.83%"), Some(45));
        assert_eq!(parse_percent("abc"), None);
        assert!(is_vulkan_failure(
            "vkCreateInstance failed -9\ninvalid gpu device"
        ));
        assert!(model_name("photo").is_ok() && model_name("x").is_err());
    }

    fn sample(dir: &Path) -> PathBuf {
        let p = dir.join("petit.png");
        RgbaImage::from_pixel(8, 6, Rgba([0, 128, 255, 255]))
            .save(&p)
            .unwrap();
        p
    }

    /// Faux Real-ESRGAN : copie l'entrée vers la sortie, affiche une progression.
    #[cfg(unix)]
    #[test]
    fn cablage_realesrgan_avec_faux_binaire() {
        use crate::tools::testutil::fake_bin;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let script = r#"while [ $# -gt 0 ]; do case "$1" in -i) IN="$2";; -o) OUT="$2";; -m) M="$2";; esac; shift; done
[ -f "$M/realesrgan-x4plus.param" ] || exit 3
echo "50.00%" >&2; cp "$IN" "$OUT"; echo "100.00%" >&2"#;
        fake_bin(&bin, "realesrgan-ncnn-vulkan", script);
        let resolver = Resolver::new(None, None, Some(bin.clone().into_os_string()));
        let src = sample(tmp.path());
        let err = execute(
            "images-agrandir-ia",
            JobInput::new(vec![src.clone()]),
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("modèle Real-ESRGAN absent"),
            "{err}"
        );

        std::fs::create_dir_all(bin.join("models")).unwrap();
        for ext in ["param", "bin"] {
            std::fs::write(bin.join(format!("models/realesrgan-x4plus.{ext}")), b"x").unwrap();
        }
        let rep = MemoryReporter::default();
        let outs = execute(
            "images-agrandir-ia",
            JobInput::new(vec![src]),
            &rep,
            &resolver,
        )
        .unwrap();
        assert!(outs[0].ends_with("petit-x4.png"));
        let progress = rep
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(k, m)| *k == EventKind::Progress && m.contains("IA"));
        assert!(progress);
    }

    /// Vrai Real-ESRGAN (installé depuis le manifeste) : sur une machine sans GPU
    /// Vulkan (cette VM), le repli Lanczos doit être produit ET signalé.
    #[test]
    fn realesrgan_reel_ou_repli_signale() {
        let Some(r) = real(&[Binary::RealEsrgan]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let rep = MemoryReporter::default();
        let outs = execute(
            "images-agrandir-ia",
            JobInput::new(vec![sample(tmp.path())]),
            &rep,
            &r,
        )
        .unwrap();
        let img = image::open(&outs[0]).unwrap();
        assert_eq!((img.width(), img.height()), (32, 24));
        if outs[0].ends_with("petit-x4-lanczos.png") {
            assert_eq!(rep.count(EventKind::Warning), 1, "repli non signalé");
        } else {
            assert!(outs[0].ends_with("petit-x4.png"));
        }
    }

    #[test]
    fn ocr_image_avec_tesseract() {
        let Some(r) = real(&[Binary::Ghostscript, Binary::Tesseract]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("t.pdf");
        crate::tools::testutil::big_text_pdf(&pdf, "Facture 2026");
        let png = execute(
            "pdf-vers-images",
            JobInput::new(vec![pdf]).with("dpi", "150"),
            &MemoryReporter::default(),
            &r,
        )
        .unwrap();
        let outs = execute(
            "images-ocr-texte",
            JobInput::new(png).with("langue", "fra"),
            &MemoryReporter::default(),
            &r,
        )
        .unwrap();
        assert!(outs[0].ends_with("t-p1.txt"));
        let text = std::fs::read_to_string(&outs[0]).unwrap();
        assert!(text.contains("Facture 2026"), "{text}");
    }
}
