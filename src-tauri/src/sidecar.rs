//! Résolution et exécution des binaires tiers (sidecars).
//!
//! Ordre de résolution : 1) sidecar bundlé (dossier de l'exécutable),
//! 2) dossier de données de l'app (`<app_data>/bin`, téléchargement au premier
//! usage), 3) PATH système. Les binaires sont toujours lancés directement via
//! [`std::process::Command`], jamais à travers un shell.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

/// Binaires tiers connus de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Binary {
    Ghostscript,
    Tesseract,
    Ffmpeg,
    ImageMagick,
    SevenZip,
    Pandoc,
    Typst,
    YtDlp,
    Whisper,
    RealEsrgan,
}

/// Mode d'obtention d'un binaire (voir PLAN.md §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// Inclus dans l'installeur (sidecar Tauri).
    Bundled,
    /// Téléchargé au premier usage dans le dossier de données de l'app.
    FirstUse,
}

impl Binary {
    pub const ALL: [Binary; 10] = [
        Binary::Ghostscript,
        Binary::Tesseract,
        Binary::Ffmpeg,
        Binary::ImageMagick,
        Binary::SevenZip,
        Binary::Pandoc,
        Binary::Typst,
        Binary::YtDlp,
        Binary::Whisper,
        Binary::RealEsrgan,
    ];

    /// Identifiant stable (utilisé par le frontend et le manifeste).
    pub fn id(self) -> &'static str {
        match self {
            Binary::Ghostscript => "ghostscript",
            Binary::Tesseract => "tesseract",
            Binary::Ffmpeg => "ffmpeg",
            Binary::ImageMagick => "imagemagick",
            Binary::SevenZip => "7zip",
            Binary::Pandoc => "pandoc",
            Binary::Typst => "typst",
            Binary::YtDlp => "yt-dlp",
            Binary::Whisper => "whisper",
            Binary::RealEsrgan => "realesrgan",
        }
    }

    /// Nom affiché.
    pub fn label(self) -> &'static str {
        match self {
            Binary::Ghostscript => "Ghostscript",
            Binary::Tesseract => "Tesseract OCR",
            Binary::Ffmpeg => "FFmpeg",
            Binary::ImageMagick => "ImageMagick",
            Binary::SevenZip => "7-Zip",
            Binary::Pandoc => "Pandoc",
            Binary::Typst => "Typst",
            Binary::YtDlp => "yt-dlp",
            Binary::Whisper => "whisper.cpp",
            Binary::RealEsrgan => "Real-ESRGAN (ncnn)",
        }
    }

    /// Licence (détails dans THIRD_PARTY.md).
    pub fn license(self) -> &'static str {
        match self {
            Binary::Ghostscript => "AGPL-3.0",
            Binary::Tesseract => "Apache-2.0",
            Binary::Ffmpeg => "LGPL-2.1+/GPL-2.0+ (selon build)",
            Binary::ImageMagick => "ImageMagick (type Apache-2.0)",
            Binary::SevenZip => "LGPL-2.1 (+ restriction unRAR)",
            Binary::Pandoc => "GPL-2.0+",
            Binary::Typst => "Apache-2.0",
            Binary::YtDlp => "Unlicense",
            Binary::Whisper => "MIT",
            Binary::RealEsrgan => "BSD-3-Clause",
        }
    }

    pub fn delivery(self) -> Delivery {
        match self {
            Binary::SevenZip => Delivery::Bundled,
            _ => Delivery::FirstUse,
        }
    }

    /// Noms d'exécutables candidats, par ordre de préférence (sans extension).
    pub fn candidates(self) -> &'static [&'static str] {
        match self {
            Binary::Ghostscript if cfg!(windows) => &["gswin64c", "gs"],
            Binary::Ghostscript => &["gs"],
            Binary::Tesseract => &["tesseract"],
            Binary::Ffmpeg => &["ffmpeg"],
            Binary::ImageMagick => &["magick"],
            Binary::SevenZip => &["7zz", "7za", "7z"],
            Binary::Pandoc => &["pandoc"],
            Binary::Typst => &["typst"],
            Binary::YtDlp => &["yt-dlp"],
            Binary::Whisper => &["whisper-cli"],
            Binary::RealEsrgan => &["realesrgan-ncnn-vulkan"],
        }
    }

    /// Arguments qui affichent la version.
    pub fn version_args(self) -> &'static [&'static str] {
        match self {
            Binary::Ffmpeg | Binary::ImageMagick => &["-version"],
            Binary::SevenZip => &[],
            Binary::Whisper | Binary::RealEsrgan => &["-h"],
            _ => &["--version"],
        }
    }
}

