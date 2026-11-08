//! Création d'archives en pur Rust (ZIP, TAR, TAR.GZ).

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::jobs::ToolContext;
use crate::{output, timeutil};

/// Seuil au-delà duquel une entrée ZIP64 est nécessaire.
const ZIP64_THRESHOLD: u64 = u32::MAX as u64;

/// Élément à archiver : chemin sur disque + nom dans l'archive.
#[derive(Debug)]
struct Entry {
    source: PathBuf,
    name: String,
    is_dir: bool,
}

fn zip_options(level: &str) -> Result<SimpleFileOptions> {
    let base = SimpleFileOptions::default();
    Ok(match level {
        "aucune" => base.compression_method(CompressionMethod::Stored),
        "rapide" => base
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(1)),
        "normale" => base.compression_method(CompressionMethod::Deflated),
        "maximale" => base
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9)),
        other => bail!("niveau de compression inconnu : {other}"),
    })
}

/// `archives-creer-zip`.
pub fn create_zip(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let options = zip_options(ctx.input.option("compression"))?;
    let mut out = reserve_archive(inputs, "zip")?;
    ctx.reporter.progress(0.0, "Inventaire des fichiers…");
    let entries = collect_entries(inputs, out.path(), ctx)?;
    let file = out.take_file()?;
    write_zip(file, &entries, options, ctx)?;
    let files = entries.iter().filter(|e| !e.is_dir).count();
    ctx.reporter.log(&format!("{files} fichier(s) archivé(s)"));
    Ok(vec![out.commit()])
}

/// Un seul élément → `<nom>.<ext>` ; plusieurs → `archive.<ext>` (dossier du premier).
fn reserve_archive(inputs: &[PathBuf], ext: &str) -> Result<output::OutputFile> {
    match inputs {
        [single] => output::beside(single, None, ext),
        [first, ..] => output::reserve(&output::parent_dir(first), "archive", ext),
        [] => bail!("aucun fichier à archiver"),
    }
}

/// `archives-creer-tar`.
pub fn create_tar(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    tar_job(ctx, "tar", |file, entries, ctx| {
        let writer = write_tar(BufWriter::new(file), entries, ctx)?;
        writer.into_inner().map_err(|e| e.into_error())?;
        Ok(())
    })
}

/// `archives-creer-targz`.
pub fn create_targz(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    tar_job(ctx, "tar.gz", |file, entries, ctx| {
        let gz = GzEncoder::new(BufWriter::new(file), Compression::default());
        let gz = write_tar(gz, entries, ctx)?;
        gz.finish()?.flush()?;
        Ok(())
    })
}

type TarWrite = fn(File, &[Entry], &ToolContext) -> Result<()>;

fn tar_job(ctx: &ToolContext, ext: &str, write: TarWrite) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let mut out = reserve_archive(inputs, ext)?;
    ctx.reporter.progress(0.0, "Inventaire des fichiers…");
    let entries = collect_entries(inputs, out.path(), ctx)?;
    write(out.take_file()?, &entries, ctx)?;
    let files = entries.iter().filter(|e| !e.is_dir).count();
    ctx.reporter.progress(100.0, "Archive terminée");
    ctx.reporter.log(&format!("{files} fichier(s) archivé(s)"));
    Ok(vec![out.commit()])
}

/// Écrit les entrées (dates et droits rwx conservés, liens déjà exclus).
fn write_tar<W: Write>(writer: W, entries: &[Entry], ctx: &ToolContext) -> Result<W> {
    let mut tar = tar::Builder::new(writer);
    tar.follow_symlinks(false);
    for (i, entry) in entries.iter().enumerate() {
        ctx.check_cancel()?;
        let percent = 5.0 + i as f32 * 90.0 / entries.len().max(1) as f32;
        ctx.reporter
            .progress(percent, &format!("Ajout de {}", entry.name));
        if entry.is_dir {
            tar.append_dir(&entry.name, &entry.source)
        } else {
            tar.append_path_with_name(&entry.source, &entry.name)
        }
        .with_context(|| format!("ajout de {}", entry.name))?;
    }
    tar.into_inner()
        .context("finalisation de l'archive impossible")
}

