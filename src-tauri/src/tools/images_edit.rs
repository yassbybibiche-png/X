//! Retouches d'images en pur Rust (crate `image`) : redimensionner, compresser,
//! supprimer l'EXIF, pivoter, retourner, rogner, niveaux de gris.
//!
//! L'orientation EXIF est appliquée à la lecture : l'image traitée est celle que
//! l'utilisateur voit. La sortie garde le format de l'entrée (sauf compression → JPEG).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};

use super::images::{OutFormat, encode, flatten_on_white, parse_quality, prepare};
use super::per_file;
use crate::jobs::ToolContext;
use crate::output;

/// Qualité JPEG des retouches (proche du sans-perte visuel).
const EDIT_QUALITY: u8 = 92;
/// Côté maximal accepté en sortie (garde-fou mémoire).
const MAX_SIDE: u32 = 20_000;
const UNREADABLE: &str = "image illisible ou format non pris en charge";

/// Ouvre une image et applique son orientation EXIF ; renvoie aussi l'orientation lue.
fn open_with_orientation(path: &Path) -> Result<(DynamicImage, Orientation)> {
    let mut decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()
        .context(UNREADABLE)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).context(UNREADABLE)?;
    img.apply_orientation(orientation);
    Ok((img, orientation))
}

pub fn open_oriented(path: &Path) -> Result<DynamicImage> {
    Ok(open_with_orientation(path)?.0)
}

/// Format de sortie = format de l'entrée (d'après l'extension).
fn same_format(path: &Path) -> Result<OutFormat> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    OutFormat::parse(&ext).with_context(|| format!("format d'entrée non pris en charge : {ext}"))
}

/// Applique `f` à chaque image ; sortie `<nom>-<suffixe>.<ext d'origine>`.
fn edit_each(
    ctx: &ToolContext,
    suffix: &str,
    f: impl Fn(DynamicImage) -> Result<DynamicImage>,
) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let format = same_format(path)?;
        let mut out = output::beside(path, Some(suffix), format.extension())?;
        let img = f(open_oriented(path)?)?;
        encode(
            &prepare(img, format),
            format,
            EDIT_QUALITY,
            out.take_file()?,
        )?;
        Ok(out.commit())
    })
}

fn positive(value: &str, what: &str) -> Result<f64> {
    let v: f64 = value
        .trim()
        .parse()
        .with_context(|| format!("{what} invalide : « {value} »"))?;
    if !(v.is_finite() && v > 0.0) {
        bail!("{what} doit être strictement positif");
    }
    Ok(v)
}

/// Nouvelle taille, proportions conservées. `cote-max` n'agrandit jamais.
pub fn target_size(w: u32, h: u32, mode: &str, value: f64) -> Result<(u32, u32)> {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let scale = match mode {
        "largeur" => value / wf,
        "hauteur" => value / hf,
        "pourcent" => value / 100.0,
        "cote-max" => (value / wf.max(hf)).min(1.0),
        other => bail!("mode de redimensionnement inconnu : {other}"),
    };
    let size = |side: f64| (side * scale).round().max(1.0);
    let (nw, nh) = (size(wf), size(hf));
    if nw > f64::from(MAX_SIDE) || nh > f64::from(MAX_SIDE) {
        bail!("taille demandée trop grande ({nw} × {nh} px, maximum {MAX_SIDE})");
    }
    Ok((nw as u32, nh as u32))
}

/// `images-redimensionner`.
pub fn resize(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let mode = ctx.input.option("mode").to_string();
    let value = positive(ctx.input.option("valeur"), "la valeur")?;
    target_size(100, 100, &mode, value)?; // mode validé avant tout fichier
    edit_each(ctx, "redim", |img| {
        let (w, h) = target_size(img.width(), img.height(), &mode, value)?;
        Ok(img.resize_exact(w, h, FilterType::Lanczos3))
    })
}

/// `images-pivoter`.
pub fn rotate(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let op: fn(&DynamicImage) -> DynamicImage = match ctx.input.option("angle") {
        "90" => DynamicImage::rotate90,
        "180" => DynamicImage::rotate180,
        "270" => DynamicImage::rotate270,
        other => bail!("angle invalide : {other} (90, 180 ou 270)"),
    };
    edit_each(ctx, "pivote", |img| Ok(op(&img)))
}

