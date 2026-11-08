//! Implémentations des outils. Seuls les outils listés dans [`implementation`]
//! fonctionnent ; les autres renvoient l'erreur « pas encore implémenté ».

pub mod archives;
pub mod images;
pub mod images_edit;
pub mod pdf;
pub mod pdf_images;
pub mod pdf_pages;
pub mod utilitaires;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::jobs::ToolContext;
use crate::output;

pub type ToolFn = fn(&ToolContext) -> Result<Vec<PathBuf>>;

/// Outils réellement implémentés (id → fonction).
const IMPLEMENTED: &[(&str, ToolFn)] = &[
    ("pdf-fusionner", pdf::merge),
    ("pdf-decouper", pdf_pages::split),
    ("pdf-extraire-pages", pdf_pages::extract_pages),
    ("pdf-supprimer-pages", pdf_pages::delete_pages),
    ("pdf-pivoter", pdf_pages::rotate),
    ("pdf-vers-texte", pdf_pages::to_text),
    ("pdf-infos", pdf_pages::info),
    ("pdf-images-vers-pdf", pdf_images::images_to_pdf),
    ("images-convertir", images::convert),
    ("images-redimensionner", images_edit::resize),
    ("images-compresser", images_edit::compress),
    ("images-supprimer-exif", images_edit::strip_metadata),
    ("images-pivoter", images_edit::rotate),
    ("images-retourner", images_edit::flip),
    ("images-rogner", images_edit::crop),
    ("images-niveaux-de-gris", images_edit::grayscale),
    ("archives-creer-zip", archives::create_zip),
    ("utilitaires-empreinte-sha256", utilitaires::hash_files),
];

pub fn implementation(id: &str) -> Option<ToolFn> {
    IMPLEMENTED.iter().find(|(k, _)| *k == id).map(|(_, f)| *f)
}

pub(crate) fn percent(done: usize, total: usize) -> f32 {
    if total == 0 {
        return 100.0;
    }
    done as f32 * 100.0 / total as f32
}

/// Applique `f` à chaque fichier d'entrée, avec progression et journal.
/// Les échecs individuels sont signalés et n'arrêtent pas le lot ; erreur
/// seulement si aucun fichier n'a pu être traité.
pub fn per_file(ctx: &ToolContext, f: impl Fn(&Path) -> Result<PathBuf>) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let mut outputs = Vec::new();
    let mut last_error = None;
    for (i, path) in inputs.iter().enumerate() {
        let name = output::file_name(path);
        ctx.reporter
            .progress(percent(i, inputs.len()), &format!("Traitement de {name}…"));
        match f(path) {
            Ok(out) => {
                ctx.reporter
                    .log(&format!("{name} → {}", output::file_name(&out)));
                outputs.push(out);
            }
            Err(e) => {
                ctx.reporter.warn(&format!("Échec pour {name} : {e:#}"));
                last_error = Some(e);
            }
        }
    }
    ctx.reporter.progress(100.0, "Traitement terminé");
    finish(outputs, last_error, inputs.len())
}

fn finish(
    outputs: Vec<PathBuf>,
    last_error: Option<anyhow::Error>,
    total: usize,
) -> Result<Vec<PathBuf>> {
    match last_error {
        Some(e) if outputs.is_empty() && total == 1 => Err(e),
        Some(_) if outputs.is_empty() => {
            bail!("Aucun fichier n'a pu être traité (voir le journal).")
        }
        _ => Ok(outputs),
    }
}
