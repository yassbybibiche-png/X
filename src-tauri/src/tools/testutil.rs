//! Aides de test : vrais binaires (PATH ou `.scratch/appdata/bin`, rempli par
//! `cargo run --example installer`) et fixtures PDF générées.
//!
//! Les tests « réels » sont ignorés proprement si le binaire est absent (CI).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

use crate::sidecar::{self, Binary, Resolver};

/// Dossier de binaires téléchargés par l'application pendant le développement.
pub fn scratch_bin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../.scratch/appdata/bin")
}

/// Résolveur réel si tous les `binaries` sont présents, sinon `None` (+ message).
pub fn real(binaries: &[Binary]) -> Option<Resolver> {
    let resolver = Resolver::new(None, Some(scratch_bin_dir()), std::env::var_os("PATH"));
    let missing: Vec<&str> = binaries
        .iter()
        .filter(|b| resolver.resolve(**b).is_none())
        .map(|b| b.label())
        .collect();
    if missing.is_empty() {
        return Some(resolver);
    }
    eprintln!(
        "binaire(s) absent(s) : {} — test réel ignoré",
        missing.join(", ")
    );
    None
}

/// Faux binaire : script shell exécutable (tests de câblage, Unix seulement).
#[cfg(unix)]
pub fn fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn save(mut doc: Document, pages: Vec<Object>, pages_id: lopdf::ObjectId, path: &Path) {
    let count = pages.len() as i64;
    let tree = dictionary! {
        "Type" => "Pages", "Kids" => pages, "Count" => count,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(tree));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

/// PDF de 2 pages en couleur avec une grande image non compressée (dégradé
/// 600×600) : se compresse nettement avec Ghostscript.
pub fn scratch_pdf(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let pixels: Vec<u8> = (0..600u32 * 600)
        .flat_map(|i| [(i % 600 / 3) as u8, (i / 600 / 3) as u8, 200])
        .collect();
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 600, "Height" => 600,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        pixels,
    );
    let image_id = doc.add_object(image);
    let ops = "1 0 0 rg 50 700 200 100 re f q 300 0 0 300 50 300 cm /Im1 Do Q";
    let kids = (0..2)
        .map(|_| {
            let content = doc.add_object(Stream::new(dictionary! {}, ops.as_bytes().to_vec()));
            let res = dictionary! { "XObject" => dictionary! { "Im1" => image_id } };
            doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => res,
            })
            .into()
        })
        .collect();
    save(doc, kids, pages_id, path);
}

/// PDF d'une page avec `text` en gros caractères (Helvetica 40 pt).
pub fn big_text_pdf(path: &Path, text: &str) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 40.into()]),
            Operation::new("Td", vec![60.into(), 600.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let res = dictionary! { "Font" => dictionary! { "F1" => font } };
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => res,
    });
    save(doc, vec![page.into()], pages_id, path);
}

/// Texte d'un PDF extrait par Ghostscript (`txtwrite`), toutes pages.
pub fn gs_text(resolver: &Resolver, pdf: &Path) -> String {
    let gs = resolver.resolve(Binary::Ghostscript).unwrap().path;
    let mut args: Vec<OsString> = ["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=txtwrite"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push("-sOutputFile=-".into());
    args.push(pdf.into());
    sidecar::run(&gs, &args, Duration::from_secs(60))
        .unwrap()
        .check("gs")
        .unwrap()
        .stdout
}
