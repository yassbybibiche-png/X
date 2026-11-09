//! Résolution et exécution des binaires tiers (sidecars).
//!
//! Ordre de résolution : 1) sidecar bundlé (dossier de l'exécutable),
//! 2) dossier de données de l'app (`<app_data>/bin`, téléchargement au premier
//! usage), 3) PATH système. Les binaires sont toujours lancés directement via
//! [`std::process::Command`], jamais à travers un shell.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Serialize, Serializer};

use crate::cancel::{CancelToken, Cancelled};

/// Binaires tiers connus de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    Qpdf,
}

/// Sérialisé par [`Binary::id`] : `ToolDef.binaries` doit employer les mêmes
/// clés que `BinaryStatus.id` (Diagnostic), sinon le frontend croit le binaire
/// absent (v0.1.0 : `seven-zip` ≠ `7zip`, 7 outils grisés à tort).
impl Serialize for Binary {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.id())
    }
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
    pub const ALL: [Binary; 11] = [
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
        Binary::Qpdf,
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
            Binary::Qpdf => "qpdf",
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
            Binary::Qpdf => "qpdf",
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
            Binary::Qpdf => "Apache-2.0",
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
            // `convert` = nom d'ImageMagick 6 (Ubuntu/Debian). Jamais sous
            // Windows : `convert.exe` y est l'outil système de conversion FAT→NTFS.
            Binary::ImageMagick if cfg!(windows) => &["magick"],
            Binary::ImageMagick => &["magick", "convert"],
            Binary::SevenZip => &["7zz", "7za", "7z"],
            Binary::Pandoc => &["pandoc"],
            Binary::Typst => &["typst"],
            Binary::YtDlp => &["yt-dlp"],
            Binary::Whisper => &["whisper-cli"],
            Binary::RealEsrgan => &["realesrgan-ncnn-vulkan"],
            Binary::Qpdf => &["qpdf"],
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

    /// Vrai si [`Binary::version_args`] affiche en fait l'aide : certains
    /// binaires sortent alors avec un code non nul (Real-ESRGAN `-h` : 255).
    fn version_is_help(self) -> bool {
        matches!(
            self,
            Binary::SevenZip | Binary::Whisper | Binary::RealEsrgan
        )
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
    bundled_dirs: Vec<PathBuf>,
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
            bundled_dirs: bundled_dir.into_iter().collect(),
            data_dir,
            path_var,
        }
    }

    /// Ajoute un dossier de binaires bundlés (ex. ressources Tauri `binaries/`).
    pub fn with_bundled_dir(mut self, dir: PathBuf) -> Self {
        self.bundled_dirs.push(dir);
        self
    }

    /// Résolveur de production : dossier de l'exécutable + `data_dir` + PATH,
    /// complété par les dossiers d'installation connus (voir [`known_install_dirs`]).
    pub fn from_env(data_dir: Option<PathBuf>) -> Self {
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        dirs.extend(known_install_dirs(std::env::consts::OS, |k| {
            std::env::var_os(k)
        }));
        Self::new(bundled, data_dir, std::env::join_paths(dirs).ok())
    }

    /// Dossier `<données app>/bin` (cible des téléchargements).
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    pub fn resolve(&self, binary: Binary) -> Option<Resolved> {
        let names = binary.candidates();
        let bundled = self
            .bundled_dirs
            .iter()
            .find_map(|d| find_in_dir(d, names))
            .map(|path| (path, Source::Bundled));
        bundled
            .or_else(|| self.in_data_dir(binary).map(|p| (p, Source::AppData)))
            .or_else(|| self.in_path(names).map(|p| (p, Source::SystemPath)))
            .map(|(path, source)| Resolved { path, source })
    }

    /// Dossier données : `<bin>/<nom>` (dépôt manuel) ou installation du manifeste.
    fn in_data_dir(&self, binary: Binary) -> Option<PathBuf> {
        let dir = self.data_dir.as_deref()?;
        find_in_dir(dir, binary.candidates())
            .or_else(|| crate::download::installed_exe(dir, binary).filter(|p| is_executable(p)))
    }

    fn in_path(&self, names: &[&str]) -> Option<PathBuf> {
        let path_var = self.path_var.as_deref()?;
        std::env::split_paths(path_var).find_map(|d| find_in_dir(&d, names))
    }
}

/// Dossiers d'installation habituels, ajoutés après le PATH :
/// - macOS : une app lancée depuis le Finder reçoit un PATH minimal
///   (`/usr/bin:/bin:/usr/sbin:/sbin`) — Homebrew (`/opt/homebrew/bin`,
///   `/usr/local/bin`) et MacPorts n'y sont pas ;
/// - Windows : les installeurs officiels de Ghostscript, Tesseract, 7-Zip,
///   ImageMagick, pandoc ne modifient pas toujours le PATH.
pub fn known_install_dirs(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    match os {
        "macos" => ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .map(PathBuf::from)
            .to_vec(),
        "windows" => windows_install_dirs(&env),
        _ => Vec::new(),
    }
}

