//! Téléchargement vidéo / audio avec yt-dlp (seul outil qui accède au réseau,
//! à la demande explicite de l'utilisateur).
//!
//! - URL validée (http/https uniquement, voir [`crate::ytdlp::validate_url`]),
//!   passée après `--` : jamais interprétée comme une option.
//! - `--ignore-config` : aucune configuration utilisateur/système (qui pourrait
//!   contenir `--exec`) n'est chargée. `--no-playlist` par défaut.
//! - yt-dlp écrit dans un dossier de travail privé créé **dans** le dossier de
//!   destination (même disque) ; chaque fichier final est ensuite placé sous un
//!   nom libre réservé (`create_new`) : jamais d'écrasement.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};

use crate::jobs::ToolContext;
use crate::output;
use crate::sidecar::{Binary, Stream};
use crate::ytdlp::validate_url;

/// Sélecteur de format yt-dlp d'une qualité vidéo.
pub fn video_format(quality: &str) -> Result<String> {
    Ok(match quality {
        "meilleure" => "bv*+ba/b".into(),
        h @ ("1080" | "720" | "480" | "360") => format!("bv*[height<={h}]+ba/b[height<={h}]"),
        other => bail!("qualité inconnue : {other}"),
    })
}

pub fn audio_format(format: &str) -> Result<&'static str> {
    Ok(match format {
        "mp3" => "mp3",
        "m4a" => "m4a",
        "opus" => "opus",
        other => bail!("format audio inconnu : {other}"),
    })
}

/// Dossier de destination : option `dossier`, sinon Téléchargements.
fn destination(ctx: &ToolContext) -> Result<PathBuf> {
    let chosen = ctx.input.option("dossier").trim();
    let dir = if chosen.is_empty() {
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .context("dossier Téléchargements introuvable")?
    } else {
        PathBuf::from(chosen)
    };
    if !dir.is_dir() {
        bail!("dossier de destination introuvable : {}", dir.display());
    }
    Ok(dir)
}

/// Arguments communs (sans format) ; l'URL est toujours le dernier, après `--`.
pub fn base_args(work: &Path, ffmpeg_dir: Option<&Path>, playlist: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--ignore-config",
        "--newline",
        "--no-mtime",
        "--windows-filenames",
        "--no-part",
        "-o",
        "%(title).150B [%(id)s].%(ext)s",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(
        if playlist {
            "--yes-playlist"
        } else {
            "--no-playlist"
        }
        .into(),
    );
    args.push("-P".into());
    args.push(work.into());
    if let Some(dir) = ffmpeg_dir {
        args.push("--ffmpeg-location".into());
        args.push(dir.into());
    }
    args
}

/// « [download]  45.3% of … » → 45.
pub fn parse_progress(line: &str) -> Option<u32> {
    let rest = line.trim().strip_prefix("[download]")?.trim_start();
    let pct = rest.split('%').next()?.trim();
    pct.parse::<f32>().ok().map(|p| p as u32)
}

fn run(ctx: &ToolContext, mut args: Vec<OsString>, url: &str) -> Result<Vec<PathBuf>> {
    let exe = ctx.binary(Binary::YtDlp)?;
    let dest = destination(ctx)?;
    let work = tempfile::Builder::new()
        .prefix(".boite-a-outils-")
        .tempdir_in(&dest)
        .context("dossier de travail impossible dans le dossier de destination")?;
    let ffmpeg_dir = ctx
        .binary(Binary::Ffmpeg)
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let playlist = ctx.input.option("playlist") == "true";
    let mut all = base_args(work.path(), ffmpeg_dir.as_deref(), playlist);
    all.append(&mut args);
    all.extend(["--".into(), url.into()]);
    let last = AtomicU32::new(u32::MAX);
    let reporter = ctx.reporter;
    let on_line = |s: Stream, line: &str| match (s, parse_progress(line)) {
        (_, Some(p)) if last.swap(p, Ordering::Relaxed) != p => {
            reporter.progress(p.min(99) as f32, "Téléchargement…")
        }
        (Stream::Stderr, None) if line.starts_with("WARNING") => reporter.warn(line),
        _ => {}
    };
    ctx.reporter.log(&format!("yt-dlp : {url}"));
    ctx.run_sidecar(&exe, "yt-dlp", &all, Some(&on_line))?;
    place_all(work.path(), &dest)
}

