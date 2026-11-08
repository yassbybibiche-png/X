//! Outils images en pur Rust (crate `image`).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader, RgbImage};

use crate::jobs::ToolContext;
use crate::output;
use crate::tools::per_file;

/// Côté maximal d'une icône ICO.
const ICO_MAX: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFormat {
    Png,
    Jpeg,
    WebP,
    Bmp,
    Tiff,
    Gif,
    Ico,
    Tga,
    Qoi,
}

impl OutFormat {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value.to_ascii_lowercase().as_str() {
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "webp" => Self::WebP,
            "bmp" => Self::Bmp,
            "tif" | "tiff" => Self::Tiff,
            "gif" => Self::Gif,
            "ico" => Self::Ico,
            "tga" => Self::Tga,
            "qoi" => Self::Qoi,
            "avif" | "heic" => bail!(
                "Le format {value} nécessite ImageMagick : non implémenté en phase 1 (prévu en phase 2)."
            ),
            other => bail!("Format de sortie inconnu : {other}"),
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Gif => "gif",
            Self::Ico => "ico",
            Self::Tga => "tga",
            Self::Qoi => "qoi",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::WebP => ImageFormat::WebP,
            Self::Bmp => ImageFormat::Bmp,
            Self::Tiff => ImageFormat::Tiff,
            Self::Gif => ImageFormat::Gif,
            Self::Ico => ImageFormat::Ico,
            Self::Tga => ImageFormat::Tga,
            Self::Qoi => ImageFormat::Qoi,
        }
    }
}

pub(crate) fn parse_quality(value: &str) -> Result<u8> {
    let q: f64 = value
        .trim()
        .parse()
        .with_context(|| format!("qualité invalide : {value}"))?;
    if !(1.0..=100.0).contains(&q) {
        bail!("la qualité doit être comprise entre 1 et 100");
    }
    Ok(q.round() as u8)
}

/// `images-convertir`.
pub fn convert(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let format = OutFormat::parse(ctx.input.option("format"))?;
    let quality = parse_quality(ctx.input.option("qualite"))?;
    per_file(ctx, |path| convert_one(path, format, quality))
}

pub fn open(path: &Path) -> Result<DynamicImage> {
    ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .context("image illisible ou format non pris en charge")
}

fn convert_one(path: &Path, format: OutFormat, quality: u8) -> Result<PathBuf> {
    let img = prepare(open(path)?, format);
    let mut out = output::beside(path, None, format.extension())?;
    encode(&img, format, quality, out.take_file()?)?;
    Ok(out.commit())
}

/// Adapte l'image aux contraintes du format cible.
pub(crate) fn prepare(img: DynamicImage, format: OutFormat) -> DynamicImage {
    match format {
        OutFormat::Png | OutFormat::Tiff => img,
        OutFormat::Jpeg => flatten_on_white(&img),
        OutFormat::Gif => DynamicImage::ImageRgba8(img.to_rgba8()),
        OutFormat::Ico => fit_icon(img),
        OutFormat::WebP | OutFormat::Bmp | OutFormat::Tga | OutFormat::Qoi => to_8bit(img),
    }
}

fn to_8bit(img: DynamicImage) -> DynamicImage {
    if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    }
}

fn fit_icon(img: DynamicImage) -> DynamicImage {
    let img = if img.width() > ICO_MAX || img.height() > ICO_MAX {
        img.thumbnail(ICO_MAX, ICO_MAX)
    } else {
        img
    };
    DynamicImage::ImageRgba8(img.to_rgba8())
}

/// JPEG n'a pas de transparence : composition sur fond blanc.
pub(crate) fn flatten_on_white(img: &DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return DynamicImage::ImageRgb8(img.to_rgb8());
    }
    let rgba = img.to_rgba8();
    let rgb = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = u16::from(p[3]);
        let blend = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
        image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])])
    });
    DynamicImage::ImageRgb8(rgb)
}