fn windows_install_dirs(env: &impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ProgramW6432", "ProgramFiles"]
        .iter()
        .filter_map(|k| env(k).map(PathBuf::from))
        .collect();
    roots.dedup();
    let mut dirs = Vec::new();
    for pf in &roots {
        dirs.extend(["7-Zip", "Tesseract-OCR", "Pandoc"].map(|d| pf.join(d)));
        // Dossiers versionnés : la version la plus récente d'abord.
        dirs.extend(versioned(&pf.join("gs"), "gs", Some("bin")));
        dirs.extend(versioned(pf, "ImageMagick-", None));
    }
    if let Some(local) = env("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Pandoc"));
    }
    dirs
}

/// Sous-dossiers de `parent` dont le nom commence par `prefix`, triés par nom
/// décroissant (≈ version la plus récente d'abord), suivis de `sub`.
fn versioned(parent: &Path, prefix: &str, sub: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| match sub {
            Some(s) => e.path().join(s),
            None => e.path(),
        })
        .collect();
    found.sort_unstable_by(|a, b| b.cmp(a));
    found
}

/// Exécutable voisin d'un binaire résolu (ex. `ffprobe` à côté de `ffmpeg`).
pub fn sibling(path: &Path, name: &str) -> Option<PathBuf> {
    find_in_dir(path.parent()?, &[name])
}

fn find_in_dir(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|n| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX)))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Erreurs typées d'exécution d'un sidecar (stderr tronqué).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarError {
    Spawn {
        program: String,
        message: String,
    },
    Timeout {
        program: String,
        seconds: u64,
    },
    Failed {
        label: String,
        status: String,
        stderr: String,
    },
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, message } => {
                write!(f, "impossible de lancer {program} : {message}")
            }
            Self::Timeout { program, seconds } => {
                write!(f, "{program} interrompu : délai de {seconds} s dépassé")
            }
            Self::Failed {
                label,
                status,
                stderr,
            } => write!(f, "{label} a échoué ({status}) : {stderr}"),
        }
    }
}

impl std::error::Error for SidecarError {}

/// Taille maximale de stderr reprise dans un message d'erreur.
const STDERR_MAX_CHARS: usize = 1200;
/// Sortie conservée en mémoire par flux (au-delà, seule la fin est gardée).
const CAPTURE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Dernières lignes non vides de `text`, limitées à [`STDERR_MAX_CHARS`].
pub fn truncate_stderr(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    let count = tail.chars().count();
    if count <= STDERR_MAX_CHARS {
        return tail;
    }
    let kept: String = tail.chars().skip(count - STDERR_MAX_CHARS).collect();
    format!("…{kept}")
}

/// Résultat d'une exécution.
#[derive(Debug)]
pub struct RunOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl RunOutput {
    /// Erreur typée ([`SidecarError::Failed`]) si le code de retour n'est pas 0.
    pub fn check(self, label: &str) -> Result<Self> {
        if self.status.success() {
            return Ok(self);
        }
        Err(SidecarError::Failed {
            label: label.to_string(),
            status: self.status.to_string(),
            stderr: truncate_stderr(&self.stderr),
        }
        .into())
    }
}

/// Flux de sortie d'un processus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Rappel appelé pour chaque ligne (séparateurs `\n` et `\r`).
pub type LineFn<'a> = &'a (dyn Fn(Stream, &str) + Sync);

/// Options d'exécution : délai, annulation, lecture ligne à ligne.
#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub timeout: Duration,
    pub cancel: Option<&'a CancelToken>,
    pub on_line: Option<LineFn<'a>>,
    /// Dossier de travail du processus (sinon celui de l'application).
    pub current_dir: Option<&'a Path>,
    /// Données écrites sur l'entrée standard puis fermée (ex. mot de passe :
    /// jamais visible dans la liste des processus, contrairement aux arguments).
    pub stdin: Option<&'a [u8]>,
}

impl RunOptions<'_> {
    pub fn timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cancel: None,
            on_line: None,
            current_dir: None,
            stdin: None,
        }
    }
}

/// Lance `program` avec `args` (sans shell), avec un délai maximal.
pub fn run<S: AsRef<OsStr>>(program: &Path, args: &[S], timeout: Duration) -> Result<RunOutput> {
    run_with(program, args, RunOptions::timeout(timeout))
}

/// Lance `program` (sans shell) ; tue le processus si le délai expire ou si la
/// tâche est annulée ; transmet chaque ligne de sortie à `on_line`.
pub fn run_with<S: AsRef<OsStr>>(
    program: &Path,
    args: &[S],
    opts: RunOptions,
) -> Result<RunOutput> {
    let args = without_network(program, args);
    let mut child = spawn(program, &args, &opts)?;
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let in_pipe = child.stdin.take();
    std::thread::scope(|s| {
        if let (Some(mut pipe), Some(data)) = (in_pipe, opts.stdin) {
            s.spawn(move || {
                use std::io::Write;
                let _ = pipe.write_all(data);
            });
        }
        let on_line = opts.on_line;
        let out = s.spawn(move || read_stream(out_pipe, Stream::Stdout, on_line));
        let err = s.spawn(move || read_stream(err_pipe, Stream::Stderr, on_line));
        let status = wait(&mut child, program, &opts);
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        Ok(RunOutput {
            status: status?,
            stdout,
            stderr,
        })
    })
}

/// Protocoles autorisés pour les entrées de ffmpeg / ffprobe.
const MEDIA_PROTOCOLS: &str = "file,pipe";

