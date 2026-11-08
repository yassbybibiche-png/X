//! `pdf-images-vers-pdf` : une image par page A4 (portrait ou paysage selon
//! l'image), centrée et ajustée sans déformation. Pur Rust (image + lopdf).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::DynamicImage;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

use super::images::{flatten_on_white, open};
use super::pdf::save;
use super::percent;
use crate::jobs::ToolContext;
use crate::output;

/// A4 en points (1/72 de pouce).
const A4: (f32, f32) = (595.28, 841.89);

pub fn images_to_pdf(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let mut out = output::beside(&inputs[0], Some("images"), "pdf")?;
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::with_capacity(inputs.len());
    for (i, path) in inputs.iter().enumerate() {
        ctx.check_cancel()?;
        let name = output::file_name(path);
        let pct = percent(i, inputs.len()) * 0.9;
        ctx.reporter.progress(pct, &format!("Ajout de {name}…"));
        let page = add_image_page(&mut doc, pages_id, path).with_context(|| name.clone())?;
        kids.push(Object::Reference(page));
    }
    ctx.reporter.progress(90.0, "Écriture du PDF…");
    let count = kids.len() as i64;
    let pages = dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    save(&mut doc, out.take_file()?)?;
    ctx.reporter
        .progress(100.0, &format!("{count} page(s) créée(s)"));
    Ok(vec![out.commit()])
}

/// Pixels 8 bits + espace de couleur PDF (transparence composée sur blanc).
fn pixels(img: &DynamicImage) -> (Vec<u8>, &'static str) {
    if img.color().has_color() {
        (flatten_on_white(img).to_rgb8().into_raw(), "DeviceRGB")
    } else {
        (flatten_on_white(img).to_luma8().into_raw(), "DeviceGray")
    }
}

/// Page A4 et rectangle (x, y, largeur, hauteur) de l'image ajustée.
pub fn fit_a4(w: u32, h: u32) -> ((f32, f32), [f32; 4]) {
    let page = if w > h { (A4.1, A4.0) } else { A4 };
    let scale = (page.0 / w as f32).min(page.1 / h as f32);
    let (dw, dh) = (w as f32 * scale, h as f32 * scale);
    (page, [(page.0 - dw) / 2.0, (page.1 - dh) / 2.0, dw, dh])
}

fn add_image_page(doc: &mut Document, parent: ObjectId, path: &Path) -> Result<ObjectId> {
    let img = open(path)?;
    let (w, h) = (img.width(), img.height());
    let (data, space) = pixels(&img);
    let xobject = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => i64::from(w), "Height" => i64::from(h),
            "ColorSpace" => space, "BitsPerComponent" => 8,
        },
        data,
    );
    let image_id = doc.add_object(xobject);
    let ((pw, ph), [x, y, dw, dh]) = fit_a4(w, h);
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![dw.into(), 0.into(), 0.into(), dh.into(), x.into(), y.into()],
            ),
            Operation::new("Do", vec!["Im1".into()]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));
    Ok(doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => parent, "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), pw.into(), ph.into()],
        "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image_id } },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{EventKind, JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use image::{Rgba, RgbaImage};

    fn png(path: &Path, w: u32, h: u32) {
        RgbaImage::from_pixel(w, h, Rgba([0, 128, 255, 255]))
            .save(path)
            .unwrap();
    }

    #[test]
    fn ajustement_a4() {
        let ((pw, ph), [x, y, dw, dh]) = fit_a4(200, 100);
        assert!(pw > ph, "paysage attendu");
        assert!((dw - pw).abs() < 0.01 && (dh - pw / 2.0).abs() < 0.01);
        assert!(x.abs() < 0.01 && (y - (ph - dh) / 2.0).abs() < 0.01);
    }

    #[test]
    fn images_vers_pdf_une_page_par_image() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        png(&a, 40, 20);
        png(&b, 10, 30);
        let job = JobInput::new(vec![a, b]);
        let rep = MemoryReporter::default();
        let outs = execute("pdf-images-vers-pdf", job, &rep, &Resolver::default()).unwrap();
        assert_eq!(outs, vec![tmp.path().join("a-images.pdf")]);
        let doc = Document::load(&outs[0]).unwrap();
        assert_eq!(doc.get_pages().len(), 2);
        let img = doc
            .objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .find(|s| s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"))
            .unwrap();
        assert!(img.dict.has(b"Width"));

        // Un PDF d'images n'a pas de couche texte : « PDF vers texte » avertit.
        let rep = MemoryReporter::default();
        let job = JobInput::new(outs);
        execute("pdf-vers-texte", job, &rep, &Resolver::default()).unwrap();
        assert!(rep.count(EventKind::Warning) >= 1);
    }

    #[test]
    fn image_invalide_aucune_sortie() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("faux.png");
        std::fs::write(&bad, b"pas une image").unwrap();
        let job = JobInput::new(vec![bad]);
        let err = execute(
            "pdf-images-vers-pdf",
            job,
            &MemoryReporter::default(),
            &Resolver::default(),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("faux.png"));
        assert!(!tmp.path().join("faux-images.pdf").exists());
    }
}
