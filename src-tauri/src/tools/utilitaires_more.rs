//! Utilitaires en pur Rust : vérifier une empreinte, renommer en lot (copies),
//! rechercher les doublons, découper / réassembler un gros fichier.
//! Aucun fichier existant n'est jamais modifié ni écrasé.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::percent;
use crate::checksum::sha256_file;
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};

/// Empreinte attendue : 64 caractères hexadécimaux, éventuellement suivis
/// d'un nom (format `sha256sum`).
pub fn parse_expected(value: &str) -> Result<String> {
    let first = value.split_whitespace().next().unwrap_or("");
    if first.len() == 64 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(first.to_ascii_lowercase());
    }
    bail!(
        "empreinte attendue invalide : 64 caractères hexadécimaux (copiez-la depuis le site de téléchargement)"
    )
}

/// `utilitaires-verifier-sha256` : verdict dans le journal ; erreur si différent.
pub fn verify_sha256(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let expected = parse_expected(ctx.input.option("attendu"))?;
    let path = &ctx.input.inputs[0];
    ctx.reporter.progress(0.0, "Calcul de l'empreinte…");
    let actual = sha256_file(path)?;
    let name = output::file_name(path);
    if actual != expected {
        bail!(
            "✘ {name} : empreinte DIFFÉRENTE — fichier corrompu ou modifié.\nattendue : {expected}\nobtenue  : {actual}"
        );
    }
    ctx.reporter
        .log(&format!("✔ {name} : empreinte conforme ({actual})"));
    Ok(Vec::new())
}

/// Caractères interdits dans un nom de fichier (Windows compris).
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Nom produit par le motif : `{n}` (numéro sur `digits` chiffres), `{nom}`
/// (nom d'origine), `{date}` (date de modification AAAA-MM-JJ).
pub fn render_pattern(
    pattern: &str,
    n: u64,
    digits: usize,
    original: &str,
    date: &str,
) -> Result<String> {
    let name = pattern
        .replace("{n}", &format!("{n:0digits$}"))
        .replace("{nom}", original)
        .replace("{date}", date);
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains(FORBIDDEN) {
        bail!("motif invalide : « {pattern} » (caractères interdits : / \\ : * ? \" < > |)");
    }
    Ok(trimmed.to_string())
}

fn modified_date(path: &Path) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return "date-inconnue".into();
    };
    let dt = time::OffsetDateTime::from(modified).to_offset(crate::timeutil::local_offset());
    format!(
        "{:04}-{:02}-{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day()
    )
}

fn uint(ctx: &ToolContext, key: &str, (min, max): (u64, u64)) -> Result<u64> {
    let v = ctx.input.option(key);
    v.trim()
        .parse()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .with_context(|| format!("valeur invalide pour « {key} » : « {v} » ({min} à {max})"))
}

fn copy_into(src: &Path, mut out: OutputFile) -> Result<PathBuf> {
    let mut w = BufWriter::new(out.take_file()?);
    std::io::copy(&mut BufReader::new(File::open(src)?), &mut w)?;
    w.flush()?;
    Ok(out.commit())
}

/// `utilitaires-renommer-lot` : copies renommées dans le dossier de chaque
/// original (originaux intacts ; nom pris → `-2`, `-3`…). Ordre = liste.
pub fn batch_rename(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let pattern = ctx.input.option("motif").to_string();
    let start = uint(ctx, "debut", (0, 1_000_000))?;
    let digits = uint(ctx, "chiffres", (1, 9))? as usize;
    if !pattern.contains("{n}") && ctx.input.inputs.len() > 1 {
        ctx.reporter
            .warn("Le motif ne contient pas {n} : les copies seront suffixées -2, -3…");
    }
    let inputs = &ctx.input.inputs;
    let mut outs = Vec::with_capacity(inputs.len());
    for (i, path) in inputs.iter().enumerate() {
        ctx.check_cancel()?;
        ctx.reporter.progress(percent(i, inputs.len()), "Copie…");
        let base = render_pattern(
            &pattern,
            start + i as u64,
            digits,
            &output::stem(path),
            &modified_date(path),
        )?;
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let out = copy_into(
            path,
            output::reserve(&output::parent_dir(path), &base, &ext)?,
        )?;
        ctx.reporter.log(&format!(
            "{} → {}",
            output::file_name(path),
            output::file_name(&out)
        ));
        outs.push(out);
    }
    Ok(outs)
}

