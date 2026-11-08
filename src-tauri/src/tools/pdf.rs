//! Outils PDF en pur Rust (lopdf).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::jobs::ToolContext;
use crate::output;

/// Attributs de page héritables depuis l'arbre /Pages (ISO 32000-1, 7.7.3.4).
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Objets collectés de tous les documents sources (après renumérotation).
#[derive(Default)]
struct Collected {
    pages: Vec<(ObjectId, Dictionary)>,
    objects: Vec<(ObjectId, Object)>,
    max_id: u32,
}

/// `pdf-fusionner` : fusion structurelle, sans re-rendu, dans l'ordre donné.
pub fn merge(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let mut collected = Collected {
        max_id: 1,
        ..Collected::default()
    };
    for (i, path) in inputs.iter().enumerate() {
        let name = output::file_name(path);
        let percent = i as f32 * 80.0 / inputs.len() as f32;
        ctx.reporter
            .progress(percent, &format!("Lecture de {name}…"));
        let pages = collect(path, &mut collected).with_context(|| name.clone())?;
        ctx.reporter.log(&format!("{name} : {pages} page(s)"));
    }
    ctx.reporter.progress(85.0, "Assemblage…");
    let page_count = collected.pages.len();
    let mut doc = assemble(collected)?;
    let mut out = output::beside(&inputs[0], Some("fusionne"), "pdf")?;
    save(&mut doc, out.take_file()?)?;
    ctx.reporter.log(&format!(
        "{page_count} page(s) au total, {} fichier(s) fusionné(s)",
        inputs.len()
    ));
    Ok(vec![out.commit()])
}

pub(crate) fn load(path: &Path) -> Result<Document> {
    let doc = Document::load(path).context("PDF illisible ou endommagé")?;
    if doc.is_encrypted() {
        bail!("PDF protégé par mot de passe : retirez d'abord la protection");
    }
    Ok(doc)
}

/// Charge un PDF, renumérote ses objets et ajoute pages + objets à `acc`.
fn collect(path: &Path, acc: &mut Collected) -> Result<usize> {
    let mut doc = load(path)?;
    doc.renumber_objects_with(acc.max_id);
    acc.max_id = doc.max_id + 1;
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if page_ids.is_empty() {
        bail!("le document ne contient aucune page");
    }
    for id in &page_ids {
        acc.pages.push((*id, flattened_page(&doc, *id)?));
    }
    acc.objects.extend(doc.objects);
    Ok(page_ids.len())
}

/// Copie du dictionnaire de page, complété des attributs hérités de ses parents
/// (sinon ils seraient perdus, l'arbre /Pages d'origine étant remplacé).
pub(crate) fn flattened_page(doc: &Document, id: ObjectId) -> Result<Dictionary> {
    let mut page = doc.get_dictionary(id)?.clone();
    let mut parent = page.get(b"Parent").and_then(Object::as_reference).ok();
    let mut depth = 0;
    while let Some(pid) = parent {
        let node = doc.get_dictionary(pid)?;
        for key in INHERITABLE {
            if !page.has(key)
                && let Ok(value) = node.get(key)
            {
                page.set(key, value.clone());
            }
        }
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
        depth += 1;
        if depth > 64 {
            bail!("arbre de pages invalide (boucle)");
        }
    }
    Ok(page)
}

fn type_name(obj: &Object) -> Option<&[u8]> {
    obj.as_dict().ok()?.get(b"Type").ok()?.as_name().ok()
}

/// Construit le document final : un seul catalogue, un seul arbre /Pages plat.
fn assemble(collected: Collected) -> Result<Document> {
    let mut doc = Document::with_version("1.7");
    let mut catalog = None;
    for (id, obj) in collected.objects {
        match type_name(&obj) {
            Some(b"Catalog") => {
                catalog.get_or_insert((id, obj));
            }
            // Pages et nœuds /Pages sont reconstruits ; signets non fusionnés.
            Some(b"Page" | b"Pages" | b"Outlines" | b"Outline") => {}
            _ => {
                doc.objects.insert(id, obj);
            }
        }
    }
    let (catalog_id, catalog_obj) = catalog.context("catalogue PDF introuvable")?;
    let pages_id = (collected.max_id, 0);
    let kids = insert_pages(&mut doc, collected.pages, pages_id);
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Count", Object::Integer(kids.len() as i64));
    pages.set("Kids", Object::Array(kids));
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    doc.objects
        .insert(catalog_id, new_catalog(catalog_obj, pages_id)?);
    doc.trailer.set("Root", Object::Reference(catalog_id));
    doc.max_id = pages_id.0;
    doc.renumber_objects();
    Ok(doc)
}

