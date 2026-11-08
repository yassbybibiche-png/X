//! Outils PDF par pages, en pur Rust (lopdf) : découper, extraire, supprimer,
//! pivoter, texte, informations.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lopdf::{Document, Object, ObjectId};

use super::pdf::{flattened_page, load, save};
use super::{per_file, percent};
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};

/// Plage de pages inclusive, numérotée à partir de 1.
pub type Range = (u32, u32);

/// Analyse « 1-3,5,8- » (« -3 » = 1 à 3, « 8- » = 8 à la fin). Vide → aucune plage.
pub fn parse_ranges(spec: &str, total: u32) -> Result<Vec<Range>> {
    spec.split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| parse_range(p, total))
        .collect()
}

fn page_number(s: &str, default: u32) -> Result<u32> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(default);
    }
    s.parse()
        .with_context(|| format!("numéro de page invalide : « {s} »"))
}

fn parse_range(part: &str, total: u32) -> Result<Range> {
    let (a, b) = match part.split_once('-') {
        Some((a, b)) => (page_number(a, 1)?, page_number(b, total)?),
        None => {
            let n = page_number(part, 0)?;
            (n, n)
        }
    };
    if a == 0 || a > b || b > total {
        bail!("plage « {part} » invalide (le document a {total} page(s))");
    }
    Ok((a, b))
}

fn page_set(ranges: &[Range]) -> BTreeSet<u32> {
    ranges.iter().flat_map(|&(a, b)| a..=b).collect()
}

fn page_count(doc: &Document) -> Result<u32> {
    let n = doc.get_pages().len() as u32;
    if n == 0 {
        bail!("le document ne contient aucune page");
    }
    Ok(n)
}

/// Plages obligatoires (option « pages » non vide).
fn required_ranges(ctx: &ToolContext, total: u32) -> Result<BTreeSet<u32>> {
    let ranges = parse_ranges(ctx.input.option("pages"), total)?;
    if ranges.is_empty() {
        bail!("indiquez les pages concernées (ex. 1-3,5)");
    }
    Ok(page_set(&ranges))
}

/// Copie du document ne gardant que les pages `keep` (dans l'ordre du document).
fn keep_pages(doc: &Document, keep: &BTreeSet<u32>) -> Result<Document> {
    let total = page_count(doc)?;
    let remove: Vec<u32> = (1..=total).filter(|n| !keep.contains(n)).collect();
    if remove.len() as u32 == total {
        bail!("aucune page ne resterait dans le document");
    }
    let mut out = doc.clone();
    out.delete_pages(&remove);
    out.prune_objects();
    out.renumber_objects();
    Ok(out)
}

/// Sortie réservée AVANT le travail, écrite puis validée.
fn write_subset(doc: &Document, keep: &BTreeSet<u32>, mut out: OutputFile) -> Result<OutputFile> {
    let mut part = keep_pages(doc, keep)?;
    save(&mut part, out.take_file()?)?;
    Ok(out)
}

fn range_suffix((a, b): Range) -> String {
    if a == b {
        format!("p{a}")
    } else {
        format!("p{a}-{b}")
    }
}

fn open_single<'a>(ctx: &'a ToolContext<'a>) -> Result<(&'a Path, Document, u32)> {
    let input = ctx.input.inputs[0].as_path();
    ctx.reporter
        .progress(0.0, &format!("Lecture de {}…", output::file_name(input)));
    let doc = load(input)?;
    let total = page_count(&doc)?;
    Ok((input, doc, total))
}

/// `pdf-decouper` : un PDF par page, ou un PDF par plage indiquée.
pub fn split(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (input, doc, total) = open_single(ctx)?;
    let mut ranges = parse_ranges(ctx.input.option("pages"), total)?;
    if ranges.is_empty() {
        ranges = (1..=total).map(|n| (n, n)).collect();
    }
    // Les sorties ne sont validées qu'à la fin : annulation ou erreur → rien ne reste.
    let mut pending = Vec::with_capacity(ranges.len());
    for (i, &range) in ranges.iter().enumerate() {
        ctx.check_cancel()?;
        let suffix = range_suffix(range);
        ctx.reporter.progress(
            percent(i, ranges.len()),
            &format!("Création de la partie {suffix}…"),
        );
        let out = output::beside(input, Some(&suffix), "pdf")?;
        pending.push(write_subset(&doc, &page_set(&[range]), out)?);
    }
    ctx.reporter.progress(100.0, "Découpage terminé");
    ctx.reporter
        .log(&format!("{} fichier(s) créé(s)", pending.len()));
    Ok(pending.into_iter().map(OutputFile::commit).collect())
}

