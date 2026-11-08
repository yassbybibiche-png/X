//! Extraction sûre d'archives ZIP et TAR (outils + installation des binaires).
//!
//! - chemins absolus, `..` et préfixes Windows refusés (« zip slip ») ;
//! - liens symboliques, liens physiques et fichiers spéciaux ignorés ;
//! - jamais d'écrasement : un fichier déjà extrait n'est pas remplacé ;
//! - dates de modification restaurées ; bits setuid/setgid jamais appliqués.

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::cancel::CancelToken;

/// Rappels pendant l'extraction.
pub struct Hooks<'a> {
    pub warn: &'a dyn Fn(&str),
    /// Avancement entre 0 et 1.
    pub progress: &'a dyn Fn(f32),
    pub cancel: Option<&'a CancelToken>,
}

impl Hooks<'_> {
    fn check(&self) -> Result<()> {
        if let Some(c) = self.cancel {
            c.check()?;
        }
        Ok(())
    }
}

/// Chemin relatif sûr après retrait de `strip` composants ; `None` si dangereux
/// (absolu, `..`) ou vide.
pub fn safe_relative(path: &Path, strip: usize) -> Option<PathBuf> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => parts.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let rel: PathBuf = parts.into_iter().skip(strip).collect();
    (!rel.as_os_str().is_empty()).then_some(rel)
}

/// Nombre de fichiers et dossiers créés.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    pub dirs: usize,
}

/// Extrait un ZIP dans `dest` (qui doit exister).
pub fn zip(archive: &Path, dest: &Path, strip: usize, hooks: &Hooks) -> Result<Stats> {
    let file = File::open(archive).with_context(|| format!("lecture de {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("archive ZIP illisible ou endommagée")?;
    let mut stats = Stats::default();
    let total = zip.len().max(1);
    for i in 0..zip.len() {
        hooks.check()?;
        (hooks.progress)(i as f32 / total as f32);
        let mut entry = zip.by_index(i)?;
        zip_entry(&mut entry, dest, strip, hooks, &mut stats)?;
    }
    Ok(stats)
}

fn zip_entry<R: Read>(
    entry: &mut zip::read::ZipFile<R>,
    dest: &Path,
    strip: usize,
    hooks: &Hooks,
    stats: &mut Stats,
) -> Result<()> {
    let raw_name = entry
        .name()
        .map(|n| n.into_owned())
        .unwrap_or_else(|_| "?".into());
    let Some(rel) = entry.enclosed_name().and_then(|n| safe_relative(&n, strip)) else {
        if strip == 0 {
            (hooks.warn)(&format!("Chemin dangereux ignoré : {raw_name}"));
        }
        return Ok(());
    };
    if entry.is_symlink() {
        (hooks.warn)(&format!("Lien symbolique ignoré : {raw_name}"));
        return Ok(());
    }
    let target = dest.join(&rel);
    if entry.is_dir() {
        fs::create_dir_all(&target)?;
        stats.dirs += 1;
        return Ok(());
    }
    let mtime = zip_mtime(entry);
    let mode = entry.unix_mode();
    if write_new(&target, entry, hooks)? {
        finish_file(&target, mtime, mode);
        stats.files += 1;
    }
    Ok(())
}

/// Date de modification : champ « extended timestamp » (UTC) sinon date DOS (locale).
fn zip_mtime<R: Read>(entry: &zip::read::ZipFile<R>) -> Option<SystemTime> {
    let extended = entry.extra_data_fields().find_map(|f| match f {
        zip::extra_fields::ExtraField::ExtendedTimestamp(ts) => ts.mod_time(),
        _ => None,
    });
    if let Some(secs) = extended {
        return Some(UNIX_EPOCH + Duration::from_secs(u64::from(secs)));
    }
    let dos = entry.last_modified()?;
    let local = time::PrimitiveDateTime::try_from(dos).ok()?;
    let at = local.assume_offset(crate::timeutil::local_offset());
    Some(SystemTime::from(at))
}

/// Crée `target` (jamais d'écrasement) et y copie `reader`. `false` si déjà présent.
fn write_new(target: &Path, reader: &mut dyn Read, hooks: &Hooks) -> Result<bool> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = match OpenOptions::new().write(true).create_new(true).open(target) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            (hooks.warn)(&format!("Doublon ignoré : {}", target.display()));
            return Ok(false);
        }
        Err(e) => return Err(e).with_context(|| format!("création de {}", target.display())),
    };
    io::copy(reader, &mut out).with_context(|| format!("écriture de {}", target.display()))?;
    Ok(true)
}

fn finish_file(target: &Path, mtime: Option<SystemTime>, mode: Option<u32>) {
    if let Some(t) = mtime
        && let Ok(f) = OpenOptions::new().write(true).open(target)
    {
        let _ = f.set_modified(t);
    }
    set_mode(target, mode);
}

#[cfg(unix)]
fn set_mode(target: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode {
        // Jamais de setuid/setgid/sticky : seulement rwx.
        let _ = fs::set_permissions(target, fs::Permissions::from_mode(m & 0o777));
    }
}

#[cfg(not(unix))]
fn set_mode(_target: &Path, _mode: Option<u32>) {}

/// Lecteur qui compte les octets lus (avancement réel des archives compressées).
pub struct Counting<'a, R> {
    inner: R,
    count: &'a Cell<u64>,
}

impl<'a, R> Counting<'a, R> {
    pub fn new(inner: R, count: &'a Cell<u64>) -> Self {
        Self { inner, count }
    }
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.set(self.count.get() + n as u64);
        Ok(n)
    }
}