/// Fichiers réguliers sous les entrées (dossiers parcourus, liens non suivis).
fn all_files(inputs: &[PathBuf]) -> Vec<PathBuf> {
    inputs
        .iter()
        .flat_map(|p| walkdir::WalkDir::new(p).follow_links(false))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect()
}

/// Groupes de fichiers identiques (même taille, puis même SHA-256).
pub fn duplicate_groups(ctx: &ToolContext, files: &[PathBuf]) -> Result<Vec<(u64, Vec<PathBuf>)>> {
    let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
    for f in files {
        by_size
            .entry(std::fs::metadata(f)?.len())
            .or_default()
            .push(f.clone());
    }
    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size
        .into_iter()
        .filter(|(s, v)| *s > 0 && v.len() > 1)
        .collect();
    let total: usize = candidates.iter().map(|(_, v)| v.len()).sum();
    let mut done = 0;
    let mut groups = Vec::new();
    for (size, paths) in candidates {
        let mut by_hash: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for p in paths {
            ctx.check_cancel()?;
            ctx.reporter
                .progress(percent(done, total), "Comparaison des contenus…");
            done += 1;
            by_hash.entry(sha256_file(&p)?).or_default().push(p);
        }
        groups.extend(
            by_hash
                .into_values()
                .filter(|v| v.len() > 1)
                .map(|v| (size, v)),
        );
    }
    Ok(groups)
}

/// `utilitaires-doublons` : rapport `doublons.txt` (rien n'est supprimé).
pub fn duplicates(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let files = all_files(&ctx.input.inputs);
    ctx.reporter
        .log(&format!("{} fichier(s) examiné(s)", files.len()));
    let groups = duplicate_groups(ctx, &files)?;
    let mut r = String::new();
    writeln!(
        r,
        "Doublons (contenu identique, SHA-256) parmi {} fichier(s)",
        files.len()
    )?;
    let wasted: u64 = groups.iter().map(|(s, v)| s * (v.len() as u64 - 1)).sum();
    writeln!(
        r,
        "{} groupe(s), {wasted} octets récupérables\n",
        groups.len()
    )?;
    for (i, (size, paths)) in groups.iter().enumerate() {
        writeln!(
            r,
            "Groupe {} — {} fichiers de {size} octets :",
            i + 1,
            paths.len()
        )?;
        for p in paths {
            writeln!(r, "  {}", p.display())?;
        }
        writeln!(r)?;
    }
    let mut out = output::reserve(&output::parent_dir(&ctx.input.inputs[0]), "doublons", "txt")?;
    out.take_file()?.write_all(r.as_bytes())?;
    ctx.reporter
        .log(&format!("{} groupe(s) de doublons", groups.len()));
    Ok(vec![out.commit()])
}

const MIB: u64 = 1024 * 1024;