/// `pdf-extraire-pages` : copie les pages choisies dans un nouveau PDF.
pub fn extract_pages(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let out = output::beside(&ctx.input.inputs[0], Some("extrait"), "pdf")?;
    let (_, doc, total) = open_single(ctx)?;
    let keep = required_ranges(ctx, total)?;
    ctx.reporter
        .progress(50.0, &format!("Extraction de {} page(s)…", keep.len()));
    let out = write_subset(&doc, &keep, out)?;
    ctx.reporter.progress(100.0, "Extraction terminée");
    Ok(vec![out.commit()])
}

/// `pdf-supprimer-pages` : retire les pages choisies.
pub fn delete_pages(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let out = output::beside(&ctx.input.inputs[0], Some("allege"), "pdf")?;
    let (_, doc, total) = open_single(ctx)?;
    let removed = required_ranges(ctx, total)?;
    let keep: BTreeSet<u32> = (1..=total).filter(|n| !removed.contains(n)).collect();
    ctx.reporter
        .progress(50.0, &format!("Suppression de {} page(s)…", removed.len()));
    let out = write_subset(&doc, &keep, out)?;
    ctx.reporter.progress(100.0, "Suppression terminée");
    ctx.reporter
        .log(&format!("{} page(s) conservée(s) sur {total}", keep.len()));
    Ok(vec![out.commit()])
}

fn parse_angle(value: &str) -> Result<i64> {
    match value.trim() {
        "90" => Ok(90),
        "180" => Ok(180),
        "270" => Ok(270),
        other => bail!("angle invalide : {other} (90, 180 ou 270)"),
    }
}

/// `pdf-pivoter` : ajoute l'angle à la rotation (héritée comprise) des pages.
pub fn rotate(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let angle = parse_angle(ctx.input.option("angle"))?;
    let spec = ctx.input.option("pages");
    per_file(ctx, |path| rotate_one(path, angle, spec))
}

fn rotate_one(path: &Path, angle: i64, spec: &str) -> Result<PathBuf> {
    let mut out = output::beside(path, Some("pivote"), "pdf")?;
    let mut doc = load(path)?;
    let pages = doc.get_pages();
    let selected = page_set(&parse_ranges(spec, pages.len() as u32)?);
    for (n, id) in pages {
        if selected.is_empty() || selected.contains(&n) {
            rotate_page(&mut doc, id, angle)?;
        }
    }
    save(&mut doc, out.take_file()?)?;
    Ok(out.commit())
}

fn rotate_page(doc: &mut Document, id: ObjectId, angle: i64) -> Result<()> {
    let current = flattened_page(doc, id)?
        .get(b"Rotate")
        .and_then(Object::as_i64)
        .unwrap_or(0);
    let page = doc.get_dictionary_mut(id)?;
    page.set("Rotate", Object::Integer((current + angle).rem_euclid(360)));
    Ok(())
}

/// `pdf-vers-texte` : texte de chaque page, pages séparées par une ligne vide.
/// Un PDF scanné (sans couche texte) donne un avertissement, pas un faux succès.
pub fn to_text(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| to_text_one(ctx, path))
}

fn to_text_one(ctx: &ToolContext, path: &Path) -> Result<PathBuf> {
    let mut out = output::beside(path, None, "txt")?;
    let doc = load(path)?;
    let pages: Vec<u32> = doc.get_pages().into_keys().collect();
    let mut text = String::new();
    for n in &pages {
        ctx.check_cancel()?;
        match doc.extract_text(&[*n]) {
            Ok(t) => text.push_str(t.trim_end()),
            Err(e) => ctx
                .reporter
                .warn(&format!("page {n} : texte illisible ({e})")),
        }
        text.push_str("\n\n");
    }
    if text.trim().is_empty() {
        ctx.reporter.warn(&format!(
            "{} : aucun texte trouvé (PDF scanné ? utilisez l'OCR)",
            output::file_name(path)
        ));
    }
    std::io::Write::write_all(&mut out.take_file()?, text.as_bytes())
        .context("écriture du texte impossible")?;
    Ok(out.commit())
}