/// Interdit le réseau aux entrées de ffmpeg / ffprobe : un fichier déposé qui
/// est en réalité une playlist (HLS `#EXTM3U`, `concat`…) pourrait sinon faire
/// contacter des serveurs (pistage, exfiltration). `-protocol_whitelist` est
/// ajouté avant chaque `-i` (ffmpeg) ou en tête (ffprobe, entrée positionnelle).
pub fn without_network<S: AsRef<OsStr>>(program: &Path, args: &[S]) -> Vec<OsString> {
    let stem = program
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut out = Vec::with_capacity(args.len() + 4);
    if stem == "ffprobe" {
        out.extend(["-protocol_whitelist".into(), MEDIA_PROTOCOLS.into()]);
    }
    for a in args {
        if stem == "ffmpeg" && a.as_ref() == "-i" {
            out.extend(["-protocol_whitelist".into(), MEDIA_PROTOCOLS.into()]);
        }
        out.push(a.as_ref().to_owned());
    }
    out
}

fn spawn<S: AsRef<OsStr>>(program: &Path, args: &[S], opts: &RunOptions) -> Result<Child> {
    let mut cmd = Command::new(program);
    if let Some(dir) = opts.current_dir {
        cmd.current_dir(dir);
    }
    let stdin = if opts.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    apply_env_fixes(&mut cmd, appimage_env_fixes());
    // Les AppImage téléchargées (ImageMagick Linux) exigent FUSE, absent par
    // défaut sur beaucoup de distributions (Ubuntu 24.04 : pas de libfuse2) :
    // cette variable les fait s'auto-extraire. Sans effet sur les autres binaires.
    cmd.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    cmd.args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    cmd.spawn().map_err(|e| {
        SidecarError::Spawn {
            program: program.display().to_string(),
            message: e.to_string(),
        }
        .into()
    })
}

/// Modification d'une variable d'environnement : `None` = la retirer.
pub type EnvFix = (OsString, Option<OsString>);

/// Variables posées par le runtime AppImage et son `AppRun`, retirées en bloc.
const APPIMAGE_VARS: [&str; 7] = [
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
    "LD_PRELOAD",
    "PYTHONHOME",
    "PYTHONDONTWRITEBYTECODE",
];

/// Environnement à rendre aux sidecars quand l'app tourne en AppImage
/// (calculé une fois ; vide hors AppImage).
fn appimage_env_fixes() -> &'static [EnvFix] {
    static FIXES: std::sync::OnceLock<Vec<EnvFix>> = std::sync::OnceLock::new();
    FIXES.get_or_init(|| match appimage_dir() {
        Some(dir) => env_fixes(std::env::vars_os(), &dir),
        None => Vec::new(),
    })
}

/// Dossier monté de l'AppImage en cours (`APPDIR`), si l'app en est une.
fn appimage_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let dir = PathBuf::from(std::env::var_os("APPDIR").filter(|d| !d.is_empty())?);
    let inside = std::env::var_os("APPIMAGE").is_some()
        || std::env::current_exe().is_ok_and(|exe| exe.starts_with(&dir));
    inside.then_some(dir)
}

/// L'AppImage transmet à ses processus fils ses bibliothèques et son Python
/// (`LD_LIBRARY_PATH`, `PYTHONHOME`… pointant dans `appdir`), ce qui casse les
/// binaires du système et ceux téléchargés (v0.1.0 : Tesseract en `symbol
/// lookup error` sur le libnghttp2 embarqué, yt-dlp en « Python path
/// configuration »). Retire les variables propres à l'AppImage et, dans les
/// listes de chemins (`PATH`, `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`, `GTK_PATH`…),
/// les entrées situées dans `appdir` et les entrées vides (= dossier courant)
/// qu'elle y ajoute. Les valeurs d'origine de l'utilisateur sont conservées.
pub fn env_fixes(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    appdir: &Path,
) -> Vec<EnvFix> {
    let marker = appdir.to_string_lossy();
    let marker = marker.trim_end_matches('/');
    let mut fixes = Vec::new();
    for (key, value) in vars {
        let name = key.to_string_lossy();
        if APPIMAGE_VARS.contains(&name.as_ref()) || name.starts_with("APPIMAGE_") {
            fixes.push((key, None));
            continue;
        }
        if marker.is_empty() || !value.to_string_lossy().contains(marker) {
            continue;
        }
        let entries: Vec<PathBuf> = std::env::split_paths(&value).collect();
        if !entries.iter().any(|p| p.starts_with(appdir)) {
            continue;
        }
        let kept: Vec<PathBuf> = entries
            .into_iter()
            .filter(|p| !p.as_os_str().is_empty() && !p.starts_with(appdir))
            .collect();
        let new = (!kept.is_empty())
            .then(|| std::env::join_paths(kept).ok())
            .flatten();
        fixes.push((key, new));
    }
    fixes
}

fn apply_env_fixes(cmd: &mut Command, fixes: &[EnvFix]) {
    for (key, value) in fixes {
        match value {
            Some(v) => cmd.env(key, v),
            None => cmd.env_remove(key),
        };
    }
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

fn read_stream<R: Read>(pipe: Option<R>, stream: Stream, on_line: Option<LineFn>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut captured = Vec::new();
    let mut pending = Vec::new();
    let mut buf = [0u8; 8192];
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        keep_tail(&mut captured, &buf[..n]);
        if let Some(f) = on_line {
            split_lines(&mut pending, &buf[..n], |l| f(stream, l));
        }
    }
    if let Some(f) = on_line {
        split_lines(&mut pending, b"\n", |l| f(stream, l));
    }
    String::from_utf8_lossy(&captured).into_owned()
}

