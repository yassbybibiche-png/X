//! Outils images avec ImageMagick : formats AVIF/HEIC, filigrane texte,
//! détourage simple, planche contact, WebP avec perte (optimisation web).
//!
//! Détourage : « seuil de couleur unie » — le fond (couleur du coin haut-gauche
//! ou couleur choisie) est rendu transparent par remplissage depuis les bords,
//! avec une tolérance. Les sujets complexes (cheveux, fonds photo) relèveraient
//! d'un modèle IA (ex. rembg / U²-Net) : option future, voir PLAN.md.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::magick::{self, Magick, hex_color, literal_text, strings};
use super::per_file;
use crate::jobs::ToolContext;
use crate::output;

/// Formats confiés à ImageMagick (absents du moteur intégré).
pub const MAGICK_FORMATS: [&str; 3] = ["avif", "heic", "heif"];

pub fn needs_magick(ext: &str) -> bool {
    MAGICK_FORMATS.contains(&ext.to_ascii_lowercase().as_str())
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Nom ImageMagick d'un format (HEIF est lu par le codeur HEIC).
fn magick_name(ext: &str) -> String {
    match ext {
        "heif" => "HEIC".into(),
        "jpg" => "JPEG".into(),
        other => other.to_ascii_uppercase(),
    }
}

/// Erreur claire si la build d'ImageMagick ne lit/n'écrit pas ce format.
fn check_support(im: &Magick, ext: &str, write: bool) -> Result<()> {
    let s = im.support(&magick_name(ext))?;
    let ok = if write { s.write } else { s.read };
    if !ok {
        let what = if write { "l'écriture" } else { "la lecture" };
        bail!(
            "Cette version d'ImageMagick ne prend pas en charge {what} du format {} \
             (bibliothèque libheif/AVIF absente de la build). Choisissez un autre format \
             ou installez une build d'ImageMagick complète (onglet Diagnostic).",
            ext.to_uppercase()
        );
    }
    Ok(())
}

/// `images-convertir` quand l'entrée ou la sortie est AVIF/HEIC.
pub fn convert_with_magick(
    ctx: &ToolContext,
    path: &Path,
    out_ext: &str,
    quality: u8,
) -> Result<PathBuf> {
    let im = Magick::new(ctx)
        .context("le format AVIF/HEIC nécessite ImageMagick — voir l'onglet Diagnostic")?;
    let in_ext = ext_of(path);
    if needs_magick(&in_ext) {
        check_support(&im, &in_ext, false)?;
    }
    if needs_magick(out_ext) {
        check_support(&im, out_ext, true)?;
    }
    let mut out = output::beside(path, None, out_ext)?;
    let mut args = vec![magick::input(path)?];
    args.extend(strings(&["-auto-orient", "-quality"]));
    args.push(quality.to_string().into());
    im.convert_into(ctx, args, &mut out, out_ext)?;
    Ok(out.commit())
}

fn int_option(ctx: &ToolContext, key: &str, (min, max): (u32, u32)) -> Result<u32> {
    let v = ctx.input.option(key);
    v.trim()
        .parse()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .with_context(|| format!("valeur invalide pour {key} : « {v} » ({min} à {max})"))
}

/// Extension de sortie = celle de l'entrée (formats intégrés), sinon PNG.
fn keep_ext(path: &Path) -> String {
    let ext = ext_of(path);
    if super::images::OutFormat::parse(&ext).is_ok() {
        ext
    } else {
        "png".into()
    }
}

/// `-gravity` d'une position.
fn gravity(position: &str) -> Result<&'static str> {
    Ok(match position {
        "centre" => "center",
        "bas-droite" => "southeast",
        "bas-gauche" => "southwest",
        "haut-droite" => "northeast",
        other => bail!("position inconnue : {other}"),
    })
}

/// Arguments du filigrane : texte blanc semi-transparent à contour sombre,
/// taille proportionnelle à la largeur de l'image.
pub fn watermark_args(
    text: &str,
    position: &str,
    opacity: u32,
    width: u32,
) -> Result<Vec<OsString>> {
    let size = (width as f32 / (text.chars().count().max(4) as f32 * 0.9)).clamp(12.0, 400.0);
    let alpha = f64::from(opacity) / 100.0;
    let mut args = strings(&["-gravity", gravity(position)?, "-pointsize"]);
    args.push(format!("{size:.0}").into());
    args.extend(strings(&["-fill"]));
    args.push(format!("rgba(255,255,255,{alpha:.2})").into());
    args.extend(strings(&["-stroke"]));
    args.push(format!("rgba(0,0,0,{:.2})", alpha * 0.6).into());
    args.extend(strings(&["-strokewidth", "1", "-annotate", "+20+20"]));
    args.push(literal_text(text).into());
    Ok(args)
}

