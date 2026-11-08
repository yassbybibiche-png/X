//! Outils images en pur Rust (crate `image`) : icône multi-tailles, bordure,
//! vignettes, optimisation web (JPEG), GIF animé.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::{Delay, DynamicImage, Frame, Rgba, RgbaImage};

use super::images::{OutFormat, encode, flatten_on_white, prepare};
use super::images_edit::open_oriented;
use super::{per_file, percent};
use crate::jobs::ToolContext;
use crate::output;

/// Tailles d'une icône Windows complète.
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn int_option(ctx: &ToolContext, key: &str, (min, max): (u32, u32)) -> Result<u32> {
    let value = ctx.input.option(key);
    value
        .trim()
        .parse()
        .ok()
        .filter(|v| (min..=max).contains(v))
        .with_context(|| format!("valeur invalide pour {key} : « {value} » ({min} à {max})"))
}

/// Image centrée sur un carré transparent (une icône est carrée).
fn square(img: &DynamicImage) -> RgbaImage {
    let side = img.width().max(img.height());
    let mut canvas = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    let (x, y) = ((side - img.width()) / 2, (side - img.height()) / 2);
    image::imageops::overlay(&mut canvas, &img.to_rgba8(), x.into(), y.into());
    canvas
}

/// Tailles retenues : celles ≤ côté de l'image (au moins 16), jamais d'agrandissement inutile.
pub fn icon_sizes(side: u32) -> Vec<u32> {
    let sizes: Vec<u32> = ICO_SIZES.iter().copied().filter(|s| *s <= side).collect();
    if sizes.is_empty() { vec![16] } else { sizes }
}

/// `images-vers-ico` : .ico contenant 16 à 256 px (PNG compressés).
pub fn to_ico(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        let mut out = output::beside(path, None, "ico")?;
        let base = square(&open_oriented(path)?);
        let sizes = icon_sizes(base.width());
        let frames = sizes
            .iter()
            .map(|s| {
                let img = image::imageops::resize(&base, *s, *s, FilterType::Lanczos3);
                let mut png = Vec::new();
                DynamicImage::ImageRgba8(img)
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
                Ok(IcoFrame::with_encoded(
                    png,
                    *s,
                    *s,
                    image::ExtendedColorType::Rgba8,
                )?)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut w = BufWriter::new(out.take_file()?);
        IcoEncoder::new(&mut w).encode_images(&frames)?;
        w.flush()?;
        ctx.reporter.log(&format!("tailles : {sizes:?} px"));
        Ok(out.commit())
    })
}

fn same_format(path: &Path) -> Result<OutFormat> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    OutFormat::parse(&ext)
}

/// Encode `img` au format de `path` dans `<nom>-<suffixe>.<ext>`.
fn save_like(path: &Path, suffix: &str, img: DynamicImage) -> Result<PathBuf> {
    let format = same_format(path)?;
    let mut out = output::beside(path, Some(suffix), format.extension())?;
    encode(&prepare(img, format), format, 92, out.take_file()?)?;
    Ok(out.commit())
}

/// `images-bordure`.
pub fn border(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let size = int_option(ctx, "epaisseur", (1, 2000))?;
    let color = parse_rgb(ctx.input.option("couleur"))?;
    per_file(ctx, |path| {
        let img = open_oriented(path)?;
        let (w, h) = (img.width() + 2 * size, img.height() + 2 * size);
        let mut canvas = RgbaImage::from_pixel(w, h, Rgba([color[0], color[1], color[2], 255]));
        image::imageops::overlay(&mut canvas, &img.to_rgba8(), size.into(), size.into());
        save_like(path, "bordure", DynamicImage::ImageRgba8(canvas))
    })
}

/// `#rrggbb` → [r, g, b].
pub fn parse_rgb(value: &str) -> Result<[u8; 3]> {
    let hex = super::magick::hex_color(value)?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Ok([byte(1), byte(3), byte(5)])
}

/// `images-vignettes` : miniatures (côté max.), format d'origine.
pub fn thumbnails(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let side = int_option(ctx, "taille", (16, 2048))?;
    per_file(ctx, |path| {
        let img = open_oriented(path)?;
        let thumb = if img.width() > side || img.height() > side {
            img.resize(side, side, FilterType::Lanczos3)
        } else {
            img
        };
        save_like(path, "vignette", thumb)
    })
}