/// Place chaque fichier téléchargé dans `dest` sous un nom libre réservé.
fn place_all(work: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(work)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("yt-dlp n'a produit aucun fichier");
    }
    files.iter().map(|f| place(f, dest)).collect()
}

fn place(file: &Path, dest: &Path) -> Result<PathBuf> {
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    output::fill_from(output::reserve(dest, &output::stem(file), &ext)?, file)
}

/// `video-telecharger`.
pub fn download_video(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let url = validate_url(ctx.input.option("url"))?;
    let format = video_format(ctx.input.option("qualite"))?;
    let args = vec![
        "-f".into(),
        format.into(),
        "--merge-output-format".into(),
        "mp4/mkv".into(),
    ];
    run(ctx, args, &url)
}

/// `audio-telecharger`.
pub fn download_audio(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let url = validate_url(ctx.input.option("url"))?;
    let format = audio_format(ctx.input.option("format"))?;
    let args = [
        "-f",
        "ba/b",
        "-x",
        "--audio-format",
        format,
        "--audio-quality",
        "0",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    run(ctx, args, &url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;

    #[test]
    fn formats_et_progression() {
        assert_eq!(
            video_format("720").unwrap(),
            "bv*[height<=720]+ba/b[height<=720]"
        );
        assert!(video_format("4k").is_err() && audio_format("wav").is_err());
        assert_eq!(
            parse_progress("[download]  45.3% of 10.00MiB at 1MiB/s"),
            Some(45)
        );
        assert_eq!(parse_progress("[info] x"), None);
        let args = base_args(Path::new("w"), None, false);
        assert!(args.contains(&"--ignore-config".into()) && args.contains(&"--no-playlist".into()));
    }

    /// Faux yt-dlp : vérifie les arguments de sécurité et écrit un fichier dans -P.
    #[cfg(unix)]
    #[test]
    fn cablage_yt_dlp_avec_faux_binaire() {
        use crate::tools::testutil::fake_bin;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let log = tmp.path().join("args.txt");
        let script = format!(
            r#"printf '%s\n' "$@" > '{}'
while [ $# -gt 0 ]; do case "$1" in -P) P="$2";; esac; shift; done
echo "[download]  50.0% of 1.00MiB"
echo "contenu" > "$P/Ma vidéo [abc].mp4""#,
            log.display()
        );
        fake_bin(&bin, "yt-dlp", &script);
        fake_bin(&bin, "ffmpeg", "exit 0");
        let resolver = Resolver::new(None, None, Some(bin.into_os_string()));
        let dest = tmp.path().join("Téléchargements");
        std::fs::create_dir(&dest).unwrap();
        for bad in ["", "file:///etc/passwd", "--exec=rm"] {
            let job = JobInput::default()
                .with("url", bad)
                .with("dossier", &dest.to_string_lossy());
            assert!(
                execute(
                    "video-telecharger",
                    job,
                    &MemoryReporter::default(),
                    &resolver
                )
                .is_err()
            );
            assert!(
                !log.exists(),
                "yt-dlp ne doit pas être lancé pour « {bad} »"
            );
        }
        std::fs::write(dest.join("Ma vidéo [abc].mp4"), b"existant").unwrap();
        let job = JobInput::default()
            .with("url", "https://www.youtube.com/watch?v=abc")
            .with("dossier", &dest.to_string_lossy());
        let outs = execute(
            "video-telecharger",
            job,
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap();
        assert_eq!(outs, vec![dest.join("Ma vidéo [abc]-2.mp4")]);
        assert_eq!(
            std::fs::read(dest.join("Ma vidéo [abc].mp4")).unwrap(),
            b"existant"
        );
        let args = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = args.lines().collect();
        assert_eq!(
            lines[lines.len() - 2..],
            ["--", "https://www.youtube.com/watch?v=abc"]
        );
        assert!(lines.contains(&"--ignore-config") && lines.contains(&"--no-playlist"));
        // Le dossier de travail a été supprimé.
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 2);
    }
}