/// `images-retourner`.
pub fn flip(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let op: fn(&DynamicImage) -> DynamicImage = match ctx.input.option("axe") {
        "horizontal" => DynamicImage::fliph,
        "vertical" => DynamicImage::flipv,
        other => bail!("axe invalide : {other}"),
    };
    edit_each(ctx, "retourne", |img| Ok(op(&img)))
}

/// `images-niveaux-de-gris` (la transparence est conservée).
pub fn grayscale(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    edit_each(ctx, "gris", |img| {
        Ok(if img.color().has_alpha() {
            DynamicImage::ImageLumaA8(img.to_luma_alpha8())
        } else {
            DynamicImage::ImageLuma8(img.to_luma8())
        })
    })
}

fn pixels(value: &str, what: &str) -> Result<u32> {
    value
        .trim()
        .parse()
        .with_context(|| format!("{what} invalide : « {value} » (nombre entier de pixels)"))
}

/// Rectangle de rognage (x, y, largeur, hauteur), contrôlé contre l'image.
pub fn crop_rect(ctx: &ToolContext) -> Result<[u32; 4]> {
    Ok([
        pixels(ctx.input.option("x"), "x")?,
        pixels(ctx.input.option("y"), "y")?,
        pixels(ctx.input.option("largeur"), "la largeur")?,
        pixels(ctx.input.option("hauteur"), "la hauteur")?,
    ])
}

fn check_crop([x, y, w, h]: [u32; 4], img: &DynamicImage) -> Result<()> {
    let fits = |start: u32, len: u32, max: u32| {
        len > 0 && start.checked_add(len).is_some_and(|e| e <= max)
    };
    if !fits(x, w, img.width()) || !fits(y, h, img.height()) {
        bail!(
            "rectangle {w} × {h} en ({x}, {y}) hors de l'image ({} × {} px)",
            img.width(),
            img.height()
        );
    }
    Ok(())
}

/// `images-rogner`.
pub fn crop(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let rect = crop_rect(ctx)?;
    edit_each(ctx, "rogne", |img| {
        check_crop(rect, &img)?;
        let [x, y, w, h] = rect;
        Ok(img.crop_imm(x, y, w, h))
    })
}

/// `images-compresser` : réencodage JPEG à la qualité choisie.
pub fn compress(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let quality = parse_quality(ctx.input.option("qualite"))?;
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let mut out = output::beside(path, Some("compresse"), "jpg")?;
        let img = flatten_on_white(&open_oriented(path)?);
        encode(&img, OutFormat::Jpeg, quality, out.take_file()?)?;
        report_sizes(ctx, path, out.path());
        Ok(out.commit())
    })
}

fn report_sizes(ctx: &ToolContext, input: &Path, output: &Path) {
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let (before, after) = (size(input), size(output));
    ctx.reporter
        .log(&format!("{before} octets → {after} octets"));
    if after >= before {
        ctx.reporter.warn(&format!(
            "{} : le fichier compressé n'est pas plus léger (baissez la qualité)",
            output::file_name(input)
        ));
    }
}

/// `images-supprimer-exif`. JPEG : segments de métadonnées retirés **sans
/// réencodage** (sans perte), sauf si l'orientation EXIF doit être appliquée.
/// Autres formats : réencodage (le crate `image` n'écrit aucune métadonnée).
pub fn strip_metadata(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let format = same_format(path)?;
        let mut out = output::beside(path, Some("sans-exif"), format.extension())?;
        let (img, orientation) = open_with_orientation(path)?;
        if format == OutFormat::Jpeg && orientation == Orientation::NoTransforms {
            let stripped = strip_jpeg(&std::fs::read(path)?)?;
            out.take_file()?
                .write_all(&stripped)
                .context("écriture impossible")?;
        } else {
            if format == OutFormat::Jpeg {
                ctx.reporter
                    .log("orientation EXIF appliquée (réencodage JPEG qualité 92)");
            }
            encode(
                &prepare(img, format),
                format,
                EDIT_QUALITY,
                out.take_file()?,
            )?;
        }
        Ok(out.commit())
    })
}