/// Origine d'un binaire résolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Bundled,
    AppData,
    SystemPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub source: Source,
}

/// Résout les binaires selon l'ordre bundlé → données app → PATH.
#[derive(Debug, Clone, Default)]
pub struct Resolver {
    bundled_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    path_var: Option<OsString>,
}

impl Resolver {
    pub fn new(
        bundled_dir: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        path_var: Option<OsString>,
    ) -> Self {
        Self {
            bundled_dir,
            data_dir,
            path_var,
        }
    }

    /// Résolveur de production : dossier de l'exécutable + `data_dir` + PATH.
    pub fn from_env(data_dir: Option<PathBuf>) -> Self {
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        Self::new(bundled, data_dir, std::env::var_os("PATH"))
    }

    pub fn resolve(&self, binary: Binary) -> Option<Resolved> {
        let names = binary.candidates();
        let dirs = [
            (self.bundled_dir.as_deref(), Source::Bundled),
            (self.data_dir.as_deref(), Source::AppData),
        ];
        for (dir, source) in dirs {
            if let Some(path) = dir.and_then(|d| find_in_dir(d, names)) {
                return Some(Resolved { path, source });
            }
        }
        let path_var = self.path_var.as_deref()?;
        std::env::split_paths(path_var)
            .find_map(|d| find_in_dir(&d, names))
            .map(|path| Resolved {
                path,
                source: Source::SystemPath,
            })
    }
}

fn find_in_dir(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|n| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX)))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Résultat d'une exécution.
#[derive(Debug)]
pub struct RunOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl RunOutput {
    /// Erreur explicite si le code de retour n'est pas 0.
    pub fn check(self, label: &str) -> Result<Self> {
        if self.status.success() {
            return Ok(self);
        }
        let tail: String = last_lines(&self.stderr, 8);
        bail!("{label} a échoué ({}) : {tail}", self.status)
    }
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Lance `program` avec `args` (sans shell), avec un délai maximal.
pub fn run<S: AsRef<OsStr>>(program: &Path, args: &[S], timeout: Duration) -> Result<RunOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("impossible de lancer {}", program.display()))?;
    let out = spawn_reader(child.stdout.take());
    let err = spawn_reader(child.stderr.take());
    let status = wait_with_timeout(&mut child, timeout, program)?;
    Ok(RunOutput {
        status,
        stdout: join_reader(out),
        stderr: join_reader(err),
    })
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

fn spawn_reader<R: Read + Send + 'static>(pipe: Option<R>) -> Option<JoinHandle<String>> {
    pipe.map(|mut p| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = p.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    })
}