/// `pdf-infos` : rapport texte (pages, version, format, métadonnées).
pub fn info(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, info_one)
}

fn info_one(path: &Path) -> Result<PathBuf> {
    let mut out = output::beside(path, Some("infos"), "txt")?;
    let report = info_report(path)?;
    std::io::Write::write_all(&mut out.take_file()?, report.as_bytes())
        .context("écriture du rapport impossible")?;
    Ok(out.commit())
}

const INFO_KEYS: [(&[u8], &str); 8] = [
    (b"Title", "Titre"),
    (b"Author", "Auteur"),
    (b"Subject", "Sujet"),
    (b"Keywords", "Mots-clés"),
    (b"Creator", "Créé avec"),
    (b"Producer", "Producteur"),
    (b"CreationDate", "Création"),
    (b"ModDate", "Modification"),
];

pub fn info_report(path: &Path) -> Result<String> {
    let doc = Document::load(path).context("PDF illisible ou endommagé")?;
    let size = std::fs::metadata(path)?.len();
    let pages = doc.get_pages();
    let mut r = String::new();
    writeln!(r, "Fichier : {}", output::file_name(path))?;
    writeln!(r, "Taille : {size} octets")?;
    writeln!(r, "Version PDF : {}", doc.version)?;
    writeln!(r, "Pages : {}", pages.len())?;
    let chiffre = if doc.is_encrypted() { "oui" } else { "non" };
    writeln!(r, "Chiffré : {chiffre}")?;
    if let Some(id) = pages.values().next() {
        writeln!(r, "Format de la page 1 : {}", page_format(&doc, *id))?;
    }
    for (key, label) in INFO_KEYS {
        if let Some(value) = info_value(&doc, key) {
            writeln!(r, "{label} : {value}")?;
        }
    }
    Ok(r)
}