pub(crate) fn encode(img: &DynamicImage, format: OutFormat, quality: u8, file: File) -> Result<()> {
    let mut writer = BufWriter::new(file);
    match format {
        OutFormat::Jpeg => {
            img.write_with_encoder(JpegEncoder::new_with_quality(&mut writer, quality))?
        }
        _ => img.write_to(&mut writer, format.image_format())?,
    }
    writer.flush().context("écriture de l'image impossible")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use image::{Rgba, RgbaImage};

    /// Image 4×3 : pixel (0,0) transparent, le reste rouge opaque.
    fn make_png(path: &Path, w: u32, h: u32) {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        img.save(path).unwrap();
    }

    fn convert_to(input: &Path, format: &str) -> Result<Vec<PathBuf>> {
        let job = JobInput::new(vec![input.to_path_buf()]).with("format", format);
        execute(
            "images-convertir",
            job,
            &MemoryReporter::default(),
            &Resolver::default(),
        )
    }

    #[test]
    fn tous_les_formats_integres() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("photo.png");
        make_png(&src, 4, 3);
        for fmt in ["jpg", "webp", "bmp", "tiff", "gif", "ico", "tga", "qoi"] {
            let outs = convert_to(&src, fmt).unwrap();
            let ext = OutFormat::parse(fmt).unwrap().extension();
            assert_eq!(outs, vec![tmp.path().join(format!("photo.{ext}"))]);
            let back = open(&outs[0]).unwrap();
            assert_eq!((back.width(), back.height()), (4, 3), "{fmt}");
        }
    }

    #[test]
    fn jpeg_transparence_sur_fond_blanc() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("logo.png");
        make_png(&src, 16, 16);
        let outs = convert_to(&src, "jpg").unwrap();
        let back = open(&outs[0]).unwrap().to_rgb8();
        let p = back.get_pixel(0, 0).0;
        assert!(p.iter().all(|c| *c > 200), "pixel transparent devenu {p:?}");
        assert_eq!(
            ImageReader::open(&outs[0])
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format(),
            Some(ImageFormat::Jpeg)
        );
    }

    #[test]
    fn ico_reduit_a_256() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("grand.png");
        make_png(&src, 512, 256);
        let back = open(&convert_to(&src, "ico").unwrap()[0]).unwrap();
        assert_eq!((back.width(), back.height()), (256, 128));
    }

    #[test]
    fn meme_format_sans_ecrasement() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("img.png");
        make_png(&src, 4, 3);
        let before = std::fs::read(&src).unwrap();
        let outs = convert_to(&src, "png").unwrap();
        assert_eq!(outs, vec![tmp.path().join("img-2.png")]);
        assert_eq!(std::fs::read(&src).unwrap(), before);
    }

    #[test]
    fn lot_avec_un_fichier_invalide() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = tmp.path().join("ok.png");
        let bad = tmp.path().join("faux.png");
        make_png(&ok, 4, 3);
        std::fs::write(&bad, b"pas une image").unwrap();
        let rep = MemoryReporter::default();
        let job = JobInput::new(vec![bad, ok]).with("format", "bmp");
        let outs = execute("images-convertir", job, &rep, &Resolver::default()).unwrap();
        assert_eq!(outs, vec![tmp.path().join("ok.bmp")]);
        assert_eq!(rep.count(crate::jobs::EventKind::Warning), 1);
        assert!(!tmp.path().join("faux.bmp").exists());
    }

    #[test]
    fn avif_non_implemente_explicite() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("a.png");
        make_png(&src, 2, 2);
        let err = convert_to(&src, "avif").unwrap_err().to_string();
        assert!(err.contains("non implémenté en phase 1"), "{err}");
    }

    #[test]
    fn qualite_hors_bornes() {
        assert!(parse_quality("0").is_err());
        assert!(parse_quality("abc").is_err());
        assert_eq!(parse_quality("85").unwrap(), 85);
    }
}
