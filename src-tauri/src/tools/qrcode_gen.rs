//! `utilitaires-qrcode` : texte ou lien → PNG (crate qrcode, rendu crate image).
//! Aucun appel réseau : le lien est seulement encodé, jamais ouvert.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use image::{GrayImage, Luma};
use qrcode::{Color, EcLevel, QrCode};

use crate::jobs::ToolContext;
use crate::output;

/// Marge blanche autour du code, en modules (norme : 4).
const QUIET: u32 = 4;

/// Niveau de correction d'erreur choisi dans les options.
pub fn ec_level(value: &str) -> Result<EcLevel> {
    Ok(match value {
        "L" => EcLevel::L,
        "M" => EcLevel::M,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        other => bail!("niveau de correction inconnu : {other}"),
    })
}

/// Image du code : `scale` pixels par module, marge comprise.
pub fn render(text: &str, level: EcLevel, scale: u32) -> Result<GrayImage> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), level).map_err(|e| {
        anyhow::anyhow!("texte impossible à encoder ({e}) : trop long pour un QR code ?")
    })?;
    let width = code.width() as u32;
    let colors = code.to_colors();
    let side = (width + 2 * QUIET) * scale;
    Ok(GrayImage::from_fn(side, side, |x, y| {
        let (mx, my) = (x / scale, y / scale);
        let inside = (QUIET..QUIET + width).contains(&mx) && (QUIET..QUIET + width).contains(&my);
        let dark = inside && colors[((my - QUIET) * width + (mx - QUIET)) as usize] == Color::Dark;
        Luma([if dark { 0 } else { 255 }])
    }))
}

/// Dossier de destination : option `dossier`, sinon Images (ou Téléchargements).
fn destination(ctx: &ToolContext) -> Result<PathBuf> {
    let chosen = ctx.input.option("dossier").trim();
    let dir = if chosen.is_empty() {
        dirs::picture_dir()
            .or_else(dirs::download_dir)
            .or_else(dirs::home_dir)
            .context("dossier Images introuvable")?
    } else {
        PathBuf::from(chosen)
    };
    if !dir.is_dir() {
        bail!("dossier de destination introuvable : {}", dir.display());
    }
    Ok(dir)
}

pub fn generate(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let text = ctx.input.option("texte");
    if text.trim().is_empty() {
        bail!("indiquez le texte ou le lien à encoder");
    }
    let scale: u32 = ctx
        .input
        .option("taille")
        .trim()
        .parse()
        .ok()
        .filter(|v| (1..=50).contains(v))
        .context("taille d'un module invalide (1 à 50 px)")?;
    let img = render(text, ec_level(ctx.input.option("correction"))?, scale)?;
    let mut out = output::reserve(&destination(ctx)?, "qrcode", "png")?;
    let mut file = std::io::BufWriter::new(out.take_file()?);
    img.write_to(&mut file, image::ImageFormat::Png)?;
    drop(file);
    ctx.reporter
        .log(&format!("QR code {}×{} px", img.width(), img.height()));
    Ok(vec![out.commit()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use std::fs;

    #[test]
    fn qrcode_png_sans_ecraser() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().to_string();
        fs::write(tmp.path().join("qrcode.png"), b"existant").unwrap();
        let job = JobInput::new(vec![])
            .with("texte", "https://exemple.fr/é")
            .with("taille", "4")
            .with("dossier", &dir);
        let outs = execute(
            "utilitaires-qrcode",
            job,
            &MemoryReporter::default(),
            &Resolver::default(),
        )
        .unwrap();
        assert_eq!(outs, vec![tmp.path().join("qrcode-2.png")]);
        assert_eq!(
            fs::read(tmp.path().join("qrcode.png")).unwrap(),
            b"existant"
        );
        let img = image::open(&outs[0]).unwrap().to_luma8();
        // Coin = marge blanche ; module (4,4) = coin du motif de repérage, noir.
        assert_eq!(img.get_pixel(0, 0).0, [255]);
        assert_eq!(img.get_pixel(QUIET * 4, QUIET * 4).0, [0]);
        assert_eq!(img.width() % 4, 0);
    }

    #[test]
    fn rendu_et_erreurs() {
        // Version 1 (21 modules) + 2×4 de marge.
        let img = render("A", EcLevel::L, 1).unwrap();
        assert_eq!(img.width(), 29);
        assert!(render(&"x".repeat(5000), EcLevel::H, 1).is_err());
        assert!(ec_level("Z").is_err());
    }
}