/// Segment à conserver : tout sauf APP1–APP15 et commentaires, à l'exception
/// d'APP2 « ICC_PROFILE » (couleurs) et d'APP14 « Adobe » (transformée couleur).
fn keep_segment(marker: u8, payload: &[u8]) -> bool {
    match marker {
        0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
        0xEE => payload.starts_with(b"Adobe"),
        0xE1..=0xEF | 0xFE => false,
        _ => true,
    }
}

/// Retire EXIF, XMP, IPTC et commentaires d'un JPEG (données d'image intactes).
pub fn strip_jpeg(data: &[u8]) -> Result<Vec<u8>> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        bail!("JPEG invalide (signature absente)");
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    while i + 1 < data.len() {
        let marker = data[i + 1];
        if data[i] != 0xFF {
            bail!("structure JPEG invalide à l'octet {i}");
        }
        if marker == 0xFF {
            i += 1; // octet de remplissage
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            // Début des données compressées (ou fin) : copiées telles quelles.
            out.extend_from_slice(&data[i..]);
            return Ok(out);
        }
        let end = segment_end(data, i)?;
        if keep_segment(marker, &data[i + 4..end]) {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    bail!("JPEG tronqué")
}

fn segment_end(data: &[u8], i: usize) -> Result<usize> {
    let len = data
        .get(i + 2..i + 4)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
        .context("JPEG tronqué")?;
    let end = i + 2 + len;
    if len < 2 || end > data.len() {
        bail!("segment JPEG invalide à l'octet {i}");
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use image::{Rgba, RgbaImage};

    fn run(id: &str, job: JobInput) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), &Resolver::default())
    }

    /// PNG w×h : pixel (0,0) vert, le reste rouge.
    fn make_png(path: &Path, w: u32, h: u32) {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([0, 255, 0, 255]));
        img.save(path).unwrap();
    }

    fn dims(path: &Path) -> (u32, u32) {
        let img = open_oriented(path).unwrap();
        (img.width(), img.height())
    }

    fn green_at(path: &Path, x: u32, y: u32) -> bool {
        let p = open_oriented(path).unwrap().to_rgb8().get_pixel(x, y).0;
        p[1] > 200 && p[0] < 60
    }

    fn fixture(w: u32, h: u32) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("photo.png");
        make_png(&src, w, h);
        (tmp, src)
    }

    #[test]
    fn tailles_cibles() {
        assert_eq!(target_size(400, 200, "largeur", 100.0).unwrap(), (100, 50));
        assert_eq!(target_size(400, 200, "hauteur", 100.0).unwrap(), (200, 100));
        assert_eq!(target_size(400, 200, "pourcent", 50.0).unwrap(), (200, 100));
        assert_eq!(
            target_size(400, 200, "cote-max", 1000.0).unwrap(),
            (400, 200)
        );
        assert_eq!(target_size(400, 200, "cote-max", 40.0).unwrap(), (40, 20));
        assert!(target_size(400, 200, "pourcent", 100_000.0).is_err());
        assert!(target_size(400, 200, "zoom", 2.0).is_err());
    }

    #[test]
    fn redimensionner() {
        let (tmp, src) = fixture(40, 20);
        let job = JobInput::new(vec![src])
            .with("mode", "largeur")
            .with("valeur", "10");
        let outs = run("images-redimensionner", job).unwrap();
        assert_eq!(outs, vec![tmp.path().join("photo-redim.png")]);
        assert_eq!(dims(&outs[0]), (10, 5));
        let bad = JobInput::new(outs).with("valeur", "-3");
        assert!(run("images-redimensionner", bad).is_err());
    }

    #[test]
    fn pivoter_et_retourner() {
        let (_tmp, src) = fixture(4, 2);
        let job = JobInput::new(vec![src.clone()]).with("angle", "90");
        let outs = run("images-pivoter", job).unwrap();
        assert_eq!(dims(&outs[0]), (2, 4));
        assert!(green_at(&outs[0], 1, 0), "coin vert après rotation horaire");
        let job = JobInput::new(vec![src]).with("axe", "horizontal");
        let outs = run("images-retourner", job).unwrap();
        assert!(outs[0].ends_with("photo-retourne.png"));
        assert!(green_at(&outs[0], 3, 0));
    }

    #[test]
    fn rogner() {
        let (_tmp, src) = fixture(10, 8);
        let job = JobInput::new(vec![src.clone()])
            .with("x", "0")
            .with("y", "0")
            .with("largeur", "3")
            .with("hauteur", "2");
        let outs = run("images-rogner", job).unwrap();
        assert_eq!(dims(&outs[0]), (3, 2));
        assert!(green_at(&outs[0], 0, 0));
        let job = JobInput::new(vec![src]).with("x", "8").with("largeur", "5");
        let err = run("images-rogner", job).unwrap_err().to_string();
        assert!(err.contains("hors de l'image"), "{err}");
    }

    #[test]
    fn niveaux_de_gris() {
        let (_tmp, src) = fixture(4, 4);
        let outs = run("images-niveaux-de-gris", JobInput::new(vec![src])).unwrap();
        let img = open_oriented(&outs[0]).unwrap();
        assert!(!img.color().has_color());
    }

    #[test]
    fn compresser_en_jpeg() {
        let (tmp, src) = fixture(64, 64);
        let job = JobInput::new(vec![src]).with("qualite", "40");
        let outs = run("images-compresser", job).unwrap();
        assert_eq!(outs, vec![tmp.path().join("photo-compresse.jpg")]);
        assert_eq!(dims(&outs[0]), (64, 64));
    }

    /// Segment APP1 EXIF minimal : orientation `o` + chaîne « secrète ».
    fn exif_segment(o: u16) -> Vec<u8> {
        let mut tiff = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        tiff.extend_from_slice(&o.to_le_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        tiff.extend_from_slice(b"GPS-SECRET");
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend(tiff);
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        seg.extend(payload);
        seg
    }

    fn jpeg_with_exif(path: &Path, w: u32, h: u32, orientation: u16) {
        let mut plain = Vec::new();
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([9, 9, 9])));
        img.write_to(
            &mut std::io::Cursor::new(&mut plain),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let mut data = plain[..2].to_vec();
        data.extend(exif_segment(orientation));
        data.extend_from_slice(&plain[2..]);
        std::fs::write(path, data).unwrap();
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn exif_retire_sans_perte() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("gps.jpg");
        jpeg_with_exif(&src, 8, 4, 1);
        let outs = run("images-supprimer-exif", JobInput::new(vec![src.clone()])).unwrap();
        assert_eq!(outs, vec![tmp.path().join("gps-sans-exif.jpg")]);
        let out = std::fs::read(&outs[0]).unwrap();
        let orig = std::fs::read(&src).unwrap();
        assert!(contains(&orig, b"GPS-SECRET"));
        assert!(!contains(&out, b"GPS-SECRET") && !contains(&out, b"Exif"));
        let sos = |d: &[u8]| d.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert_eq!(
            &out[sos(&out)..],
            &orig[sos(&orig)..],
            "données d'image modifiées"
        );
        assert_eq!(dims(&outs[0]), (8, 4));
    }

    #[test]
    fn exif_orientation_appliquee() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("portrait.jpg");
        jpeg_with_exif(&src, 8, 4, 6); // 6 = rotation de 90° à l'affichage
        assert_eq!(dims(&src), (4, 8));
        let outs = run("images-supprimer-exif", JobInput::new(vec![src])).unwrap();
        let out = std::fs::read(&outs[0]).unwrap();
        assert!(!contains(&out, b"GPS-SECRET"));
        assert_eq!(dims(&outs[0]), (4, 8));
    }

    #[test]
    fn jpeg_invalide() {
        assert!(strip_jpeg(b"pas un jpeg").is_err());
        assert!(strip_jpeg(&[0xFF, 0xD8, 0xFF, 0xE1, 0xFF]).is_err());
    }
}