/// `images-filigrane`.
pub fn watermark(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let text = ctx.input.option("texte").trim().to_string();
    if text.is_empty() {
        bail!("indiquez le texte du filigrane");
    }
    let opacity = int_option(ctx, "opacite", (5, 100))?;
    let position = ctx.input.option("position").to_string();
    gravity(&position)?;
    let im = Magick::new(ctx)?;
    per_file(ctx, |path| {
        let (w, _) = image::image_dimensions(path).context("image illisible")?;
        let ext = keep_ext(path);
        let mut out = output::beside(path, Some("filigrane"), &ext)?;
        let mut args = vec![magick::input(path)?, "-auto-orient".into()];
        args.extend(watermark_args(&text, &position, opacity, w)?);
        im.convert_into(ctx, args, &mut out, &ext)?;
        Ok(out.commit())
    })
}

/// Couleur du fond : choisie, ou pixel du coin haut-gauche (`auto`).
fn background_color(choice: &str, path: &Path) -> Result<String> {
    if !choice.trim().eq_ignore_ascii_case("auto") {
        return hex_color(choice);
    }
    let img = super::images_edit::open_oriented(path)?.to_rgb8();
    let p = img.get_pixel(0, 0).0;
    Ok(format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]))
}

/// Arguments du détourage : bordure de 1 px de la couleur du fond (relie tous
/// les bords), remplissage transparent depuis le coin, puis retrait de la bordure.
pub fn cutout_args(color: &str, tolerance: u32) -> Vec<OsString> {
    let mut args = strings(&[
        "-alpha",
        "set",
        "-bordercolor",
        color,
        "-border",
        "1",
        "-fuzz",
    ]);
    args.push(format!("{tolerance}%").into());
    args.extend(strings(&[
        "-fill",
        "none",
        "-draw",
        "color 0,0 floodfill",
        "-shave",
        "1x1",
    ]));
    args
}

/// `images-detourer` : PNG transparent.
pub fn cutout(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let tolerance = int_option(ctx, "tolerance", (0, 100))?;
    let choice = ctx.input.option("couleur").to_string();
    let im = Magick::new(ctx)?;
    per_file(ctx, |path| {
        let color = background_color(&choice, path)?;
        let mut out = output::beside(path, Some("detoure"), "png")?;
        let mut args = vec![magick::input(path)?, "-auto-orient".into()];
        args.extend(cutout_args(&color, tolerance));
        im.convert_into(ctx, args, &mut out, "png")?;
        ctx.reporter.log(&format!(
            "{} : fond {color} retiré",
            output::file_name(path)
        ));
        Ok(out.commit())
    })
}

/// `images-planche-contact` : `planche.jpg` dans le dossier de la 1re image.
pub fn contact_sheet(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let cols = int_option(ctx, "colonnes", (1, 20))?;
    let size = int_option(ctx, "taille", (32, 1024))?;
    let im = Magick::new(ctx)?;
    let inputs = &ctx.input.inputs;
    let mut out = output::reserve(&output::parent_dir(&inputs[0]), "planche", "jpg")?;
    let mut args = Vec::new();
    if ctx.input.option("etiquettes") == "true" {
        args.extend(strings(&["-label", "%f"]));
    }
    for p in inputs {
        args.push(magick::input(p)?);
    }
    args.extend(strings(&["-tile"]));
    args.push(format!("{cols}x").into());
    args.extend(strings(&["-geometry"]));
    args.push(format!("{size}x{size}+6+6").into());
    args.extend(strings(&["-background", "white", "-quality", "88"]));
    ctx.reporter
        .progress(10.0, &format!("Assemblage de {} image(s)…", inputs.len()));
    im.montage_into(ctx, args, &mut out, "jpg")?;
    Ok(vec![out.commit()])
}

