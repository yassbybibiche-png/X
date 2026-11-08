//! Outils PDF en pur Rust (lopdf) qui modifient le contenu des pages :
//! filigrane, numérotation, suppression des métadonnées.
//!
//! Le contenu ajouté est dessiné **après** le contenu d'origine, encadré par
//! `q … Q` pour qu'un état graphique laissé ouvert par la page ne le déforme pas.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use super::pdf::{flattened_page, load, save};
use super::per_file;
use crate::jobs::ToolContext;
use crate::output;

/// Noms de ressources propres à l'application (évite toute collision).
const FONT: &str = "BaOF1";
const GSTATE: &str = "BaOGS1";

/// Rectangle visible d'une page (CropBox, sinon MediaBox) : x0, y0, largeur, hauteur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

fn number(obj: &Object) -> Option<f32> {
    match obj {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn page_rect(page: &Dictionary) -> Rect {
    let values: Vec<f32> = [b"CropBox".as_slice(), b"MediaBox"]
        .iter()
        .find_map(|k| page.get(k).ok()?.as_array().ok())
        .map(|a| a.iter().filter_map(number).collect())
        .unwrap_or_default();
    match values[..] {
        [x0, y0, x1, y1] => Rect {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x1 - x0).abs(),
            h: (y1 - y0).abs(),
        },
        _ => Rect {
            x: 0.0,
            y: 0.0,
            w: 595.0,
            h: 842.0,
        },
    }
}

/// Encodage WinAnsi (police standard Helvetica) ; caractères non
/// représentables remplacés par `?` (signalé par `lossy`).
pub fn win_ansi(text: &str) -> (Vec<u8>, bool) {
    let mut lossy = false;
    let bytes = text
        .chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '€' => 0x80,
            '’' => 0x92,
            'œ' => 0x9c,
            'Œ' => 0x8c,
            _ => {
                lossy = true;
                b'?'
            }
        })
        .collect();
    (bytes, lossy)
}

fn dict_of(doc: &Document, obj: Option<&Object>) -> Dictionary {
    match obj {
        Some(Object::Reference(id)) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    }
}

/// Ressources de la page (héritées comprises) complétées de notre police et
/// de notre état graphique.
fn resources_with_ours(doc: &Document, page: &Dictionary, ids: (ObjectId, ObjectId)) -> Dictionary {
    let mut res = dict_of(doc, page.get(b"Resources").ok());
    let mut fonts = dict_of(doc, res.get(b"Font").ok());
    fonts.set(FONT, Object::Reference(ids.0));
    let mut states = dict_of(doc, res.get(b"ExtGState").ok());
    states.set(GSTATE, Object::Reference(ids.1));
    res.set("Font", Object::Dictionary(fonts));
    res.set("ExtGState", Object::Dictionary(states));
    res
}

fn contents_of(page: &Dictionary) -> Vec<Object> {
    match page.get(b"Contents") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(obj @ Object::Reference(_)) => vec![obj.clone()],
        _ => Vec::new(),
    }
}

/// Ajoute à chaque page le contenu produit par `ops(numéro, total, rect)`.
fn stamp_pages(
    doc: &mut Document,
    opacity: f32,
    ops: impl Fn(u32, u32, Rect) -> Vec<Operation>,
) -> Result<()> {
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let state =
        doc.add_object(dictionary! { "Type" => "ExtGState", "ca" => opacity, "CA" => opacity });
    let open = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let pages = doc.get_pages();
    let total = pages.len() as u32;
    for (n, id) in pages {
        let page = flattened_page(doc, id)?;
        let resources = resources_with_ours(doc, &page, (font, state));
        let mut body = vec![Operation::new("Q", vec![]), Operation::new("q", vec![])];
        body.extend(ops(n, total, page_rect(&page)));
        body.push(Operation::new("Q", vec![]));
        let encoded = Content { operations: body }.encode()?;
        let stamp = doc.add_object(Stream::new(dictionary! {}, encoded));
        let mut contents = vec![Object::Reference(open)];
        contents.extend(contents_of(&page));
        contents.push(Object::Reference(stamp));
        let dict = doc.get_dictionary_mut(id)?;
        dict.set("Resources", Object::Dictionary(resources));
        dict.set("Contents", Object::Array(contents));
    }
    Ok(())
}

