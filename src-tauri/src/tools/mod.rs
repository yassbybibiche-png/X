//! Implémentations des outils. Seuls les outils listés dans [`implementation`]
//! fonctionnent ; les autres renvoient l'erreur « pas encore implémenté ».

pub mod archives;
pub mod archives_extract;
pub mod documents;
pub mod images;
pub mod images_edit;
pub mod media;
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
    ("documents-md-vers-docx", documents::md_to_docx),
    ("documents-md-vers-odt", documents::md_to_odt),
    ("documents-md-vers-html", documents::md_to_html),
    ("documents-md-vers-epub", documents::md_to_epub),
    ("documents-docx-vers-md", documents::docx_to_md),
    ("documents-docx-vers-odt", documents::docx_to_odt),
    ("documents-docx-vers-html", documents::docx_to_html),
    ("documents-odt-vers-docx", documents::odt_to_docx),
    ("documents-odt-vers-md", documents::odt_to_md),
    ("documents-html-vers-md", documents::html_to_md),
    ("documents-html-vers-docx", documents::html_to_docx),
    ("documents-epub-vers-md", documents::epub_to_md),
    ("documents-extraire-texte", documents::extract_text),
    ("images-convertir", images::convert),
    ("images-redimensionner", images_edit::resize),
    ("images-compresser", images_edit::compress),
    ("images-supprimer-exif", images_edit::strip_metadata),
    ("images-pivoter", images_edit::rotate),
    ("images-retourner", images_edit::flip),
    ("images-rogner", images_edit::crop),
    ("images-niveaux-de-gris", images_edit::grayscale),
    ("video-convertir", media::convert_video),
    ("video-compresser", media::compress_video),
    ("video-couper", media::cut),
    ("video-extraire-audio", media::to_audio),
    ("video-extraire-image", media::frame),
    ("video-supprimer-son", media::mute),
    ("video-infos", media::info),
    ("audio-convertir", media::to_audio),
    ("audio-couper", media::cut),
    ("archives-creer-zip", archives::create_zip),
    ("archives-extraire-zip", archives_extract::extract_zip),
    ("archives-creer-tar", archives::create_tar),
    ("archives-creer-targz", archives::create_targz),
    ("archives-extraire-tar", archives_extract::extract_tar),
    ("archives-lister", archives_extract::list),
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
