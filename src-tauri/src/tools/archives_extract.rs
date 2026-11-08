//! Extraction et listage d'archives en pur Rust (ZIP, TAR, TAR.GZ), via
//! [`crate::extract`] (anti « zip slip », liens ignorés, jamais d'écrasement).

use std::cell::Cell;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;

use super::per_file;
use crate::extract::{self, Counting, Hooks, Stats};
use crate::jobs::ToolContext;
use crate::output;

/// Nom de l'archive sans extension(s) : `a.tar.gz`, `a.tgz`, `a.zip` → `a`.
pub fn archive_base(path: &Path) -> String {
    let name = output::file_name(path);
    let lower = name.to_lowercase();
    for ext in [".tar.gz", ".tgz", ".tar", ".zip", ".gz"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    output::stem(path)
}

fn is_gzip(path: &Path) -> Result<bool> {
    let mut magic = [0u8; 2];
    let n = File::open(path)?.read(&mut magic)?;
    Ok(n == 2 && magic == [0x1F, 0x8B])
}

fn hooks<'a>(ctx: &'a ToolContext, warn: &'a dyn Fn(&str), progress: &'a dyn Fn(f32)) -> Hooks<'a> {
    Hooks {
        warn,
        progress,
        cancel: Some(ctx.cancel),
    }
}

/// Réserve `<nom>/` à côté de l'archive puis y extrait avec `run`.
fn extract_into(
    ctx: &ToolContext,
    archive: &Path,
    run: impl Fn(&Path, &Hooks) -> Result<Stats>,
) -> Result<PathBuf> {
    let dir = output::reserve_dir(&output::parent_dir(archive), &archive_base(archive))?;
    let warn = |m: &str| ctx.reporter.warn(m);
    let name = output::file_name(archive);
    let progress = |f: f32| {
        ctx.reporter
            .progress(f * 100.0, &format!("Extraction de {name}…"))
    };
    let stats = run(dir.path(), &hooks(ctx, &warn, &progress))?;
    ctx.reporter.log(&format!(
        "{name} : {} fichier(s), {} dossier(s) extrait(s)",
        stats.files, stats.dirs
    ));
    Ok(dir.commit())
}

/// `archives-extraire-zip`.
pub fn extract_zip(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        extract_into(ctx, path, |dest, hooks| extract::zip(path, dest, 0, hooks))
    })
}

/// `archives-extraire-tar` : .tar, .tar.gz, .tgz (compression détectée au contenu).
pub fn extract_tar(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        extract_into(ctx, path, |dest, hooks| untar(path, dest, hooks))
    })
}

fn untar(path: &Path, dest: &Path, hooks: &Hooks) -> Result<Stats> {
    let total = std::fs::metadata(path)?.len();
    let position = Cell::new(0);
    let counted = Counting::new(BufReader::new(File::open(path)?), &position);
    if is_gzip(path)? {
        extract::tar(GzDecoder::new(counted), dest, 0, hooks, (&position, total))
    } else {
        extract::tar(counted, dest, 0, hooks, (&position, total))
    }
}

/// Ligne de listage : taille, nom (dossiers suffixés de `/`).
pub(crate) struct Item {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// `archives-lister` : `<nom>-contenu.txt`.
pub fn list(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, |path| {
        let mut out = output::beside(path, Some("contenu"), "txt")?;
        let report = if is_builtin(path) {
            render(&output::file_name(path), &list_items(path)?)?
        } else {
            super::archives_7z::list_with_7z(ctx, path)?
        };
        out.take_file()?
            .write_all(report.as_bytes())
            .context("écriture de la liste impossible")?;
        Ok(out.commit())
    })
}

/// Formats listés par le moteur intégré (les autres passent par 7-Zip).
fn is_builtin(path: &Path) -> bool {
    let lower = output::file_name(path).to_lowercase();
    [".zip", ".tar", ".tar.gz", ".tgz"]
        .iter()
        .any(|e| lower.ends_with(e))
}

fn list_items(path: &Path) -> Result<Vec<Item>> {
    let lower = output::file_name(path).to_lowercase();
    if lower.ends_with(".zip") {
        return list_zip(path);
    }
    let file = BufReader::new(File::open(path)?);
    if is_gzip(path)? {
        list_tar(GzDecoder::new(file))
    } else if lower.ends_with(".tar") {
        list_tar(file)
    } else {
        bail!("format non pris en charge en interne (zip, tar, tar.gz, tgz)")
    }
}

fn list_zip(path: &Path) -> Result<Vec<Item>> {
    let mut zip = zip::ZipArchive::new(File::open(path)?).context("archive ZIP illisible")?;
    (0..zip.len())
        .map(|i| {
            let e = zip.by_index_raw(i)?;
            Ok(Item {
                name: e.name().map(|n| n.into_owned()).unwrap_or_default(),
                size: e.size(),
                is_dir: e.is_dir(),
            })
        })
        .collect()
}