fn text_ops(text: &[u8], size: f32, matrix: [f32; 6], gray: f32) -> Vec<Operation> {
    vec![
        Operation::new("gs", vec![Object::Name(GSTATE.into())]),
        Operation::new("g", vec![gray.into()]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(FONT.into()), size.into()]),
        Operation::new("Tm", matrix.iter().map(|v| Object::Real(*v)).collect()),
        Operation::new(
            "Tj",
            vec![Object::String(text.to_vec(), lopdf::StringFormat::Literal)],
        ),
        Operation::new("ET", vec![]),
    ]
}

/// Largeur moyenne d'un caractère Helvetica, en fraction de la taille.
const AVG_WIDTH: f32 = 0.55;

/// Filigrane centré, en diagonale, taille adaptée à la page.
pub fn watermark_ops(text: &[u8], r: Rect) -> Vec<Operation> {
    let diagonal = (r.w * r.w + r.h * r.h).sqrt();
    let len = text.len().max(1) as f32;
    let size = (0.7 * diagonal / (AVG_WIDTH * len)).clamp(10.0, 150.0);
    let width = AVG_WIDTH * size * len;
    let (sin, cos) = (r.h.atan2(r.w)).sin_cos();
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let x = cx - cos * width / 2.0 + sin * size * 0.35;
    let y = cy - sin * width / 2.0 - cos * size * 0.35;
    text_ops(text, size, [cos, sin, -sin, cos, x, y], 0.5)
}

fn parse_opacity(value: &str) -> Result<f32> {
    let v: f32 = value
        .trim()
        .parse()
        .ok()
        .filter(|v| (5.0..=100.0).contains(v))
        .with_context(|| format!("opacité invalide : « {value} » (5 à 100 %)"))?;
    Ok(v / 100.0)
}

fn encoded_text(ctx: &ToolContext, text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() {
        bail!("indiquez le texte à ajouter");
    }
    let (bytes, lossy) = win_ansi(text);
    if lossy {
        ctx.reporter
            .warn("Certains caractères ne sont pas disponibles dans la police standard : remplacés par « ? ».");
    }
    Ok(bytes)
}

/// `pdf-filigrane`.
pub fn watermark(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let text = encoded_text(ctx, ctx.input.option("texte"))?;
    let opacity = parse_opacity(ctx.input.option("opacite"))?;
    per_file(ctx, |path| {
        rewrite(path, "filigrane", |doc| {
            stamp_pages(doc, opacity, |_, _, r| watermark_ops(&text, r))
        })
    })
}

/// Libellé du numéro de page selon le format choisi.
pub fn page_label(format: &str, n: u32, total: u32) -> Result<String> {
    Ok(match format {
        "n" => n.to_string(),
        "n-sur-total" => format!("{n} / {total}"),
        "page-n" => format!("Page {n}"),
        other => bail!("format de numérotation inconnu : {other}"),
    })
}

/// Numéro en bas de page (centré ou à droite), 10 pt, noir.
pub fn number_ops(label: &str, r: Rect, position: &str) -> Vec<Operation> {
    let size = 10.0;
    let width = AVG_WIDTH * size * label.len() as f32;
    let x = match position {
        "bas-droite" => r.x + r.w - 36.0 - width,
        _ => r.x + (r.w - width) / 2.0,
    };
    text_ops(
        label.as_bytes(),
        size,
        [1.0, 0.0, 0.0, 1.0, x, r.y + 24.0],
        0.0,
    )
}

/// `pdf-numeroter`.
pub fn number_pages(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let format = ctx.input.option("format").to_string();
    let position = ctx.input.option("position").to_string();
    if !matches!(position.as_str(), "bas-centre" | "bas-droite") {
        bail!("position inconnue : {position}");
    }
    page_label(&format, 1, 1)?;
    per_file(ctx, |path| {
        rewrite(path, "numerote", |doc| {
            stamp_pages(doc, 1.0, |n, total, r| {
                let label = page_label(&format, n, total).unwrap_or_default();
                number_ops(&label, r, &position)
            })
        })
    })
}

