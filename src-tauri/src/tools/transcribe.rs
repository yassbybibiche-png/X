//! Transcription locale avec whisper.cpp (`whisper-cli`) : sous-titres SRT/VTT
//! et texte brut. Aucune donnée ne quitte la machine.
//!
//! Étapes : ffmpeg → WAV 16 kHz mono (dossier temporaire) → `whisper-cli
//! -m <modèle> -f audio.wav -of <tmp>/sortie -osrt|-ovtt|-otxt -pp` → copies
//! dans les sorties réservées à côté de l'original.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};

use super::media::{base_args, run_progress};
use crate::jobs::ToolContext;
use crate::models::{self, Model};
use crate::output::{self, OutputFile};
use crate::sidecar::{Binary, Stream};

/// Langue passée à whisper (`auto` = détection).
pub fn language(value: &str) -> Result<&'static str> {
    Ok(match value {
        "auto" => "auto",
        "fr" => "fr",
        "en" => "en",
        other => bail!("langue inconnue : {other}"),
    })
}

/// Formats demandés → options whisper + extensions.
pub fn formats(value: &str) -> Result<&'static [(&'static str, &'static str)]> {
    Ok(match value {
        "srt" => &[("-osrt", "srt")],
        "vtt" => &[("-ovtt", "vtt")],
        "srt+vtt" => &[("-osrt", "srt"), ("-ovtt", "vtt")],
        "txt" => &[("-otxt", "txt")],
        other => bail!("format de sous-titres inconnu : {other}"),
    })
}

/// « whisper_print_progress_callback: progress =  45% » → 45.
pub fn parse_progress(line: &str) -> Option<u32> {
    let (_, rest) = line.split_once("progress =")?;
    rest.trim().trim_end_matches('%').trim().parse().ok()
}

fn threads() -> String {
    std::thread::available_parallelism()
        .map_or(4, |n| n.get().min(8))
        .to_string()
}

pub fn whisper_args(
    model: &Path,
    wav: &Path,
    base: &Path,
    lang: &str,
    flags: &[&str],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-m".into(),
        model.into(),
        "-f".into(),
        wav.into(),
        "-l".into(),
        lang.into(),
        "-t".into(),
        threads().into(),
        "-pp".into(),
        "-of".into(),
        base.into(),
    ];
    args.extend(flags.iter().map(OsString::from));
    args
}

/// Moteur prêt : binaire + modèle (erreur claire si le modèle manque).
struct Engine {
    exe: PathBuf,
    model: PathBuf,
    lang: &'static str,
}

fn engine(ctx: &ToolContext) -> Result<Engine> {
    let exe = ctx.binary(Binary::Whisper)?;
    let choice =
        Model::from_option(ctx.input.option("modele")).context("modèle whisper inconnu")?;
    let model = models::locate(ctx.resolver, choice, Some(&exe))
        .ok_or_else(|| anyhow::anyhow!(models::missing_message(ctx.resolver, choice)))?;
    Ok(Engine {
        exe,
        model,
        lang: language(ctx.input.option("langue"))?,
    })
}