fn list_tar<R: Read>(reader: R) -> Result<Vec<Item>> {
    let mut archive = tar::Archive::new(reader);
    let mut items = Vec::new();
    for entry in archive.entries().context("archive TAR illisible")? {
        let entry = entry.context("archive TAR endommagée")?;
        items.push(Item {
            name: entry.path()?.to_string_lossy().into_owned(),
            size: entry.size(),
            is_dir: entry.header().entry_type().is_dir(),
        });
    }
    Ok(items)
}

pub(crate) fn render(archive: &str, items: &[Item]) -> Result<String> {
    let mut r = String::new();
    writeln!(r, "Contenu de {archive}")?;
    writeln!(r)?;
    for item in items {
        let slash = if item.is_dir && !item.name.ends_with('/') {
            "/"
        } else {
            ""
        };
        writeln!(r, "{:>14}  {}{slash}", item.size, item.name)?;
    }
    let files: Vec<&Item> = items.iter().filter(|i| !i.is_dir).collect();
    let total: u64 = files.iter().map(|i| i.size).sum();
    writeln!(r)?;
    writeln!(
        r,
        "{} fichier(s), {total} octets (non compressés)",
        files.len()
    )?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use std::fs;

    fn run(id: &str, inputs: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        execute(
            id,
            JobInput::new(inputs),
            &MemoryReporter::default(),
            &Resolver::default(),
        )
    }

    fn sample(root: &Path) -> PathBuf {
        let dir = root.join("projet");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("LISEZMOI.txt"), "bonjour").unwrap();
        fs::write(dir.join("src").join("main.rs"), "fn main() {}").unwrap();
        dir
    }

    fn roundtrip(create: &str, ext: &str, extract: &str) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sample(tmp.path());
        let archive = run(create, vec![dir.clone()]).unwrap().remove(0);
        assert_eq!(archive, tmp.path().join(format!("projet.{ext}")));
        let out = run(extract, vec![archive.clone()]).unwrap().remove(0);
        // `projet/` existe déjà : jamais d'écrasement → `projet-2/`.
        assert_eq!(out, tmp.path().join("projet-2"));
        let main = out.join("projet").join("src").join("main.rs");
        assert_eq!(fs::read_to_string(main).unwrap(), "fn main() {}");
        let listing = run("archives-lister", vec![archive]).unwrap().remove(0);
        let text = fs::read_to_string(listing).unwrap();
        assert!(text.contains("projet/src/main.rs"), "{text}");
        assert!(text.contains("2 fichier(s), 19 octets"), "{text}");
    }

    #[test]
    fn zip_creer_extraire_lister() {
        roundtrip("archives-creer-zip", "zip", "archives-extraire-zip");
    }

    #[test]
    fn tar_creer_extraire_lister() {
        roundtrip("archives-creer-tar", "tar", "archives-extraire-tar");
    }

    #[test]
    fn targz_creer_extraire_lister() {
        roundtrip("archives-creer-targz", "tar.gz", "archives-extraire-tar");
    }

    #[test]
    fn dates_conservees_dans_le_tar() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sample(tmp.path());
        let f = dir.join("LISEZMOI.txt");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let archive = run("archives-creer-targz", vec![dir]).unwrap().remove(0);
        let out = run("archives-extraire-tar", vec![archive])
            .unwrap()
            .remove(0);
        let mtime = out
            .join("projet/LISEZMOI.txt")
            .metadata()
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime, old);
    }

    #[test]
    fn dates_conservees_dans_le_zip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sample(tmp.path());
        let f = dir.join("LISEZMOI.txt");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let archive = run("archives-creer-zip", vec![dir]).unwrap().remove(0);
        let out = run("archives-extraire-zip", vec![archive])
            .unwrap()
            .remove(0);
        let mtime = out
            .join("projet/LISEZMOI.txt")
            .metadata()
            .unwrap()
            .modified()
            .unwrap();
        let diff = mtime.duration_since(old).unwrap_or_else(|e| e.duration());
        assert!(diff.as_secs() <= 2, "écart {diff:?}");
    }

    #[test]
    fn archive_corrompue_aucun_dossier_residuel() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("faux.zip");
        fs::write(&bad, b"pas un zip").unwrap();
        assert!(run("archives-extraire-zip", vec![bad]).is_err());
        assert!(!tmp.path().join("faux").exists());
    }

    #[test]
    fn noms_de_base() {
        assert_eq!(archive_base(Path::new("/x/a.tar.gz")), "a");
        assert_eq!(archive_base(Path::new("b.TGZ")), "b");
        assert_eq!(archive_base(Path::new("c.zip")), "c");
        assert_eq!(archive_base(Path::new(".zip")), ".zip");
    }
}