/// Charge, transforme, enregistre dans `<nom>-<suffixe>.pdf` (réservé d'abord).
fn rewrite(
    path: &Path,
    suffix: &str,
    edit: impl Fn(&mut Document) -> Result<()>,
) -> Result<PathBuf> {
    let mut out = output::beside(path, Some(suffix), "pdf")?;
    let mut doc = load(path)?;
    edit(&mut doc)?;
    save(&mut doc, out.take_file()?)?;
    Ok(out.commit())
}

/// Clés de métadonnées retirées de tous les objets.
const METADATA_KEYS: [&[u8]; 2] = [b"Metadata", b"PieceInfo"];

/// Retire le dictionnaire /Info, les flux XMP (/Metadata) et /PieceInfo.
pub fn strip_metadata_doc(doc: &mut Document) {
    doc.trailer.remove(b"Info");
    for obj in doc.objects.values_mut() {
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        for key in METADATA_KEYS {
            dict.remove(key);
        }
    }
    doc.prune_objects();
}

/// `pdf-supprimer-metadonnees`.
pub fn strip_metadata(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        rewrite(path, "sans-metadonnees", |doc| {
            strip_metadata_doc(doc);
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use crate::tools::pdf::tests::make_pdf;

    fn run(id: &str, job: JobInput) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), &Resolver::default())
    }

    fn fixture(n: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("doc.pdf");
        make_pdf(&src, "P", n);
        (tmp, src)
    }

    #[test]
    fn encodage_win_ansi() {
        assert_eq!(
            win_ansi("Été €"),
            (vec![0xc9, b't', 0xe9, b' ', 0x80], false)
        );
        assert!(win_ansi("日本").1);
    }

    #[test]
    fn filigrane_sur_chaque_page() {
        let (_tmp, src) = fixture(2);
        let job = JobInput::new(vec![src]).with("texte", "CONFIDENTIEL");
        let outs = run("pdf-filigrane", job).unwrap();
        assert!(outs[0].ends_with("doc-filigrane.pdf"));
        let doc = Document::load(&outs[0]).unwrap();
        for p in 1..=2 {
            let text = doc.extract_text(&[p]).unwrap();
            assert!(
                text.contains("CONFIDENTIEL") && text.contains(&format!("P-{p}")),
                "{text}"
            );
        }
        let bad = JobInput::new(vec![outs[0].clone()]).with("opacite", "0");
        assert!(run("pdf-filigrane", bad).is_err());
    }

    #[test]
    fn numerotation() {
        let (_tmp, src) = fixture(3);
        let job = JobInput::new(vec![src]).with("format", "n-sur-total");
        let outs = run("pdf-numeroter", job).unwrap();
        let doc = Document::load(&outs[0]).unwrap();
        assert!(doc.extract_text(&[2]).unwrap().contains("2 / 3"));
        assert_eq!(page_label("page-n", 4, 9).unwrap(), "Page 4");
        assert!(page_label("romain", 1, 1).is_err());
    }

    #[test]
    fn metadonnees_supprimees() {
        let (_tmp, src) = fixture(1);
        let mut doc = Document::load(&src).unwrap();
        let info = doc.add_object(dictionary! { "Author" => Object::string_literal("Moi") });
        doc.trailer.set("Info", info);
        let xmp = doc.add_object(Stream::new(
            dictionary! {"Type" => "Metadata"},
            b"secret-xmp".to_vec(),
        ));
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(root).unwrap().set("Metadata", xmp);
        doc.save(&src).unwrap();

        let outs = run("pdf-supprimer-metadonnees", JobInput::new(vec![src])).unwrap();
        let clean = Document::load(&outs[0]).unwrap();
        assert!(clean.trailer.get(b"Info").is_err());
        let root = clean.trailer.get(b"Root").unwrap().as_reference().unwrap();
        assert!(!clean.get_dictionary(root).unwrap().has(b"Metadata"));
        let raw = std::fs::read(&outs[0]).unwrap();
        assert!(!raw.windows(3).any(|w| w == b"Moi"));
        assert!(clean.extract_text(&[1]).unwrap().contains("P-1"));
    }
}