/// `images-optimiser-web` (JPEG) : côté max., qualité 80, métadonnées retirées.
pub fn web_jpeg(ctx: &ToolContext, side: u32, path: &Path) -> Result<PathBuf> {
    let img = open_oriented(path)?;
    let img = if img.width() > side || img.height() > side {
        img.resize(side, side, FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = output::beside(path, Some("web"), "jpg")?;
    encode(
        &flatten_on_white(&img),
        OutFormat::Jpeg,
        80,
        out.take_file()?,
    )?;
    let (a, b) = (
        std::fs::metadata(path)?.len(),
        std::fs::metadata(out.path())?.len(),
    );
    ctx.reporter
        .log(&format!("{} : {a} → {b} octets", output::file_name(path)));
    Ok(out.commit())
}

/// `images-assembler-gif` : images dans l'ordre, ajustées à la taille de la première.
pub fn animated_gif(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let delay_ms = int_option(ctx, "delai", (20, 10_000))?;
    let inputs = &ctx.input.inputs;
    let mut out = output::beside(&inputs[0], Some("anime"), "gif")?;
    let first = open_oriented(&inputs[0])?;
    let (w, h) = (first.width(), first.height());
    if w * h > 4_000_000 {
        bail!(
            "première image trop grande pour un GIF ({w} × {h} px, max. 4 Mpx) : réduisez-la d'abord"
        );
    }
    let mut frames = Vec::with_capacity(inputs.len());
    for (i, path) in inputs.iter().enumerate() {
        ctx.check_cancel()?;
        ctx.reporter.progress(
            percent(i, inputs.len()) * 0.5,
            &format!("Lecture de {}…", output::file_name(path)),
        );
        let img = if i == 0 {
            first.clone()
        } else {
            open_oriented(path)?
        };
        frames.push(Frame::from_parts(
            fit(&img, w, h),
            0,
            0,
            Delay::from_numer_denom_ms(delay_ms, 1),
        ));
    }
    ctx.reporter.progress(60.0, "Encodage du GIF…");
    let mut writer = BufWriter::new(out.take_file()?);
    {
        let mut enc = GifEncoder::new_with_speed(&mut writer, 10);
        enc.set_repeat(Repeat::Infinite)?;
        enc.encode_frames(frames)?;
    }
    writer.flush()?;
    Ok(vec![out.commit()])
}

/// Image ajustée (proportions conservées) et centrée sur un fond transparent `w`×`h`.
fn fit(img: &DynamicImage, w: u32, h: u32) -> RgbaImage {
    let resized = if img.width() == w && img.height() == h {
        img.to_rgba8()
    } else {
        img.resize(w, h, FilterType::Triangle).to_rgba8()
    };
    let mut canvas = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    let (x, y) = ((w - resized.width()) / 2, (h - resized.height()) / 2);
    image::imageops::overlay(&mut canvas, &resized, x.into(), y.into());
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use image::AnimationDecoder;
    use image::codecs::gif::GifDecoder;

    fn png(dir: &Path, name: &str, w: u32, h: u32, c: [u8; 4]) -> PathBuf {
        let p = dir.join(name);
        RgbaImage::from_pixel(w, h, Rgba(c)).save(&p).unwrap();
        p
    }

    fn run(id: &str, job: JobInput) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), &Resolver::default())
    }

    #[test]
    fn icone_multi_tailles() {
        let tmp = tempfile::tempdir().unwrap();
        let src = png(tmp.path(), "logo.png", 300, 100, [255, 0, 0, 255]);
        let outs = run("images-vers-ico", JobInput::new(vec![src])).unwrap();
        assert_eq!(outs, vec![tmp.path().join("logo.ico")]);
        let data = std::fs::read(&outs[0]).unwrap();
        assert_eq!(u16::from_le_bytes([data[4], data[5]]), 7, "7 tailles");
        assert_eq!(icon_sizes(40), [16, 24, 32]);
        assert_eq!(icon_sizes(8), [16]);
    }

    #[test]
    fn bordure_et_vignette() {
        let tmp = tempfile::tempdir().unwrap();
        let src = png(tmp.path(), "a.png", 40, 20, [0, 0, 255, 255]);
        let job = JobInput::new(vec![src.clone()])
            .with("epaisseur", "5")
            .with("couleur", "#ff0000");
        let out = image::open(&run("images-bordure", job).unwrap()[0])
            .unwrap()
            .to_rgba8();
        assert_eq!(out.dimensions(), (50, 30));
        assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(25, 15).0, [0, 0, 255, 255]);
        let job = JobInput::new(vec![src]).with("taille", "16");
        let thumb = run("images-vignettes", job).unwrap();
        assert!(thumb[0].ends_with("a-vignette.png"));
        assert_eq!(image::open(&thumb[0]).unwrap().width(), 16);
    }

    #[test]
    fn gif_anime_ordre_et_delai() {
        let tmp = tempfile::tempdir().unwrap();
        let a = png(tmp.path(), "a.png", 20, 10, [255, 0, 0, 255]);
        let b = png(tmp.path(), "b.png", 40, 40, [0, 255, 0, 255]);
        let outs = run(
            "images-assembler-gif",
            JobInput::new(vec![a, b]).with("delai", "200"),
        )
        .unwrap();
        assert!(outs[0].ends_with("a-anime.gif"));
        let dec = GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&outs[0]).unwrap(),
        ))
        .unwrap();
        let frames = dec.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].buffer().dimensions(), (20, 10));
        assert_eq!(frames[0].delay().numer_denom_ms(), (200, 1));
    }

    #[test]
    fn optimiser_web_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let src = png(tmp.path(), "photo.png", 3000, 1000, [10, 20, 30, 255]);
        let job = JobInput::new(vec![src])
            .with("cote_max", "1200")
            .with("format", "jpg");
        let outs = run("images-optimiser-web", job).unwrap();
        assert!(outs[0].ends_with("photo-web.jpg"));
        assert_eq!(image::open(&outs[0]).unwrap().width(), 1200);
    }
}