/// Nom dans l'archive : composants relatifs joints par `/` (jamais absolu, jamais `..`).
fn archive_name(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn collect_entries(
    inputs: &[PathBuf],
    output_path: &Path,
    ctx: &ToolContext,
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for input in inputs {
        let base = output::parent_dir(input);
        for item in WalkDir::new(input).follow_links(false).sort_by_file_name() {
            let item = item.with_context(|| format!("lecture de {}", input.display()))?;
            if let Some(entry) = to_entry(&item, &base, output_path, ctx) {
                if seen.insert(entry.name.clone()) {
                    entries.push(entry);
                } else {
                    ctx.reporter
                        .warn(&format!("Nom en double ignoré : {}", entry.name));
                }
            }
        }
    }
    Ok(entries)
}

fn to_entry(
    item: &walkdir::DirEntry,
    base: &Path,
    output_path: &Path,
    ctx: &ToolContext,
) -> Option<Entry> {
    let path = item.path();
    let kind = item.file_type();
    if kind.is_symlink() {
        ctx.reporter
            .warn(&format!("Lien symbolique ignoré : {}", path.display()));
        return None;
    }
    if path == output_path {
        return None;
    }
    let name = archive_name(path, base)?;
    Some(Entry {
        source: path.to_path_buf(),
        name,
        is_dir: kind.is_dir(),
    })
}

fn write_zip(
    file: File,
    entries: &[Entry],
    options: SimpleFileOptions,
    ctx: &ToolContext,
) -> Result<()> {
    let mut zip = ZipWriter::new(BufWriter::new(file));
    for (i, entry) in entries.iter().enumerate() {
        let percent = 5.0 + i as f32 * 90.0 / entries.len().max(1) as f32;
        ctx.reporter
            .progress(percent, &format!("Ajout de {}", entry.name));
        add_entry(&mut zip, entry, options)?;
    }
    let mut writer = zip.finish().context("finalisation du ZIP impossible")?;
    writer.flush()?;
    Ok(())
}

fn add_entry<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    entry: &Entry,
    options: SimpleFileOptions,
) -> Result<()> {
    if entry.is_dir {
        zip.add_directory(format!("{}/", entry.name), options)?;
        return Ok(());
    }
    let mut source = File::open(&entry.source)
        .with_context(|| format!("lecture de {}", entry.source.display()))?;
    let meta = source.metadata()?;
    let mut options = options.large_file(meta.len() >= ZIP64_THRESHOLD);
    // Date de modification conservée (format DOS, heure locale, précision 2 s).
    if let Some(dt) = meta.modified().ok().and_then(timeutil::zip_datetime) {
        options = options.last_modified_time(dt);
    }
    zip.start_file(entry.name.as_str(), options)?;
    std::io::copy(&mut source, zip).with_context(|| format!("compression de {}", entry.name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{EventKind, JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use std::fs;
    use std::io::Read;

    fn zip_files(inputs: Vec<PathBuf>, level: &str) -> Result<Vec<PathBuf>> {
        let job = JobInput::new(inputs).with("compression", level);
        execute(
            "archives-creer-zip",
            job,
            &MemoryReporter::default(),
            &Resolver::default(),
        )
    }

    fn contents(zip_path: &Path) -> Vec<(String, String)> {
        let mut archive = zip::ZipArchive::new(File::open(zip_path).unwrap()).unwrap();
        let mut out = Vec::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            out.push((f.name().unwrap().to_string(), s));
        }
        out.sort();
        out
    }

    fn fixture(root: &Path) -> (PathBuf, PathBuf) {
        let dir = root.join("dossier");
        fs::create_dir_all(dir.join("sous")).unwrap();
        fs::write(dir.join("a.txt"), "contenu A").unwrap();
        fs::write(dir.join("sous").join("b.txt"), "contenu B ".repeat(100)).unwrap();
        let c = root.join("c.txt");
        fs::write(&c, "contenu C").unwrap();
        (dir, c)
    }

    #[test]
    fn dossier_et_fichier_arborescence_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, c) = fixture(tmp.path());
        let outs = zip_files(vec![dir, c], "normale").unwrap();
        assert_eq!(outs, vec![tmp.path().join("archive.zip")]);
        let names: Vec<(String, String)> = contents(&outs[0]);
        let expected = vec![
            ("c.txt".to_string(), "contenu C".to_string()),
            ("dossier/".to_string(), String::new()),
            ("dossier/a.txt".to_string(), "contenu A".to_string()),
            ("dossier/sous/".to_string(), String::new()),
            ("dossier/sous/b.txt".to_string(), "contenu B ".repeat(100)),
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn un_seul_dossier_porte_son_nom() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, _) = fixture(tmp.path());
        let outs = zip_files(vec![dir], "maximale").unwrap();
        assert_eq!(outs, vec![tmp.path().join("dossier.zip")]);
    }

    #[test]
    fn tous_les_niveaux_et_pas_d_ecrasement() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, c) = fixture(tmp.path());
        fs::write(tmp.path().join("c.zip"), b"existant").unwrap();
        for (i, level) in ["aucune", "rapide", "normale", "maximale"]
            .iter()
            .enumerate()
        {
            let outs = zip_files(vec![c.clone()], level).unwrap();
            assert_eq!(outs, vec![tmp.path().join(format!("c-{}.zip", i + 2))]);
            assert_eq!(
                contents(&outs[0]),
                vec![("c.txt".into(), "contenu C".into())]
            );
        }
        assert_eq!(fs::read(tmp.path().join("c.zip")).unwrap(), b"existant");
        assert!(zip_files(vec![c], "inconnu").is_err());
    }

    #[test]
    fn doublons_signales() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, c) = fixture(tmp.path());
        let rep = MemoryReporter::default();
        let job = JobInput::new(vec![c.clone(), c]).with("compression", "normale");
        let outs = execute("archives-creer-zip", job, &rep, &Resolver::default()).unwrap();
        assert_eq!(contents(&outs[0]).len(), 1);
        assert_eq!(rep.count(EventKind::Warning), 1);
    }

    #[cfg(unix)]
    #[test]
    fn liens_symboliques_ignores() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, _) = fixture(tmp.path());
        std::os::unix::fs::symlink("/etc/hostname", dir.join("lien")).unwrap();
        let outs = zip_files(vec![dir], "normale").unwrap();
        assert!(contents(&outs[0]).iter().all(|(n, _)| !n.contains("lien")));
    }

    #[test]
    fn noms_relatifs_sans_remontee() {
        let base = Path::new("/a/b");
        assert_eq!(
            archive_name(Path::new("/a/b/c/d.txt"), base).as_deref(),
            Some("c/d.txt")
        );
        assert_eq!(archive_name(Path::new("/a/b"), base), None);
        assert_eq!(archive_name(Path::new("/x/y"), base), None);
    }
}