fn keep_tail(captured: &mut Vec<u8>, chunk: &[u8]) {
    captured.extend_from_slice(chunk);
    if captured.len() > CAPTURE_MAX_BYTES {
        let excess = captured.len() - CAPTURE_MAX_BYTES;
        captured.drain(..excess);
    }
}

/// Découpe en lignes (`\n`, `\r`), en ignorant les retours arrière (`\x08`).
fn split_lines(pending: &mut Vec<u8>, chunk: &[u8], mut emit: impl FnMut(&str)) {
    for &b in chunk {
        match b {
            b'\n' | b'\r' => {
                if !pending.is_empty() {
                    emit(&String::from_utf8_lossy(pending));
                    pending.clear();
                }
            }
            0x08 => {}
            _ => pending.push(b),
        }
    }
}

fn wait(child: &mut Child, program: &Path, opts: &RunOptions) -> Result<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if opts.cancel.is_some_and(CancelToken::is_cancelled) {
            kill(child);
            return Err(Cancelled.into());
        }
        if start.elapsed() >= opts.timeout {
            kill(child);
            return Err(SidecarError::Timeout {
                program: program.display().to_string(),
                seconds: opts.timeout.as_secs(),
            }
            .into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Résultat du test d'exécution d'un binaire (Diagnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Le binaire s'exécute ; version extraite si elle est lisible.
    Runs(Option<String>),
    /// Le binaire est trouvé mais ne s'exécute pas (bibliothèque manquante,
    /// erreur Python…) ; message d'erreur tronqué.
    Broken(String),
}

/// Lance le binaire avec ses arguments « version » : il ne compte comme
/// présent que s'il s'exécute réellement (v0.1.0 : un Tesseract en
/// `symbol lookup error` était affiché « Présent »).
pub fn probe(binary: Binary, path: &Path) -> Probe {
    let out = match run(path, binary.version_args(), Duration::from_secs(10)) {
        Ok(out) => out,
        // Lent sans planter (ex. AppImage qui s'auto-extrait) : bénéfice du doute.
        Err(e)
            if matches!(
                e.downcast_ref::<SidecarError>(),
                Some(SidecarError::Timeout { .. })
            ) =>
        {
            return Probe::Runs(None);
        }
        Err(e) => return Probe::Broken(e.to_string()),
    };
    let help_shown = binary.version_is_help()
        && (out.stdout.to_ascii_lowercase().contains("usage")
            || out.stderr.to_ascii_lowercase().contains("usage"));
    if out.status.success() || help_shown {
        return Probe::Runs(parse_version(&out.stdout, &out.stderr));
    }
    let detail = first_line(&out.stderr)
        .or_else(|| first_line(&out.stdout))
        .unwrap_or_default();
    let detail: String = detail.chars().take(200).collect();
    Probe::Broken(format!("ne s'exécute pas ({}) : {detail}", out.status))
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

/// Numéro de version lu dans la première ligne de stdout, sinon de stderr :
/// premier mot qui commence par un chiffre (ou `v` + chiffre) et contient un
/// point (« ffmpeg version 8.1.2 Copyright… » → `8.1.2`). `None` si illisible.
pub fn parse_version(stdout: &str, stderr: &str) -> Option<String> {
    [stdout, stderr]
        .into_iter()
        .filter_map(first_line)
        .find_map(|line| line.split_whitespace().find_map(version_token))
}

fn version_token(word: &str) -> Option<String> {
    let w = word.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | ':'));
    let w = w
        .strip_prefix(['v', 'V'])
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(w);
    let valid = w.starts_with(|c: char| c.is_ascii_digit())
        && w.contains('.')
        && w.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_' | '~'));
    valid.then(|| w.chars().take(40).collect())
}

/// Version du binaire (`None` s'il ne s'exécute pas ou si elle est illisible).
pub fn version(binary: Binary, path: &Path) -> Option<String> {
    match probe(binary, path) {
        Probe::Runs(v) => v,
        Probe::Broken(_) => None,
    }
}

/// Disponibilité d'un binaire pour l'écran Diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Trouvé (bundlé, données app ou PATH) et s'exécute.
    Present,
    /// Trouvé mais ne s'exécute pas (voir `BinaryStatus.error`).
    Casse,
    /// Absent mais téléchargeable depuis le manifeste épinglé.
    Telechargeable,
    /// Absent et non téléchargeable sur cette plateforme (raison fournie).
    Absent,
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
    pub state: State,
    pub source: Option<Source>,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Version épinglée dans `deps.lock.json` (si téléchargeable).
    pub pinned_version: Option<String>,
    /// Taille totale à télécharger, en octets.
    pub download_size: Option<u64>,
    /// Raison pour laquelle le binaire n'est pas téléchargeable ici.
    pub unavailable_reason: Option<String>,
    /// Erreur d'exécution d'un binaire trouvé mais cassé ([`State::Casse`]).
    pub error: Option<String>,
}