/// Audio 16 kHz mono (format attendu par whisper) dans `dir`.
fn to_wav(ctx: &ToolContext, input: &Path, dir: &Path) -> Result<PathBuf> {
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let wav = dir.join("audio.wav");
    let mut args = base_args();
    args.extend(["-i".into(), input.as_os_str().to_owned()]);
    args.extend(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"].map(OsString::from));
    args.push(wav.clone().into());
    ctx.reporter.progress(0.0, "Extraction du son…");
    run_progress(ctx, &ffmpeg, &output::file_name(input), &args, (None, None))?;
    Ok(wav)
}

fn run_whisper(
    ctx: &ToolContext,
    e: &Engine,
    wav: &Path,
    base: &Path,
    flags: &[&str],
) -> Result<()> {
    let last = AtomicU32::new(u32::MAX);
    let reporter = ctx.reporter;
    let on_line = |_: Stream, line: &str| {
        if let Some(p) = parse_progress(line)
            && last.swap(p, Ordering::Relaxed) != p
        {
            reporter.progress(p.min(99) as f32, "Transcription…");
        }
    };
    let args = whisper_args(&e.model, wav, base, e.lang, flags);
    ctx.run_sidecar(&e.exe, "whisper", &args, Some(&on_line))?;
    Ok(())
}

/// Transcrit `path` ; une sortie par format demandé (tout ou rien).
fn transcribe_one(
    ctx: &ToolContext,
    e: &Engine,
    path: &Path,
    wanted: &[(&str, &str)],
) -> Result<Vec<PathBuf>> {
    let mut outs: Vec<OutputFile> = wanted
        .iter()
        .map(|(_, ext)| output::beside(path, None, ext))
        .collect::<Result<_>>()?;
    let tmp = tempfile::tempdir().context("dossier temporaire impossible")?;
    let wav = to_wav(ctx, path, tmp.path())?;
    let base = tmp.path().join("sortie");
    let flags: Vec<&str> = wanted.iter().map(|(f, _)| *f).collect();
    run_whisper(ctx, e, &wav, &base, &flags)?;
    for ((_, ext), out) in wanted.iter().zip(outs.iter_mut()) {
        let produced = base.with_extension(ext);
        let text = std::fs::read(&produced)
            .with_context(|| format!("whisper n'a pas produit le fichier .{ext}"))?;
        std::io::Write::write_all(&mut out.take_file()?, &text)?;
    }
    Ok(outs.into_iter().map(OutputFile::commit).collect())
}

fn transcribe(
    ctx: &ToolContext,
    wanted: &'static [(&'static str, &'static str)],
) -> Result<Vec<PathBuf>> {
    let e = engine(ctx)?;
    let mut all = Vec::new();
    for path in &ctx.input.inputs {
        ctx.check_cancel()?;
        all.extend(transcribe_one(ctx, &e, path, wanted)?);
    }
    Ok(all)
}

/// `video-sous-titres` : `<nom>.srt` et/ou `<nom>.vtt`.
pub fn subtitles(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    transcribe(ctx, formats(ctx.input.option("format"))?)
}

/// `audio-transcrire` : `<nom>.txt`.
pub fn transcript(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    transcribe(ctx, formats("txt")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use crate::tools::testutil::real;

    #[test]
    fn options_et_progression() {
        assert_eq!(
            parse_progress("whisper_print_progress_callback: progress =  45%"),
            Some(45)
        );
        assert_eq!(parse_progress("autre"), None);
        assert_eq!(formats("srt+vtt").unwrap().len(), 2);
        assert!(language("de").is_err());
    }

    /// Vrai whisper-cli (installé depuis le manifeste) sans modèle : erreur claire.
    #[test]
    fn modele_absent_message_clair_avec_le_vrai_whisper() {
        let Some(r) = real(&[Binary::Whisper, Binary::Ffmpeg]) else {
            return;
        };
        if models::locate(
            &r,
            Model::WhisperSmall,
            r.resolve(Binary::Whisper).map(|x| x.path).as_deref(),
        )
        .is_some()
        {
            return; // modèle réellement présent : rien à vérifier ici
        }
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("voix.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        let err = execute(
            "audio-transcrire",
            JobInput::new(vec![wav]),
            &MemoryReporter::default(),
            &r,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ggml-small-q5_1.bin") && msg.contains("absent"),
            "{msg}"
        );
        assert!(!tmp.path().join("voix.txt").exists());
    }

    /// Faux whisper-cli + modèle factice : câblage complet (WAV réel via ffmpeg,
    /// arguments, progression, SRT + VTT copiés à côté de la vidéo).
    #[cfg(unix)]
    #[test]
    fn cablage_whisper_avec_faux_binaire() {
        use crate::tools::testutil::fake_bin;
        let Some(real_r) = real(&[Binary::Ffmpeg]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let script = r#"while [ $# -gt 0 ]; do case "$1" in -of) OF="$2";; -f) F="$2";; -l) L="$2";; esac; shift; done
[ -s "$F" ] || exit 4
echo "whisper_print_progress_callback: progress =  50%" >&2
printf '1\n00:00:00,000 --> 00:00:01,000\nBonjour (%s)\n\n' "$L" > "$OF.srt"
printf 'WEBVTT\n\n00:00.000 --> 00:01.000\nBonjour\n' > "$OF.vtt""#;
        fake_bin(&bin, "whisper-cli", script);
        let ffmpeg_dir = real_r
            .resolve(Binary::Ffmpeg)
            .unwrap()
            .path
            .parent()
            .unwrap()
            .to_path_buf();
        let path = std::env::join_paths([bin.clone(), ffmpeg_dir]).unwrap();
        let data = tmp.path().join("donnees/bin");
        let resolver = Resolver::new(None, Some(data.clone()), Some(path));
        let src = tmp.path().join("film.wav");
        let ff = real_r.resolve(Binary::Ffmpeg).unwrap().path;
        let args = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=1",
        ];
        let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
        all.push(src.clone().into());
        crate::sidecar::run(&ff, &all, std::time::Duration::from_secs(30)).unwrap();

        let job = || {
            JobInput::new(vec![src.clone()])
                .with("format", "srt+vtt")
                .with("langue", "fr")
        };
        let err = execute(
            "video-sous-titres",
            job(),
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap_err();
        assert!(err.to_string().contains("absent"), "{err}");

        let model_dir = tmp.path().join("donnees/models/whisper-small");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("ggml-small-q5_1.bin"), b"faux").unwrap();
        let outs = execute(
            "video-sous-titres",
            job(),
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap();
        assert_eq!(
            outs,
            vec![tmp.path().join("film.srt"), tmp.path().join("film.vtt")]
        );
        assert!(
            std::fs::read_to_string(&outs[0])
                .unwrap()
                .contains("Bonjour (fr)")
        );
        assert!(
            std::fs::read_to_string(&outs[1])
                .unwrap()
                .starts_with("WEBVTT")
        );
    }
}