/// Réserve `<base>.001` … `<base>.<count>` (ou `<base>-2.001`… si un nom est
/// pris) : toutes les parties d'une série partagent la même base.
fn reserve_series(dir: &Path, base: &str, count: u64) -> Result<Vec<OutputFile>> {
    for n in 1..=1000u32 {
        let stem = output::candidate(base, "", n);
        let mut parts = Vec::with_capacity(count as usize);
        for k in 1..=count {
            match output::reserve_exact(dir, &format!("{stem}.{k:03}")) {
                Ok(Some(p)) => parts.push(p),
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        if parts.len() as u64 == count {
            return Ok(parts);
        }
    }
    bail!("aucune série de noms libre pour {base}")
}

/// `utilitaires-decouper` : parties de N Mo, `<nom>.001`, `<nom>.002`…
pub fn split_file(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let size = uint(ctx, "taille", (1, 1_000_000))? * MIB;
    let path = &ctx.input.inputs[0];
    let len = std::fs::metadata(path)?.len();
    let count = len.div_ceil(size).max(1);
    if count > 999 {
        bail!("trop de parties ({count}) : augmentez la taille (999 parties au maximum)");
    }
    let mut parts = reserve_series(&output::parent_dir(path), &output::file_name(path), count)?;
    let mut reader = BufReader::new(File::open(path)?);
    for (i, part) in parts.iter_mut().enumerate() {
        ctx.check_cancel()?;
        ctx.reporter.progress(
            percent(i, count as usize),
            &format!("Partie {}/{count}…", i + 1),
        );
        let mut w = BufWriter::new(part.take_file()?);
        std::io::copy(&mut (&mut reader).take(size), &mut w)?;
        w.flush()?;
    }
    ctx.reporter
        .log(&format!("SHA-256 de l'original : {}", sha256_file(path)?));
    Ok(parts.into_iter().map(OutputFile::commit).collect())
}

/// Numéro de partie d'un nom `xxx.NNN`.
fn part_number(path: &Path) -> Option<u32> {
    let ext = path.extension()?.to_str()?;
    (ext.len() == 3).then(|| ext.parse().ok()).flatten()
}

/// Parties à assembler : celles données, ou toute la série si seule `.001` l'est.
/// Base de la série (`film.mkv-2.001` → `film.mkv-2`). Construite par
/// chaînes : `Path::with_extension` prendrait `mkv-2` pour l'extension.
fn series_base(part: &Path) -> String {
    output::stem(part)
}

fn series(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut parts: Vec<PathBuf> = if let [single] = inputs {
        let (dir, base) = (output::parent_dir(single), series_base(single));
        (1..=999)
            .map(|k| dir.join(format!("{base}.{k:03}")))
            .take_while(|p| p.is_file())
            .collect()
    } else {
        inputs.to_vec()
    };
    parts.sort_by_key(|p| part_number(p));
    for (i, p) in parts.iter().enumerate() {
        if part_number(p) != Some(i as u32 + 1) {
            bail!(
                "série incomplète ou mal nommée : partie {:03} attendue, trouvé {}",
                i + 1,
                output::file_name(p)
            );
        }
    }
    if parts.is_empty() {
        bail!("aucune partie .001 trouvée");
    }
    Ok(parts)
}

/// Nom d'origine (base, extension) d'une série : `film.mkv` → (`film`, `mkv`) ;
/// `film.mkv-2` (série renommée pour ne pas écraser) → (`film`, `mkv`).
pub fn original_name(base: &str) -> (String, String) {
    let Some((stem, ext)) = base
        .rsplit_once('.')
        .filter(|(s, e)| !s.is_empty() && !e.is_empty())
    else {
        return (base.to_string(), String::new());
    };
    let ext = match ext.rsplit_once('-') {
        Some((e, n)) if !e.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => e,
        _ => ext,
    };
    (stem.to_string(), ext.to_string())
}

/// `utilitaires-reassembler` : `<nom>.001…` → `<nom>` (ou `<nom>-2.<ext>`).
pub fn join_files(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let parts = series(&ctx.input.inputs)?;
    let (stem, ext) = original_name(&series_base(&parts[0]));
    let mut out = output::reserve(&output::parent_dir(&parts[0]), &stem, &ext)?;
    let mut w = BufWriter::new(out.take_file()?);
    for (i, p) in parts.iter().enumerate() {
        ctx.check_cancel()?;
        ctx.reporter.progress(
            percent(i, parts.len()),
            &format!("Partie {}/{}…", i + 1, parts.len()),
        );
        std::io::copy(&mut BufReader::new(File::open(p)?), &mut w)?;
    }
    w.flush()?;
    drop(w);
    ctx.reporter.log(&format!(
        "{} partie(s) assemblée(s) ; SHA-256 : {}",
        parts.len(),
        sha256_file(out.path())?
    ));
    Ok(vec![out.commit()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use std::fs;

    fn run(id: &str, job: JobInput) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), &Resolver::default())
    }

    #[test]
    fn verifier_une_empreinte() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("abc.txt");
        fs::write(&f, b"abc").unwrap();
        let good = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD  abc.txt";
        assert!(
            run(
                "utilitaires-verifier-sha256",
                JobInput::new(vec![f.clone()]).with("attendu", good)
            )
            .is_ok()
        );
        let bad = "0".repeat(64);
        let err = run(
            "utilitaires-verifier-sha256",
            JobInput::new(vec![f.clone()]).with("attendu", &bad),
        )
        .unwrap_err();
        assert!(err.to_string().contains("DIFFÉRENTE"));
        assert!(parse_expected("abc").is_err());
    }

    #[test]
    fn renommer_en_lot_sans_ecraser() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("IMG_1.jpg");
        let b = tmp.path().join("IMG_2.jpg");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        fs::write(tmp.path().join("vacances-001.jpg"), b"existant").unwrap();
        let job = JobInput::new(vec![a.clone(), b])
            .with("motif", "vacances-{n}")
            .with("debut", "1")
            .with("chiffres", "3");
        let outs = run("utilitaires-renommer-lot", job).unwrap();
        let names: Vec<String> = outs.iter().map(|p| output::file_name(p)).collect();
        assert_eq!(names, ["vacances-001-2.jpg", "vacances-002.jpg"]);
        assert_eq!(
            fs::read(tmp.path().join("vacances-001.jpg")).unwrap(),
            b"existant"
        );
        assert!(a.exists(), "original intact");
        assert!(render_pattern("../x{n}", 1, 2, "a", "d").is_err());
        assert_eq!(
            render_pattern("{date}-{nom}", 1, 2, "a", "2026-11-08").unwrap(),
            "2026-11-08-a"
        );
    }

    #[test]
    fn doublons() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("photos");
        fs::create_dir_all(d.join("sous")).unwrap();
        fs::write(d.join("a.jpg"), b"meme contenu").unwrap();
        fs::write(d.join("sous/copie.jpg"), b"meme contenu").unwrap();
        fs::write(d.join("b.jpg"), b"autre contenu").unwrap();
        fs::write(d.join("c.jpg"), b"Meme contenu").unwrap(); // même taille, contenu différent
        let outs = run("utilitaires-doublons", JobInput::new(vec![d])).unwrap();
        assert_eq!(outs, vec![tmp.path().join("doublons.txt")]);
        let r = fs::read_to_string(&outs[0]).unwrap();
        assert!(r.contains("1 groupe(s), 12 octets"), "{r}");
        assert!(r.contains("copie.jpg") && !r.contains("c.jpg"), "{r}");
    }

    #[test]
    fn decouper_puis_reassembler() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("film.mkv");
        let data: Vec<u8> = (0..(2 * MIB + 123)).map(|i| (i % 251) as u8).collect();
        fs::write(&f, &data).unwrap();
        fs::write(tmp.path().join("film.mkv.002"), b"existant").unwrap();
        let parts = run(
            "utilitaires-decouper",
            JobInput::new(vec![f.clone()]).with("taille", "1"),
        )
        .unwrap();
        let names: Vec<String> = parts.iter().map(|p| output::file_name(p)).collect();
        assert_eq!(
            names,
            ["film.mkv-2.001", "film.mkv-2.002", "film.mkv-2.003"]
        );
        assert_eq!(
            fs::read(tmp.path().join("film.mkv.002")).unwrap(),
            b"existant"
        );

        let joined = run(
            "utilitaires-reassembler",
            JobInput::new(vec![parts[0].clone()]),
        )
        .unwrap();
        assert_eq!(joined, vec![tmp.path().join("film-2.mkv")]);
        assert_eq!(fs::read(&joined[0]).unwrap(), data);
        assert_eq!(
            original_name("archive.tar"),
            ("archive".into(), "tar".into())
        );
        assert_eq!(
            original_name("sans-extension"),
            ("sans-extension".into(), String::new())
        );
        let gap = run(
            "utilitaires-reassembler",
            JobInput::new(vec![parts[0].clone(), parts[2].clone()]),
        );
        assert!(gap.unwrap_err().to_string().contains("incomplète"));
    }
}