/// Extrait un flux TAR (déjà décompressé) dans `dest`.
/// `position` = octets consommés du fichier source, `total` = sa taille.
pub fn tar<R: Read>(
    reader: R,
    dest: &Path,
    strip: usize,
    hooks: &Hooks,
    progress: (&Cell<u64>, u64),
) -> Result<Stats> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_mtime(true);
    archive.set_preserve_permissions(false);
    archive.set_unpack_xattrs(false);
    let mut stats = Stats::default();
    for entry in archive.entries().context("archive TAR illisible")? {
        hooks.check()?;
        let (position, total) = progress;
        (hooks.progress)(position.get() as f32 / total.max(1) as f32);
        tar_entry(
            entry.context("archive TAR endommagée")?,
            dest,
            strip,
            hooks,
            &mut stats,
        )?;
    }
    Ok(stats)
}

fn tar_entry<R: Read>(
    mut entry: tar::Entry<R>,
    dest: &Path,
    strip: usize,
    hooks: &Hooks,
    stats: &mut Stats,
) -> Result<()> {
    let path = entry.path()?.into_owned();
    let kind = entry.header().entry_type();
    if !(kind.is_file() || kind.is_dir()) {
        (hooks.warn)(&format!(
            "Élément ignoré (lien ou fichier spécial) : {}",
            path.display()
        ));
        return Ok(());
    }
    let Some(rel) = safe_relative(&path, strip) else {
        if strip == 0 || path.components().count() > strip {
            (hooks.warn)(&format!("Chemin dangereux ignoré : {}", path.display()));
        }
        return Ok(());
    };
    let target = dest.join(rel);
    if kind.is_dir() {
        fs::create_dir_all(&target)?;
        stats.dirs += 1;
        return Ok(());
    }
    if target.exists() {
        (hooks.warn)(&format!("Doublon ignoré : {}", target.display()));
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    entry
        .unpack(&target)
        .with_context(|| format!("écriture de {}", target.display()))?;
    stats.files += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn hooks<'a>(warn: &'a dyn Fn(&str)) -> Hooks<'a> {
        Hooks {
            warn,
            progress: &|_| {},
            cancel: None,
        }
    }

    #[test]
    fn chemins_surs() {
        assert_eq!(
            safe_relative(Path::new("a/./b.txt"), 0),
            Some(PathBuf::from("a/b.txt"))
        );
        assert_eq!(
            safe_relative(Path::new("pandoc-3/bin/pandoc"), 1),
            Some(PathBuf::from("bin/pandoc"))
        );
        assert_eq!(safe_relative(Path::new("../x"), 0), None);
        assert_eq!(safe_relative(Path::new("a/../../x"), 0), None);
        assert_eq!(safe_relative(Path::new("/etc/passwd"), 0), None);
        assert_eq!(safe_relative(Path::new("seul"), 1), None);
    }

    /// ZIP piégé écrit à la main : `../evasion.txt` et `/abs.txt` doivent être refusés.
    #[test]
    fn zip_slip_refuse() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("piege.zip");
        let mut w = zip::ZipWriter::new(File::create(&archive).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for name in ["../evasion.txt", "/abs.txt", "ok/bon.txt"] {
            w.start_file(name, opts).unwrap();
            w.write_all(b"x").unwrap();
        }
        w.finish().unwrap();
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let warnings = std::sync::Mutex::new(Vec::new());
        let warn = |m: &str| warnings.lock().unwrap().push(m.to_string());
        let stats = zip(&archive, &dest, 0, &hooks(&warn)).unwrap();
        assert!(dest.join("ok/bon.txt").is_file());
        assert!(!tmp.path().join("evasion.txt").exists());
        assert!(stats.files >= 1);
        assert!(!Path::new("/abs.txt").exists());
    }

    #[test]
    fn tar_liens_et_remontees_ignores() {
        let tmp = tempfile::tempdir().unwrap();
        let mut data = Vec::new();
        {
            let mut b = tar::Builder::new(&mut data);
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o4755);
            h.set_mtime(1_000_000_000);
            h.set_cksum();
            b.append_data(&mut h.clone(), "dir/fichier.txt", &b"abc"[..])
                .unwrap();
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            b.append_link(&mut link, "lien", "/etc/passwd").unwrap();
            // tar::Builder refuse `..` : on forge l'en-tête à la main.
            let mut evil = tar::Header::new_old();
            evil.as_old_mut().name[..9].copy_from_slice(b"../x.txt\0");
            evil.set_size(1);
            evil.set_cksum();
            b.append(&evil, &b"x"[..]).unwrap();
            b.finish().unwrap();
        }
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let warnings = std::sync::Mutex::new(Vec::new());
        let warn = |m: &str| warnings.lock().unwrap().push(m.to_string());
        let pos = Cell::new(0);
        let stats = tar(&data[..], &dest, 0, &hooks(&warn), (&pos, 1)).unwrap();
        assert_eq!(stats.files, 1);
        let f = dest.join("dir/fichier.txt");
        assert_eq!(fs::read(&f).unwrap(), b"abc");
        let mtime = f.metadata().unwrap().modified().unwrap();
        assert_eq!(mtime, UNIX_EPOCH + Duration::from_secs(1_000_000_000));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(f.metadata().unwrap().permissions().mode() & 0o7000, 0);
        }
        assert!(!dest.join("lien").exists());
        assert!(!tmp.path().join("x.txt").exists());
        assert_eq!(warnings.lock().unwrap().len(), 2);
    }
}