pub fn status(resolver: &Resolver, binary: Binary) -> BinaryStatus {
    let resolved = resolver.resolve(binary);
    let probe = resolved.as_ref().map(|r| probe(binary, &r.path));
    let offer = crate::download::offer(binary);
    let state = match (&probe, &offer) {
        (Some(Probe::Runs(_)), _) => State::Present,
        (Some(Probe::Broken(_)), _) => State::Casse,
        (None, Ok(_)) => State::Telechargeable,
        (None, Err(_)) => State::Absent,
    };
    let (version, error) = match probe {
        Some(Probe::Runs(v)) => (v, None),
        Some(Probe::Broken(e)) => (None, Some(e)),
        None => (None, None),
    };
    BinaryStatus {
        id: binary.id(),
        label: binary.label(),
        license: binary.license(),
        delivery: binary.delivery(),
        present: state == State::Present,
        state,
        source: resolved.as_ref().map(|r| r.source),
        path: resolved.map(|r| r.path.display().to_string()),
        version,
        error,
        pinned_version: offer.as_ref().ok().map(|o| o.version.clone()),
        download_size: offer.as_ref().ok().map(|o| o.size),
        unavailable_reason: offer.err(),
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

    #[test]
    fn serialise_par_son_id() {
        for b in Binary::ALL {
            assert_eq!(serde_json::to_value(b).unwrap(), b.id());
        }
    }

    #[test]
    fn dossiers_d_installation_connus() {
        let mac = known_install_dirs("macos", |_| None);
        assert!(mac.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(known_install_dirs("linux", |_| None).is_empty());

        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("Program Files");
        for d in [
            "gs/gs10.02.1/bin",
            "gs/gs10.08.0/bin",
            "ImageMagick-7.1.2-Q16-HDRI",
        ] {
            fs::create_dir_all(pf.join(d)).unwrap();
        }
        let local = tmp.path().join("AppData/Local");
        let env = |k: &str| match k {
            "ProgramFiles" | "ProgramW6432" => Some(pf.clone().into_os_string()),
            "LOCALAPPDATA" => Some(local.clone().into_os_string()),
            _ => None,
        };
        let win = known_install_dirs("windows", env);
        let pos = |p: PathBuf| win.iter().position(|d| *d == p).unwrap();
        assert!(pos(pf.join("gs/gs10.08.0/bin")) < pos(pf.join("gs/gs10.02.1/bin")));
        assert!(win.contains(&pf.join("Tesseract-OCR")));
        assert!(win.contains(&pf.join("ImageMagick-7.1.2-Q16-HDRI")));
        assert!(win.contains(&local.join("Pandoc")));
        assert_eq!(win.iter().filter(|d| **d == pf.join("7-Zip")).count(), 1);
    }

    #[test]
    fn ffmpeg_sans_reseau() {
        let args = without_network(
            Path::new("/x/ffmpeg.exe"),
            &["-y", "-i", "a.mp4", "-i", "b.srt", "o.mp4"],
        );
        let pl = ["-protocol_whitelist", MEDIA_PROTOCOLS];
        let expected: Vec<OsString> = ["-y"]
            .iter()
            .chain(&pl)
            .chain(&["-i", "a.mp4"])
            .chain(&pl)
            .chain(&["-i", "b.srt", "o.mp4"])
            .map(OsString::from)
            .collect();
        assert_eq!(args, expected);
        assert_eq!(
            without_network(Path::new("ffprobe"), &["f.mkv"])[..2],
            pl.map(OsString::from)
        );
        assert_eq!(
            without_network(Path::new("gs"), &["-i"]),
            vec![OsString::from("-i")]
        );
    }

    /// Le vrai ffmpeg refuse toute entrée réseau, même passée explicitement.
    #[test]
    fn ffmpeg_reel_refuse_le_reseau() {
        let Some(r) = crate::tools::testutil::real(&[Binary::Ffmpeg]) else {
            return;
        };
        let ffmpeg = r.resolve(Binary::Ffmpeg).unwrap().path;
        let out = run(
            &ffmpeg,
            &["-hide_banner", "-i", "http://127.0.0.1:9/pistage.ts"],
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(out.stderr.contains("not on whitelist"), "{}", out.stderr);
    }

    /// Les AppImage (ImageMagick Linux) doivent tourner sans FUSE.
    #[cfg(unix)]
    #[test]
    fn appimage_sans_fuse() {
        let out = run(
            Path::new("/usr/bin/env"),
            &[] as &[&str],
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(out.stdout.contains("APPIMAGE_EXTRACT_AND_RUN=1"));
    }

    /// Environnement d'une AppImage linuxdeploy (runtime + `AppRun` + hook
    /// GTK) tel que l'app v0.1.0 le recevait et le transmettait aux sidecars.
    #[cfg(unix)]
    fn appimage_env(appdir: &Path, user_ld: Option<&str>) -> Vec<(OsString, OsString)> {
        let a = appdir.display();
        let path = std::env::var("PATH").unwrap_or_default();
        let ld = user_ld.unwrap_or_default();
        [
            ("APPDIR", a.to_string()),
            ("APPIMAGE", "/home/u/Boite a outils.AppImage".into()),
            ("ARGV0", "./Boite a outils.AppImage".into()),
            ("OWD", "/home/u".into()),
            ("APPIMAGE_EXTRACT_AND_RUN", "1".into()),
            ("LD_PRELOAD", format!("{a}/usr/lib/libhook.so")),
            (
                "PATH",
                format!("{a}/usr/bin/:{a}/usr/sbin/:{a}/bin/:{path}"),
            ),
            (
                "LD_LIBRARY_PATH",
                format!("{a}/usr/lib/:{a}/usr/lib/x86_64-linux-gnu/:{a}/lib/:{ld}"),
            ),
            ("PYTHONHOME", format!("{a}/usr/")),
            ("PYTHONPATH", format!("{a}/usr/share/pyshared/:")),
            ("PYTHONDONTWRITEBYTECODE", "1".into()),
            (
                "XDG_DATA_DIRS",
                format!("{a}/usr/share:/usr/share:{a}/usr/share/:"),
            ),
            (
                "PERLLIB",
                format!("{a}/usr/share/perl5/:{a}/usr/lib/perl5/:"),
            ),
            (
                "GSETTINGS_SCHEMA_DIR",
                format!("{a}//usr/share/glib-2.0/schemas"),
            ),
            ("GTK_PATH", format!("{a}//usr/lib/gtk-3.0")),
            ("GTK_DATA_PREFIX", a.to_string()),
            ("GTK_THEME", "Adwaita:light".into()),
            ("LANG", "C.UTF-8".into()),
        ]
        .map(|(k, v)| (k.into(), v.into()))
        .to_vec()
    }

    /// Environnement final après correction (même logique que `Command`).
    #[cfg(unix)]
    fn fixed(vars: &[(OsString, OsString)], appdir: &Path) -> Vec<(String, String)> {
        let mut env: std::collections::BTreeMap<OsString, OsString> =
            vars.iter().cloned().collect();
        for (k, v) in env_fixes(vars.to_vec(), appdir) {
            match v {
                Some(v) => env.insert(k, v),
                None => env.remove(&k),
            };
        }
        env.into_iter()
            .map(|(k, v)| (k.to_string_lossy().into(), v.to_string_lossy().into()))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn environnement_appimage_nettoye() {
        let appdir = Path::new("/tmp/.mount_BoiteXy12");
        let env = fixed(&appimage_env(appdir, Some("/opt/perso/lib")), appdir);
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        for k in [
            "APPDIR",
            "APPIMAGE",
            "ARGV0",
            "OWD",
            "APPIMAGE_EXTRACT_AND_RUN",
            "LD_PRELOAD",
            "PYTHONHOME",
            "PYTHONPATH",
            "PYTHONDONTWRITEBYTECODE",
            "PERLLIB",
            "GSETTINGS_SCHEMA_DIR",
            "GTK_PATH",
            "GTK_DATA_PREFIX",
        ] {
            assert_eq!(get(k), None, "{k} doit être retirée");
        }
        assert!(
            env.iter().all(|(_, v)| !v.contains(".mount_BoiteXy12")),
            "{env:?}"
        );
        // Valeurs de l'utilisateur conservées, entrées vides (= dossier courant) retirées.
        assert_eq!(get("LD_LIBRARY_PATH"), Some("/opt/perso/lib"));
        assert_eq!(get("PATH"), std::env::var("PATH").ok().as_deref());
        assert_eq!(get("XDG_DATA_DIRS"), Some("/usr/share"));
        assert_eq!(get("GTK_THEME"), Some("Adwaita:light"));
        assert_eq!(get("LANG"), Some("C.UTF-8"));
        // Sans valeur d'origine, LD_LIBRARY_PATH disparaît complètement.
        let env = fixed(&appimage_env(appdir, None), appdir);
        assert!(env.iter().all(|(k, _)| k != "LD_LIBRARY_PATH"));
    }

    #[test]
    fn environnement_ordinaire_inchange() {
        let vars: Vec<(OsString, OsString)> = [
            ("PATH", "/usr/local/bin:/usr/bin:/bin"),
            ("LD_LIBRARY_PATH", "/opt/perso/lib"),
            ("PYTHONPATH", "/home/u/py"),
            // Préfixe commun sans être dans le dossier : conservé.
            (
                "XDG_DATA_DIRS",
                "/tmp/.mount_BoiteXy12-autre/share:/usr/share",
            ),
        ]
        .map(|(k, v)| (k.into(), v.into()))
        .to_vec();
        assert_eq!(env_fixes(vars, Path::new("/tmp/.mount_BoiteXy12")), vec![]);
    }

    /// Le spawn réel applique les corrections : vérifié avec `env`.
    #[cfg(unix)]
    #[test]
    fn corrections_appliquees_au_processus_fils() {
        let appdir = Path::new("/tmp/.mount_BoiteXy12");
        let vars = appimage_env(appdir, None);
        let mut cmd = Command::new("/usr/bin/env");
        cmd.env_clear().envs(vars.iter().cloned());
        apply_env_fixes(&mut cmd, &env_fixes(vars, appdir));
        let out = String::from_utf8(cmd.output().unwrap().stdout).unwrap();
        assert!(!out.contains(".mount_BoiteXy12"), "{out}");
        assert!(out.contains("GTK_THEME=Adwaita:light"), "{out}");
    }

    /// Vrais Tesseract / yt-dlp lancés sous environnement AppImage simulé :
    /// cassés avant correction, fonctionnels après. `BOITE_APPIMAGE_EXTRAITE`
    /// (dossier `squashfs-root` d'une vraie AppImage) reproduit en plus la
    /// panne de Tesseract sur le libnghttp2 embarqué et vérifie le 7-Zip bundlé.
    #[cfg(unix)]
    #[test]
    fn tesseract_et_ytdlp_sous_appimage() {
        let tmp = tempfile::tempdir().unwrap();
        let real_appdir = std::env::var_os("BOITE_APPIMAGE_EXTRAITE").map(PathBuf::from);
        let appdir = real_appdir.clone().unwrap_or_else(|| {
            fs::create_dir_all(tmp.path().join("usr/lib")).unwrap();
            tmp.path().to_path_buf()
        });
        let vars = appimage_env(&appdir, None);
        let launch = |program: &Path, args: &[&str], fix: bool| {
            let mut cmd = Command::new(program);
            cmd.args(args).env_clear().envs(vars.iter().cloned());
            if fix {
                apply_env_fixes(&mut cmd, &env_fixes(vars.clone(), &appdir));
            }
            let out = cmd.output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr);
            (out.status.success(), text)
        };
        let mut checked = vec![];
        // Binaires du PATH (cas v0.1.0) puis téléchargés au premier usage.
        let system = Resolver::new(None, None, std::env::var_os("PATH"));
        let downloaded = Resolver::new(None, Some(crate::tools::testutil::scratch_bin_dir()), None);
        for (r, origin) in [(system, "PATH"), (downloaded, "téléchargé")] {
            if let Some(p) = r.resolve(Binary::YtDlp).map(|r| r.path) {
                // Script Python (paquet du système) : cassé par `PYTHONHOME`.
                let script = fs::read(&p).unwrap_or_default();
                let shebang = script.split(|b| *b == b'\n').next().unwrap_or_default();
                if shebang.starts_with(b"#!") && String::from_utf8_lossy(shebang).contains("python")
                {
                    let (ok, text) = launch(&p, &["--version"], false);
                    assert!(!ok && text.contains("Python path configuration"), "{text}");
                }
                let (ok, text) = launch(&p, &["--version"], true);
                assert!(ok && parse_version(&text, "").is_some(), "{text}");
                checked.push(format!("yt-dlp ({origin})"));
            }
            if let Some(p) = r.resolve(Binary::Tesseract).map(|r| r.path) {
                if real_appdir.is_some() {
                    let (ok, text) = launch(&p, &["--version"], false);
                    assert!(!ok && text.contains("symbol lookup error"), "{text}");
                }
                let (ok, text) = launch(&p, &["--version"], true);
                assert!(ok && text.starts_with("tesseract "), "{text}");
                checked.push(format!("tesseract ({origin})"));
            }
        }
        let bundled = appdir.join("usr/bin/7zz");
        if bundled.exists() {
            let (_, text) = launch(&bundled, &[], true);
            assert!(text.contains("7-Zip"), "{text}");
            checked.push("7zz (bundlé)".into());
        }
        eprintln!("vérifiés sous AppImage simulée : {checked:?}");
    }

    /// Diagnostic des vrais Tesseract / yt-dlp du PATH : « Présent ». Lancé
    /// dans l'environnement d'une AppImage (`APPDIR`, `APPIMAGE`,
    /// `LD_LIBRARY_PATH`…), ce test couvre aussi la détection automatique.
    #[test]
    fn diagnostic_reel_tesseract_ytdlp() {
        let r = Resolver::new(None, None, std::env::var_os("PATH"));
        for b in [Binary::Tesseract, Binary::YtDlp] {
            if r.resolve(b).is_none() {
                eprintln!("{} absent — test réel ignoré", b.label());
                continue;
            }
            let s = status(&r, b);
            assert_eq!(s.state, State::Present, "{s:?}");
        }
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

    #[cfg(unix)]
    #[test]
    fn annulation_tue_le_processus() {
        let token = CancelToken::default();
        let t2 = token.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            t2.cancel();
        });
        let start = Instant::now();
        let opts = RunOptions {
            cancel: Some(&token),
            ..RunOptions::timeout(Duration::from_secs(30))
        };
        let err = run_with(Path::new("sleep"), &["10"], opts).unwrap_err();
        canceller.join().unwrap();
        assert!(crate::cancel::is_cancelled(&err));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn lignes_transmises_au_fil_de_l_eau() {
        let lines = std::sync::Mutex::new(Vec::new());
        let on_line = |s: Stream, l: &str| lines.lock().unwrap().push((s, l.to_string()));
        let opts = RunOptions {
            on_line: Some(&on_line),
            ..RunOptions::timeout(Duration::from_secs(5))
        };
        run_with(Path::new("printf"), &["a\\rb\\nc"], opts).unwrap();
        let got: Vec<String> = lines
            .lock()
            .unwrap()
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        assert_eq!(got, ["a", "b", "c"]);
    }

    #[cfg(unix)]
    #[test]
    fn donnees_sur_l_entree_standard() {
        let opts = RunOptions {
            stdin: Some(b"secret\n"),
            ..RunOptions::timeout(Duration::from_secs(5))
        };
        let out = run_with(Path::new("cat"), &[] as &[&str], opts).unwrap();
        assert_eq!(out.stdout, "secret\n");
    }

    #[test]
    fn stderr_tronque() {
        let long: String = (0..50).map(|i| format!("ligne {i}\n")).collect();
        let t = truncate_stderr(&long);
        assert!(t.starts_with("ligne 42") && t.ends_with("ligne 49"), "{t}");
        let huge = "x".repeat(5000);
        assert_eq!(truncate_stderr(&huge).chars().count(), STDERR_MAX_CHARS + 1);
    }

    #[cfg(unix)]
    #[test]
    fn echec_erreur_typee() {
        let out = run(Path::new("ls"), &["/nexiste/pas"], Duration::from_secs(5)).unwrap();
        let err = out.check("ls").unwrap_err();
        let typed = err.downcast_ref::<SidecarError>().unwrap();
        assert!(matches!(typed, SidecarError::Failed { label, .. } if label == "ls"));
        let err = run(
            Path::new("/nexiste/pas"),
            &[] as &[&str],
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<SidecarError>(),
            Some(SidecarError::Spawn { .. })
        ));
    }

    #[test]
    fn diagnostic_binaire_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let s = status(&resolver(tmp.path()), Binary::Ghostscript);
        assert!(!s.present);
        assert_eq!(s.path, None);
        assert_eq!(s.license, "AGPL-3.0");
    }

    #[test]
    fn version_extraite_proprement() {
        let cas = [
            ("ffmpeg version 8.1.2 Copyright (c) 2000-2026", "8.1.2"),
            ("ffmpeg version 6.1.1-static https://x", "6.1.1-static"),
            ("Version: ImageMagick 6.9.12-98 Q16 x86_64", "6.9.12-98"),
            ("\n7-Zip (z) 26.00 (x64) : Copyright", "26.00"),
            ("10.02.1", "10.02.1"),
            ("2024.04.09", "2024.04.09"),
            ("tesseract 5.3.4\n leptonica-1.84.1", "5.3.4"),
            ("typst 0.15.1 (abcdef12)", "0.15.1"),
            ("qpdf version 11.9.0", "11.9.0"),
            ("whatever v1.9.5", "1.9.5"),
        ];
        for (sortie, attendu) in cas {
            assert_eq!(parse_version(sortie, ""), Some(attendu.into()), "{sortie}");
        }
        // Version sur stderr seulement (anciens Tesseract).
        assert_eq!(parse_version("", "tesseract 4.1.1"), Some("4.1.1".into()));
        // Illisible : jamais de texte brut (aide, build git, ligne 2 « gcc 13.2.0 »).
        for sortie in [
            "usage: whisper-cli [options] file0 file1 ...",
            "ffmpeg version N-113-gabc Copyright (c) 2000-2024\nbuilt with gcc 13.2.0",
            "Python path configuration:",
        ] {
            assert_eq!(parse_version(sortie, ""), None, "{sortie}");
        }
    }

    /// v0.1.0 (VM de vérification) : Tesseract en `symbol lookup error` et
    /// yt-dlp en erreur Python étaient affichés « Présent » avec l'erreur
    /// en guise de version.
    #[cfg(unix)]
    #[test]
    fn binaire_casse_signale_et_non_present() {
        use crate::tools::testutil::fake_bin;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("path");
        fake_bin(
            &path,
            "tesseract",
            "echo 'tesseract: symbol lookup error: /lib/libcurl.so.4: undefined symbol: nghttp2_option_set_no_rfc' >&2; exit 127",
        );
        fake_bin(
            &path,
            "yt-dlp",
            "echo 'Python path configuration:' >&2; echo '  PYTHONHOME = (not set)' >&2; exit 1",
        );
        let r = resolver(tmp.path());
        for b in [Binary::Tesseract, Binary::YtDlp] {
            let s = status(&r, b);
            assert_eq!(s.state, State::Casse, "{s:?}");
            assert!(!s.present);
            assert_eq!(s.version, None);
            assert_eq!(s.source, Some(Source::SystemPath));
            assert!(s.error.as_deref().unwrap().contains("ne s'exécute pas"));
        }
        let s = status(&r, Binary::Tesseract);
        assert!(s.error.unwrap().contains("symbol lookup error"));
        assert_eq!(serde_json::to_value(State::Casse).unwrap(), "casse");
    }

    #[cfg(unix)]
    #[test]
    fn binaire_sain_present_avec_sa_version() {
        use crate::tools::testutil::fake_bin;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("path");
        fake_bin(
            &path,
            "qpdf",
            "echo 'qpdf version 11.9.0'; echo 'Run qpdf --copyright'",
        );
        // `-h` affiche l'aide avec un code non nul (Real-ESRGAN : 255) : sain.
        fake_bin(
            &path,
            "realesrgan-ncnn-vulkan",
            "echo 'Usage: realesrgan-ncnn-vulkan -i infile -o outfile' >&2; exit 255",
        );
        let r = resolver(tmp.path());
        let s = status(&r, Binary::Qpdf);
        assert_eq!((s.state, s.present), (State::Present, true));
        assert_eq!(s.version.as_deref(), Some("11.9.0"));
        assert_eq!(s.error, None);
        let s = status(&r, Binary::RealEsrgan);
        assert_eq!(
            (s.state, s.present, s.version),
            (State::Present, true, None)
        );
    }

    /// Le vrai ffmpeg de la VM : présent, version numérique seule.
    #[test]
    fn diagnostic_du_vrai_ffmpeg() {
        let Some(r) = crate::tools::testutil::real(&[Binary::Ffmpeg]) else {
            return;
        };
        let s = status(&r, Binary::Ffmpeg);
        assert_eq!(s.state, State::Present, "{s:?}");
        let v = s.version.expect("version lue");
        assert!(
            v.starts_with(|c: char| c.is_ascii_digit()) && !v.contains(' '),
            "{v}"
        );
    }
}