fn info_value(doc: &Document, key: &[u8]) -> Option<String> {
    let info = doc.trailer.get(b"Info").ok()?;
    let dict = match info {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        other => other.as_dict().ok()?,
    };
    let value = lopdf::decode_text_string(dict.get(key).ok()?).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn number(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

fn page_format(doc: &Document, id: ObjectId) -> String {
    let media = flattened_page(doc, id)
        .ok()
        .and_then(|p| p.get(b"MediaBox").ok().cloned());
    let values: Vec<f64> = media
        .as_ref()
        .and_then(|m| m.as_array().ok())
        .map(|a| a.iter().filter_map(number).collect())
        .unwrap_or_default();
    match values[..] {
        [x0, y0, x1, y1] => {
            let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
            let mm = |pt: f64| (pt * 25.4 / 72.0).round();
            format!("{w:.0} × {h:.0} pt ({} × {} mm)", mm(w), mm(h))
        }
        _ => "inconnu".to_string(),
    }
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

    fn texts(path: &Path) -> Vec<String> {
        let doc = Document::load(path).unwrap();
        let n = doc.get_pages().len() as u32;
        (1..=n)
            .map(|p| doc.extract_text(&[p]).unwrap().trim().to_string())
            .collect()
    }

    fn fixture(n: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("doc.pdf");
        make_pdf(&src, "P", n);
        (tmp, src)
    }

    #[test]
    fn plages_de_pages() {
        assert_eq!(parse_ranges("1-3, 5", 9).unwrap(), vec![(1, 3), (5, 5)]);
        assert_eq!(parse_ranges("-2;8-", 9).unwrap(), vec![(1, 2), (8, 9)]);
        assert!(parse_ranges("", 3).unwrap().is_empty());
        for bad in ["0", "4", "3-2", "a", "1-x"] {
            assert!(parse_ranges(bad, 3).is_err(), "{bad}");
        }
    }

    #[test]
    fn decouper_une_page_par_fichier() {
        let (tmp, src) = fixture(3);
        let outs = run("pdf-decouper", JobInput::new(vec![src])).unwrap();
        let names: Vec<_> = outs.iter().map(|p| output::file_name(p)).collect();
        assert_eq!(names, ["doc-p1.pdf", "doc-p2.pdf", "doc-p3.pdf"]);
        assert_eq!(texts(&tmp.path().join("doc-p2.pdf")), ["P-2"]);
    }

    #[test]
    fn decouper_par_plages() {
        let (_tmp, src) = fixture(4);
        let job = JobInput::new(vec![src]).with("pages", "1-2,4");
        let outs = run("pdf-decouper", job).unwrap();
        assert_eq!(texts(&outs[0]), ["P-1", "P-2"]);
        assert_eq!(texts(&outs[1]), ["P-4"]);
        assert!(outs[0].ends_with("doc-p1-2.pdf"));
    }

    #[test]
    fn decouper_plage_invalide_aucune_sortie() {
        let (tmp, src) = fixture(2);
        let job = JobInput::new(vec![src]).with("pages", "1,5");
        assert!(run("pdf-decouper", job).is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn extraire_pages() {
        let (_tmp, src) = fixture(5);
        let job = JobInput::new(vec![src]).with("pages", "2,4-5");
        let outs = run("pdf-extraire-pages", job).unwrap();
        assert!(outs[0].ends_with("doc-extrait.pdf"));
        assert_eq!(texts(&outs[0]), ["P-2", "P-4", "P-5"]);
    }

    #[test]
    fn extraire_sans_pages_refuse() {
        let (tmp, src) = fixture(2);
        let err = run("pdf-extraire-pages", JobInput::new(vec![src])).unwrap_err();
        assert!(err.to_string().contains("indiquez les pages"));
        assert!(!tmp.path().join("doc-extrait.pdf").exists());
    }

    #[test]
    fn supprimer_pages() {
        let (_tmp, src) = fixture(4);
        let job = JobInput::new(vec![src.clone()]).with("pages", "1,3");
        let outs = run("pdf-supprimer-pages", job).unwrap();
        assert_eq!(texts(&outs[0]), ["P-2", "P-4"]);
        let all = JobInput::new(vec![src]).with("pages", "1-4");
        assert!(run("pdf-supprimer-pages", all).is_err());
    }

    fn rotation(doc: &Document, page: u32) -> i64 {
        let id = doc.get_pages()[&page];
        flattened_page(doc, id)
            .unwrap()
            .get(b"Rotate")
            .and_then(Object::as_i64)
            .unwrap_or(0)
    }

    #[test]
    fn pivoter_certaines_pages_cumule_l_angle() {
        let (_tmp, src) = fixture(3);
        let job = JobInput::new(vec![src])
            .with("angle", "270")
            .with("pages", "2-3");
        let outs = run("pdf-pivoter", job).unwrap();
        assert!(outs[0].ends_with("doc-pivote.pdf"));
        let job = JobInput::new(outs.clone()).with("angle", "180");
        let twice = run("pdf-pivoter", job).unwrap();
        let doc = Document::load(&twice[0]).unwrap();
        assert_eq!(
            [rotation(&doc, 1), rotation(&doc, 2), rotation(&doc, 3)],
            [180, 90, 90]
        );
    }

    #[test]
    fn pivoter_angle_invalide() {
        let (_tmp, src) = fixture(1);
        let job = JobInput::new(vec![src]).with("angle", "45");
        assert!(run("pdf-pivoter", job).is_err());
    }

    #[test]
    fn pdf_vers_texte() {
        let (tmp, src) = fixture(2);
        let outs = run("pdf-vers-texte", JobInput::new(vec![src])).unwrap();
        assert_eq!(outs, vec![tmp.path().join("doc.txt")]);
        let text = std::fs::read_to_string(&outs[0]).unwrap();
        assert!(text.contains("P-1") && text.contains("P-2"), "{text}");
    }

    #[test]
    fn infos_pdf() {
        let (_tmp, src) = fixture(3);
        let outs = run("pdf-infos", JobInput::new(vec![src])).unwrap();
        assert!(outs[0].ends_with("doc-infos.txt"));
        let report = std::fs::read_to_string(&outs[0]).unwrap();
        assert!(report.contains("Pages : 3"), "{report}");
        assert!(report.contains("595 × 842 pt (210 × 297 mm)"), "{report}");
        assert!(report.contains("Chiffré : non"));
    }
}