fn insert_pages(
    doc: &mut Document,
    pages: Vec<(ObjectId, Dictionary)>,
    parent: ObjectId,
) -> Vec<Object> {
    pages
        .into_iter()
        .map(|(id, mut page)| {
            page.set("Parent", Object::Reference(parent));
            doc.objects.insert(id, Object::Dictionary(page));
            Object::Reference(id)
        })
        .collect()
}

fn new_catalog(obj: Object, pages_id: ObjectId) -> Result<Object> {
    let mut dict = obj.as_dict()?.clone();
    dict.set("Pages", Object::Reference(pages_id));
    for key in [
        &b"Outlines"[..],
        b"PageLabels",
        b"Names",
        b"AcroForm",
        b"StructTreeRoot",
    ] {
        // Propres au premier document : incohérents après fusion.
        dict.remove(key);
    }
    Ok(Object::Dictionary(dict))
}

pub(crate) fn save(doc: &mut Document, file: std::fs::File) -> Result<()> {
    doc.compress();
    let mut writer = BufWriter::new(file);
    doc.save_to(&mut writer)
        .context("écriture du PDF impossible")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use lopdf::content::{Content, Operation};
    use lopdf::{Stream, dictionary};

    /// Génère un PDF de `n` pages ; la page k contient le texte `<label>-<k>`.
    /// Les ressources et la MediaBox sont portées par /Pages (héritage).
    pub fn make_pdf(path: &Path, label: &str, n: usize) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let kids: Vec<Object> = (1..=n)
            .map(|k| {
                let content = Content {
                    operations: vec![
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec!["F1".into(), 24.into()]),
                        Operation::new("Td", vec![100.into(), 600.into()]),
                        Operation::new("Tj", vec![Object::string_literal(format!("{label}-{k}"))]),
                        Operation::new("ET", vec![]),
                    ],
                };
                let stream = Stream::new(dictionary! {}, content.encode().unwrap());
                let content_id = doc.add_object(stream);
                doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
                })
                .into()
            })
            .collect();
        let pages = dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => n as i64,
            "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    fn page_text(doc: &Document, page: u32) -> String {
        doc.extract_text(&[page]).unwrap_or_default()
    }

    fn merge_files(files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        execute(
            "pdf-fusionner",
            JobInput::new(files),
            &MemoryReporter::default(),
            &Resolver::default(),
        )
    }

    #[test]
    fn fusion_ordre_et_nombre_de_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.pdf");
        let b = tmp.path().join("b.pdf");
        make_pdf(&a, "A", 1);
        make_pdf(&b, "B", 2);
        let outs = merge_files(vec![a.clone(), b]).unwrap();
        assert_eq!(outs, vec![tmp.path().join("a-fusionne.pdf")]);
        let merged = Document::load(&outs[0]).unwrap();
        assert_eq!(merged.get_pages().len(), 3);
        assert!(page_text(&merged, 1).contains("A-1"));
        assert!(page_text(&merged, 2).contains("B-1"));
        assert!(page_text(&merged, 3).contains("B-2"));
    }

    #[test]
    fn attributs_herites_conserves() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.pdf");
        make_pdf(&a, "A", 2);
        let outs = merge_files(vec![a.clone(), a]).unwrap();
        let merged = Document::load(&outs[0]).unwrap();
        for id in merged.get_pages().values() {
            let page = merged.get_dictionary(*id).unwrap();
            assert!(page.has(b"MediaBox") && page.has(b"Resources"));
        }
    }

    #[test]
    fn fusion_ne_remplace_pas_une_sortie_existante() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.pdf");
        make_pdf(&a, "A", 1);
        std::fs::write(tmp.path().join("a-fusionne.pdf"), b"existant").unwrap();
        let outs = merge_files(vec![a.clone(), a]).unwrap();
        assert_eq!(outs, vec![tmp.path().join("a-fusionne-2.pdf")]);
        assert_eq!(
            std::fs::read(tmp.path().join("a-fusionne.pdf")).unwrap(),
            b"existant"
        );
    }

    #[test]
    fn pdf_invalide_aucune_sortie_residuelle() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.pdf");
        let bad = tmp.path().join("faux.pdf");
        make_pdf(&a, "A", 1);
        std::fs::write(&bad, b"ceci n'est pas un PDF").unwrap();
        let err = merge_files(vec![a, bad]).unwrap_err();
        assert!(format!("{err:#}").contains("faux.pdf"));
        assert!(!tmp.path().join("a-fusionne.pdf").exists());
    }
}