/// `images-optimiser-web` au format WebP (avec perte, qualité 80).
pub fn web_webp(ctx: &ToolContext, side: u32, path: &Path) -> Result<PathBuf> {
    let im = Magick::new(ctx)
        .context("le WebP avec perte nécessite ImageMagick — voir l'onglet Diagnostic")?;
    check_support(&im, "webp", true)?;
    let mut out = output::beside(path, Some("web"), "webp")?;
    let mut args = vec![magick::input(path)?];
    args.extend(strings(&["-auto-orient", "-strip", "-resize"]));
    args.push(format!("{side}x{side}>").into());
    args.extend(strings(&["-quality", "80"]));
    im.convert_into(ctx, args, &mut out, "webp")?;
    Ok(out.commit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::{Binary, Resolver};
    use crate::tools::testutil::real;
    use image::{Rgba, RgbaImage};

    #[test]
    fn arguments() {
        let a: Vec<String> = cutout_args("#ffffff", 10)
            .iter()
            .map(|s| s.to_string_lossy().into())
            .collect();
        assert!(a.windows(2).any(|w| w == ["-fuzz", "10%"]));
        let w = watermark_args("@/etc/passwd", "centre", 40, 800).unwrap();
        assert_eq!(w.last().unwrap(), "\\@/etc/passwd");
        assert!(gravity("milieu").is_err());
    }

    fn run(id: &str, job: JobInput, r: &Resolver) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), r)
    }

    /// Fond blanc 60×40, carré rouge au centre.
    fn subject(dir: &Path) -> PathBuf {
        let mut img = RgbaImage::from_pixel(60, 40, Rgba([255, 255, 255, 255]));
        for x in 20..40 {
            for y in 10..30 {
                img.put_pixel(x, y, Rgba([200, 0, 0, 255]));
            }
        }
        let p = dir.join("objet.png");
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn detourer_filigrane_planche_avec_imagemagick() {
        let Some(r) = real(&[Binary::ImageMagick]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = subject(tmp.path());
        let cut = run("images-detourer", JobInput::new(vec![src.clone()]), &r).unwrap();
        assert!(cut[0].ends_with("objet-detoure.png"));
        let img = image::open(&cut[0]).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (60, 40));
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "fond transparent");
        assert_eq!(img.get_pixel(30, 20).0, [200, 0, 0, 255], "sujet intact");

        let job = JobInput::new(vec![src.clone()]).with("texte", "@/etc/passwd 100%");
        let wm = run("images-filigrane", job, &r).unwrap();
        let marked = image::open(&wm[0]).unwrap().to_rgba8();
        let original = image::open(&src).unwrap().to_rgba8();
        assert_eq!(marked.dimensions(), (60, 40));
        assert_ne!(marked.as_raw(), original.as_raw(), "filigrane absent");

        let job = JobInput::new(vec![src.clone(), cut[0].clone()])
            .with("colonnes", "2")
            .with("taille", "50");
        let sheet = run("images-planche-contact", job, &r).unwrap();
        assert_eq!(sheet, vec![tmp.path().join("planche.jpg")]);
        let s = image::open(&sheet[0]).unwrap();
        assert_eq!((s.width(), s.height()), (2 * (50 + 12), 50 + 12));
    }

    #[test]
    fn avif_heic_selon_la_build() {
        let Some(r) = real(&[Binary::ImageMagick]) else {
            return;
        };
        let im = Magick::from_path(r.resolve(Binary::ImageMagick).unwrap().path).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let src = subject(tmp.path());
        let job = JobInput::new(vec![src.clone()]).with("format", "avif");
        let result = run("images-convertir", job, &r);
        if im.support("AVIF").unwrap().write {
            let outs = result.unwrap();
            assert!(outs[0].ends_with("objet.avif"));
            let back = run(
                "images-convertir",
                JobInput::new(outs).with("format", "png"),
                &r,
            )
            .unwrap();
            assert_eq!(image::open(&back[0]).unwrap().width(), 60);
        } else {
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("ne prend pas en charge l'écriture du format AVIF"),
                "{err}"
            );
            assert!(!tmp.path().join("objet.avif").exists());
        }
        // Lecture AVIF réelle (fichier produit par ffmpeg) → PNG.
        if let (Some(ff), true) = (r.resolve(Binary::Ffmpeg), im.support("AVIF").unwrap().read) {
            let avif = tmp.path().join("rouge.avif");
            let args = [
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=32x24",
            ];
            let mut all: Vec<OsString> = strings(&args);
            all.extend(strings(&["-frames:v", "1", "-pix_fmt", "yuv420p"]));
            all.push(avif.clone().into());
            let made =
                crate::sidecar::run(&ff.path, &all, std::time::Duration::from_secs(60)).unwrap();
            if made.status.success() {
                let png = run(
                    "images-convertir",
                    JobInput::new(vec![avif]).with("format", "png"),
                    &r,
                )
                .unwrap();
                let p = image::open(&png[0]).unwrap().to_rgb8();
                assert_eq!(p.dimensions(), (32, 24));
                assert!(p.get_pixel(5, 5).0[0] > 200, "rouge attendu");
            }
        }
        let webp = run(
            "images-optimiser-web",
            JobInput::new(vec![src])
                .with("format", "webp")
                .with("cote_max", "30"),
            &r,
        )
        .unwrap();
        assert_eq!(image::open(&webp[0]).unwrap().width(), 30);
    }
}