fn join_reader(handle: Option<JoinHandle<String>>) -> String {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

fn wait_with_timeout(child: &mut Child, timeout: Duration, program: &Path) -> Result<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} interrompu : délai de {} s dépassé",
                program.display(),
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Première ligne non vide de la sortie « version » du binaire.
pub fn version(binary: Binary, path: &Path) -> Option<String> {
    let out = run(path, binary.version_args(), Duration::from_secs(10)).ok()?;
    out.stdout
        .lines()
        .chain(out.stderr.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

/// État d'un binaire pour l'écran Diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub license: &'static str,
    pub delivery: Delivery,
    pub present: bool,
    pub source: Option<Source>,
    pub path: Option<String>,
    pub version: Option<String>,
}

pub fn status(resolver: &Resolver, binary: Binary) -> BinaryStatus {
    let resolved = resolver.resolve(binary);
    let version = resolved.as_ref().and_then(|r| version(binary, &r.path));
    BinaryStatus {
        id: binary.id(),
        label: binary.label(),
        license: binary.license(),
        delivery: binary.delivery(),
        present: resolved.is_some(),
        source: resolved.as_ref().map(|r| r.source),
        path: resolved.map(|r| r.path.display().to_string()),
        version,
    }
}

pub fn check_all(resolver: &Resolver) -> Vec<BinaryStatus> {
    Binary::ALL.iter().map(|b| status(resolver, *b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_exe(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        fs::write(&path, b"fake").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn resolver(root: &Path) -> Resolver {
        let path_var = std::env::join_paths([root.join("path")]).unwrap();
        Resolver::new(
            Some(root.join("bundled")),
            Some(root.join("data")),
            Some(path_var),
        )
    }

    #[test]
    fn absent_partout() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolver(tmp.path()).resolve(Binary::Pandoc), None);
    }

    #[test]
    fn path_en_dernier_recours() {
        let tmp = tempfile::tempdir().unwrap();
        let p = fake_exe(&tmp.path().join("path"), "pandoc");
        let r = resolver(tmp.path()).resolve(Binary::Pandoc).unwrap();
        assert_eq!(
            r,
            Resolved {
                path: p,
                source: Source::SystemPath
            }
        );
    }

    #[test]
    fn donnees_app_avant_path() {
        let tmp = tempfile::tempdir().unwrap();
        fake_exe(&tmp.path().join("path"), "pandoc");
        let p = fake_exe(&tmp.path().join("data"), "pandoc");
        let r = resolver(tmp.path()).resolve(Binary::Pandoc).unwrap();
        assert_eq!(
            r,
            Resolved {
                path: p,
                source: Source::AppData
            }
        );
    }

    #[test]
    fn bundle_avant_tout() {
        let tmp = tempfile::tempdir().unwrap();
        fake_exe(&tmp.path().join("path"), "7z");
        fake_exe(&tmp.path().join("data"), "7zz");
        let p = fake_exe(&tmp.path().join("bundled"), "7za");
        let r = resolver(tmp.path()).resolve(Binary::SevenZip).unwrap();
        assert_eq!(
            r,
            Resolved {
                path: p,
                source: Source::Bundled
            }
        );
    }

    #[test]
    fn ordre_des_candidats_dans_un_dossier() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bundled");
        fake_exe(&dir, "7z");
        let best = fake_exe(&dir, "7zz");
        let r = resolver(tmp.path()).resolve(Binary::SevenZip).unwrap();
        assert_eq!(r.path, best);
    }

    #[cfg(unix)]
    #[test]
    fn fichier_non_executable_ignore() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bundled");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pandoc"), b"pas executable").unwrap();
        assert_eq!(resolver(tmp.path()).resolve(Binary::Pandoc), None);
    }

    #[test]
    fn candidats_et_ids_uniques() {
        let mut ids: Vec<_> = Binary::ALL.iter().map(|b| b.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Binary::ALL.len());
        assert!(Binary::ALL.iter().all(|b| !b.candidates().is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn execution_directe_capture_stdout() {
        let out = run(
            Path::new("/bin/echo"),
            &["bonjour", "a b;$(rm -rf x)"],
            Duration::from_secs(5),
        )
        .unwrap()
        .check("echo")
        .unwrap();
        // Sans shell : l'argument est transmis tel quel, rien n'est interprété.
        assert_eq!(out.stdout.trim(), "bonjour a b;$(rm -rf x)");
    }

    #[cfg(unix)]
    #[test]
    fn delai_depasse_tue_le_processus() {
        let start = Instant::now();
        let err = run(Path::new("sleep"), &["5"], Duration::from_millis(200)).unwrap_err();
        assert!(err.to_string().contains("délai"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn code_retour_non_nul_signale() {
        let out = run(Path::new("false"), &[] as &[&str], Duration::from_secs(5)).unwrap();
        assert!(out.check("false").is_err());
    }

    #[test]
    fn diagnostic_binaire_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let s = status(&resolver(tmp.path()), Binary::Ghostscript);
        assert!(!s.present);
        assert_eq!(s.path, None);
        assert_eq!(s.license, "AGPL-3.0");
    }
}
