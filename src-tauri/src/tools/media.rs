//! Outils vidéo et audio avec ffmpeg (appelé directement, jamais via un shell).
//!
//! La sortie est réservée (fichier vide créé par `create_new`) **avant** de
//! lancer ffmpeg, qui l'écrit ensuite avec `-y` : ce `-y` ne peut remplacer que
//! notre propre réservation, jamais un fichier de l'utilisateur.
//! Progression : `-progress pipe:1` (temps encodé) rapporté à la durée lue
//! dans l'en-tête `Duration:` de ffmpeg (ou à la durée de l'extrait).

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::per_file;
use crate::jobs::ToolContext;
use crate::output;
use crate::sidecar::{self, Binary, Stream};

/// Paramètres d'un appel ffmpeg pour un fichier.
#[derive(Debug, Default)]
pub struct Plan {
    pub suffix: Option<String>,
    pub ext: String,
    /// Options avant `-i` (ex. `-ss`).
    pub pre: Vec<String>,
    /// Options après `-i` (codecs, filtres…).
    pub post: Vec<String>,
    /// Durée attendue de la sortie (s), si connue.
    pub span: Option<f64>,
    /// Dossier de travail de ffmpeg (ex. sous-titres référencés par un nom relatif).
    pub dir: Option<PathBuf>,
}

/// Arguments complets ; la sortie est toujours le dernier argument.
pub fn ffmpeg_args(plan: &Plan, input: &Path, output: &Path) -> Vec<OsString> {
    let mut args = base_args();
    args.extend(plan.pre.iter().map(OsString::from));
    args.push("-i".into());
    args.push(input.into());
    args.extend(plan.post.iter().map(OsString::from));
    args.push(output.into());
    args
}

pub(crate) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Secondes depuis « 90 », « 1:30 », « 00:01:30.5 ».
pub fn parse_time(value: &str) -> Result<f64> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() > 3 || value.trim().is_empty() {
        bail!("instant invalide : « {value} » (ex. 90, 1:30 ou 00:01:30)");
    }
    parts.iter().try_fold(0.0, |acc, p| {
        let v: f64 = p
            .trim()
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite() && *v >= 0.0)
            .with_context(|| format!("instant invalide : « {value} »"))?;
        Ok(acc * 60.0 + v)
    })
}

/// « Duration: 00:01:02.50, … » → secondes.
pub fn parse_duration_line(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("Duration:")?;
    parse_time(rest.split(',').next()?.trim()).ok()
}

/// Exécute ffmpeg avec progression (pourcentage entier, sans doublons).
fn run_ffmpeg(
    ctx: &ToolContext,
    ffmpeg: &Path,
    input: &Path,
    plan: &Plan,
    out: &Path,
) -> Result<()> {
    let args = ffmpeg_args(plan, input, out);
    let name = output::file_name(input);
    run_progress(ctx, ffmpeg, &name, &args, (plan.span, plan.dir.as_deref()))
}

/// Exécute ffmpeg (arguments complets, `-progress pipe:1` inclus) ; `span` =
/// durée attendue de la sortie (sinon lue dans l'en-tête `Duration:`).
pub(crate) fn run_progress(
    ctx: &ToolContext,
    ffmpeg: &Path,
    name: &str,
    args: &[OsString],
    (span, dir): (Option<f64>, Option<&Path>),
) -> Result<()> {
    let total_us = AtomicU64::new(span.map_or(0, |s| (s * 1e6) as u64));
    let last = AtomicU32::new(u32::MAX);
    let reporter = ctx.reporter;
    let on_line = |stream: Stream, line: &str| {
        if stream == Stream::Stderr {
            if let Some(d) = parse_duration_line(line) {
                let _ = total_us.compare_exchange(
                    0,
                    (d * 1e6) as u64,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            return;
        }
        let Some(done) = line
            .strip_prefix("out_time_us=")
            .and_then(|v| v.trim().parse::<u64>().ok())
        else {
            return;
        };
        let total = total_us.load(Ordering::Relaxed);
        if total > 0 {
            let pct = (done as f64 * 100.0 / total as f64).min(99.0) as u32;
            if last.swap(pct, Ordering::Relaxed) != pct {
                reporter.progress(pct as f32, &format!("Traitement de {name}…"));
            }
        }
    };
    ctx.reporter
        .progress(0.0, &format!("Traitement de {name}…"));
    let opts = sidecar::RunOptions {
        current_dir: dir,
        ..ctx.run_options(Some(&on_line))
    };
    sidecar::run_with(ffmpeg, args, opts)?.check("ffmpeg")?;
    Ok(())
}

/// Début des arguments ffmpeg communs (avant les entrées).
pub(crate) fn base_args() -> Vec<OsString> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

/// `ffprobe -show_format -show_streams` en JSON (None si ffprobe est absent).
pub(crate) fn probe(ctx: &ToolContext, ffmpeg: &Path, path: &Path) -> Result<Option<Value>> {
    let Some(ffprobe) = sidecar::sibling(ffmpeg, "ffprobe") else {
        return Ok(None);
    };
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-show_format",
        "-show_streams",
        "-of",
        "json",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(path.into());
    let run = ctx.run_sidecar(&ffprobe, "ffprobe", &args, None)?;
    Ok(Some(
        serde_json::from_str(&run.stdout).context("réponse de ffprobe illisible")?,
    ))
}

/// Durée (s) d'après ffprobe.
pub(crate) fn probe_duration(json: &Value) -> Option<f64> {
    seconds(&json["format"], "duration")
}

/// Applique le plan ffmpeg de chaque fichier ; sortie réservée avant l'appel.
pub(crate) fn media_each(
    ctx: &ToolContext,
    plan_for: impl Fn(&Path) -> Result<Plan>,
) -> Result<Vec<PathBuf>> {
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    per_file(ctx, |path| {
        ctx.check_cancel()?;
        let plan = plan_for(path)?;
        let out = output::beside(path, plan.suffix.as_deref(), &plan.ext)?;
        run_ffmpeg(ctx, &ffmpeg, path, &plan, out.path())?;
        Ok(out.commit())
    })
}

pub(crate) fn input_ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "mkv".into())
}

/// Codecs vidéo + audio d'un conteneur de sortie.
pub fn video_codecs(format: &str) -> Result<Vec<String>> {
    let h264 = [
        "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p", "-c:a", "aac",
        "-b:a", "160k",
    ];
    let mut args = match format {
        "mp4" | "mov" | "mkv" => strings(&h264),
        "webm" => strings(&[
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "32",
            "-b:v",
            "0",
            "-row-mt",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "128k",
        ]),
        other => bail!("format vidéo inconnu : {other}"),
    };
    if matches!(format, "mp4" | "mov") {
        args.extend(strings(&["-movflags", "+faststart"]));
    }
    Ok(args)
}

/// Codec audio (sans vidéo) d'un format de sortie.
pub fn audio_codec(format: &str) -> Result<Vec<String>> {
    Ok(strings(match format {
        "mp3" => &["-vn", "-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" => &["-vn", "-c:a", "aac", "-b:a", "192k"],
        "ogg" => &["-vn", "-c:a", "libvorbis", "-q:a", "5"],
        "opus" => &["-vn", "-c:a", "libopus", "-b:a", "128k"],
        "wav" => &["-vn", "-c:a", "pcm_s16le"],
        "flac" => &["-vn", "-c:a", "flac"],
        other => bail!("format audio inconnu : {other}"),
    }))
}

/// `video-convertir`.
pub fn convert_video(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let format = ctx.input.option("format").to_string();
    let post = video_codecs(&format)?;
    media_each(ctx, |_| {
        Ok(Plan {
            ext: format.clone(),
            post: post.clone(),
            ..Plan::default()
        })
    })
}

/// `video-compresser` : H.264, CRF réglable (18 = quasi sans perte, 35 = très léger).
pub fn compress_video(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let crf: u32 = ctx
        .input
        .option("crf")
        .trim()
        .parse()
        .ok()
        .filter(|c| (18..=40).contains(c))
        .context("qualité CRF invalide (18 à 40)")?;
    let mut post = video_codecs("mp4")?;
    let i = post.iter().position(|a| a == "-crf").map_or(0, |i| i + 1);
    post[i] = crf.to_string();
    media_each(ctx, |_| {
        Ok(Plan {
            suffix: Some("compresse".into()),
            ext: "mp4".into(),
            post: post.clone(),
            ..Plan::default()
        })
    })
}

/// Début, durée (si fin indiquée) et mode de coupe.
pub fn cut_plan(start: &str, end: &str, precision: &str, ext: String) -> Result<Plan> {
    let from = if start.trim().is_empty() {
        0.0
    } else {
        parse_time(start)?
    };
    let span = if end.trim().is_empty() {
        None
    } else {
        let to = parse_time(end)?;
        if to <= from {
            bail!("la fin doit être après le début");
        }
        Some(to - from)
    };
    let mut post = Vec::new();
    if let Some(s) = span {
        post.extend(["-t".to_string(), format!("{s:.3}")]);
    }
    match precision {
        "rapide" => post.extend(strings(&["-c", "copy", "-avoid_negative_ts", "make_zero"])),
        "precise" => {}
        other => bail!("mode de coupe inconnu : {other}"),
    }
    Ok(Plan {
        suffix: Some("extrait".into()),
        ext,
        pre: vec!["-ss".into(), format!("{from:.3}")],
        post,
        span,
        dir: None,
    })
}

/// `video-couper` et `audio-couper` (format conservé).
pub fn cut(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (start, end) = (ctx.input.option("debut"), ctx.input.option("fin"));
    let precision = ctx.input.option("precision");
    cut_plan(start, end, precision, String::new())?; // validé avant tout fichier
    media_each(ctx, |path| cut_plan(start, end, precision, input_ext(path)))
}

/// `video-extraire-audio` et `audio-convertir`.
pub fn to_audio(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let format = ctx.input.option("format").to_string();
    let post = audio_codec(&format)?;
    media_each(ctx, |_| {
        Ok(Plan {
            ext: format.clone(),
            post: post.clone(),
            ..Plan::default()
        })
    })
}

/// `video-supprimer-son` : vidéo copiée sans réencodage, pistes audio retirées.
pub fn mute(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    media_each(ctx, |path| {
        Ok(Plan {
            suffix: Some("muet".into()),
            ext: input_ext(path),
            post: strings(&["-map", "0", "-map", "-0:a", "-c", "copy"]),
            ..Plan::default()
        })
    })
}

/// `video-extraire-image` : PNG de l'image à l'instant donné.
pub fn frame(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let at = parse_time(ctx.input.option("instant"))?;
    let label = format!(
        "t{}s",
        format!("{at:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
    );
    media_each(ctx, |_| {
        Ok(Plan {
            suffix: Some(label.clone()),
            ext: "png".into(),
            pre: vec!["-ss".into(), format!("{at:.3}")],
            post: strings(&["-frames:v", "1", "-update", "1"]),
            ..Plan::default()
        })
    })
}

/// `video-infos` : rapport texte (ffprobe, sinon en-tête de ffmpeg).
pub fn info(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let ffprobe = sidecar::sibling(&ffmpeg, "ffprobe");
    per_file(ctx, |path| {
        let mut out = output::beside(path, Some("infos"), "txt")?;
        let report = match &ffprobe {
            Some(probe) => probe_report(ctx, probe, path)?,
            None => header_report(ctx, &ffmpeg, path)?,
        };
        out.take_file()?
            .write_all(report.as_bytes())
            .context("écriture du rapport impossible")?;
        Ok(out.commit())
    })
}

fn probe_report(ctx: &ToolContext, ffprobe: &Path, path: &Path) -> Result<String> {
    let args: Vec<OsString> = [
        "-v",
        "error",
        "-show_format",
        "-show_streams",
        "-of",
        "json",
    ]
    .iter()
    .map(OsString::from)
    .chain([path.as_os_str().to_owned()])
    .collect();
    let run = ctx.run_sidecar(ffprobe, "ffprobe", &args, None)?;
    let json: Value = serde_json::from_str(&run.stdout).context("réponse de ffprobe illisible")?;
    render_probe(&output::file_name(path), &json)
}

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?
        .as_str()
        .filter(|s| !s.is_empty() && *s != "N/A")
}

fn seconds(v: &Value, key: &str) -> Option<f64> {
    field(v, key)?.parse().ok()
}

pub fn render_probe(name: &str, json: &Value) -> Result<String> {
    let mut r = String::new();
    let format = &json["format"];
    writeln!(r, "Fichier : {name}")?;
    if let Some(f) = field(format, "format_long_name").or(field(format, "format_name")) {
        writeln!(r, "Conteneur : {f}")?;
    }
    if let Some(d) = seconds(format, "duration") {
        writeln!(r, "Durée : {} ({d:.2} s)", hms(d))?;
    }
    if let Some(s) = field(format, "size") {
        writeln!(r, "Taille : {s} octets")?;
    }
    if let Some(b) = seconds(format, "bit_rate") {
        writeln!(r, "Débit : {:.0} kb/s", b / 1000.0)?;
    }
    for (i, s) in json["streams"].as_array().into_iter().flatten().enumerate() {
        writeln!(r, "Piste {i} : {}", stream_line(s))?;
    }
    Ok(r)
}

fn stream_line(s: &Value) -> String {
    let codec = field(s, "codec_name").unwrap_or("?");
    let mut line = match field(s, "codec_type") {
        Some("video") => format!(
            "vidéo {codec}, {}×{} px, {} im/s",
            s["width"],
            s["height"],
            field(s, "avg_frame_rate").map_or("?".into(), frame_rate)
        ),
        Some("audio") => format!(
            "audio {codec}, {} Hz, {} canal/canaux",
            field(s, "sample_rate").unwrap_or("?"),
            s["channels"]
        ),
        Some(other) => format!("{other} {codec}"),
        None => format!("inconnue {codec}"),
    };
    if let Some(lang) = s["tags"].get("language").and_then(Value::as_str) {
        let _ = write!(line, " [{lang}]");
    }
    line
}

fn frame_rate(rate: &str) -> String {
    match rate.split_once('/') {
        Some((n, d)) => match (n.parse::<f64>(), d.parse::<f64>()) {
            (Ok(n), Ok(d)) if d > 0.0 => format!("{:.2}", n / d),
            _ => rate.to_string(),
        },
        None => rate.to_string(),
    }
}

fn hms(secs: f64) -> String {
    let t = secs.round() as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

/// Secours sans ffprobe : bloc « Input #0 » de `ffmpeg -i` (qui sort en code 1).
fn header_report(ctx: &ToolContext, ffmpeg: &Path, path: &Path) -> Result<String> {
    let args = [OsString::from("-hide_banner"), "-i".into(), path.into()];
    let opts = sidecar::RunOptions {
        cancel: Some(ctx.cancel),
        ..sidecar::RunOptions::timeout(std::time::Duration::from_secs(60))
    };
    let run = sidecar::run_with(ffmpeg, &args, opts)?;
    let lines: Vec<&str> = run
        .stderr
        .lines()
        .skip_while(|l| !l.starts_with("Input #"))
        .take_while(|l| !l.starts_with("At least one output"))
        .collect();
    if lines.is_empty() {
        bail!(
            "fichier média illisible : {}",
            sidecar::truncate_stderr(&run.stderr)
        );
    }
    Ok(format!(
        "Fichier : {}\n{}\n",
        output::file_name(path),
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;

    #[test]
    fn instants() {
        assert_eq!(parse_time("90").unwrap(), 90.0);
        assert_eq!(parse_time("1:30").unwrap(), 90.0);
        assert_eq!(parse_time("00:01:30.5").unwrap(), 90.5);
        for bad in ["", "abc", "-1", "1:2:3:4"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_duration_line("  Duration: 00:01:02.50, start: 0.0"),
            Some(62.5)
        );
    }

    #[test]
    fn arguments_ffmpeg() {
        let plan = cut_plan("1:00", "1:30", "rapide", "mp4".into()).unwrap();
        let args = ffmpeg_args(&plan, Path::new("in.mp4"), Path::new("out.mp4"));
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into()).collect();
        let pos = |s: &str| args.iter().position(|a| a == s).unwrap();
        assert!(pos("-ss") < pos("-i") && pos("-i") < pos("-t"));
        assert_eq!(args[pos("-t") + 1], "30.000");
        assert_eq!(args.last().unwrap(), "out.mp4");
        assert!(args.contains(&"-nostdin".to_string()));
        assert!(cut_plan("10", "5", "precise", String::new()).is_err());
        assert!(video_codecs("avi").is_err() && audio_codec("aiff").is_err());
    }

    #[test]
    fn rapport_ffprobe() {
        let json: Value = serde_json::from_str(
            r#"{"format":{"format_name":"mov,mp4","duration":"62.5","size":"1000","bit_rate":"128000"},
                "streams":[{"codec_type":"video","codec_name":"h264","width":640,"height":360,"avg_frame_rate":"25/1"},
                           {"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2,"tags":{"language":"fra"}}]}"#,
        )
        .unwrap();
        let r = render_probe("film.mp4", &json).unwrap();
        assert!(r.contains("Durée : 00:01:03 (62.50 s)"), "{r}");
        assert!(r.contains("vidéo h264, 640×360 px, 25.00 im/s"), "{r}");
        assert!(
            r.contains("audio aac, 48000 Hz, 2 canal/canaux [fra]"),
            "{r}"
        );
    }

    // ---- Tests de bout en bout avec le vrai ffmpeg (ignorés s'il est absent) ----

    fn real_ffmpeg() -> Option<(Resolver, PathBuf)> {
        let resolver = Resolver::new(None, None, std::env::var_os("PATH"));
        let path = resolver.resolve(Binary::Ffmpeg)?.path;
        Some((resolver, path))
    }

    /// Clip de 2 s, 64×48, mire + son sinusoïdal.
    fn clip(ffmpeg: &Path, dir: &Path) -> PathBuf {
        let out = dir.join("clip.mp4");
        let args = [
            "-hide_banner",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ];
        let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
        all.push(out.clone().into());
        sidecar::run(ffmpeg, &all, std::time::Duration::from_secs(60))
            .unwrap()
            .check("ffmpeg")
            .unwrap();
        out
    }

    fn probe(path: &Path) -> Value {
        let (_, ffmpeg) = real_ffmpeg().unwrap();
        let ffprobe = sidecar::sibling(&ffmpeg, "ffprobe").expect("ffprobe");
        let args = [
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ];
        let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
        all.push(path.into());
        let run = sidecar::run(&ffprobe, &all, std::time::Duration::from_secs(30)).unwrap();
        serde_json::from_str(&run.stdout).unwrap()
    }

    fn duration(path: &Path) -> f64 {
        seconds(&probe(path)["format"], "duration").unwrap()
    }

    fn stream_types(path: &Path) -> Vec<String> {
        probe(path)["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["codec_type"].as_str().unwrap().to_string())
            .collect()
    }

    fn run(id: &str, job: JobInput, resolver: &Resolver) -> Vec<PathBuf> {
        execute(id, job, &MemoryReporter::default(), resolver)
            .unwrap_or_else(|e| panic!("{id} : {e:#}"))
    }

    #[test]
    fn bout_en_bout_avec_ffmpeg() {
        let Some((resolver, ffmpeg)) = real_ffmpeg() else {
            eprintln!("ffmpeg absent : test de bout en bout ignoré");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = clip(&ffmpeg, tmp.path());
        let job = |k: &str, v: &str| JobInput::new(vec![src.clone()]).with(k, v);

        let mkv = run("video-convertir", job("format", "mkv"), &resolver);
        assert_eq!(mkv, vec![tmp.path().join("clip.mkv")]);
        assert_eq!(stream_types(&mkv[0]), ["video", "audio"]);

        let small = run("video-compresser", job("crf", "35"), &resolver);
        assert!(small[0].ends_with("clip-compresse.mp4"));

        let cut = run(
            "video-couper",
            job("debut", "0.5").with("fin", "1.5"),
            &resolver,
        );
        assert!(
            (duration(&cut[0]) - 1.0).abs() < 0.25,
            "{}",
            duration(&cut[0])
        );

        let mp3 = run("video-extraire-audio", job("format", "mp3"), &resolver);
        assert_eq!(stream_types(&mp3[0]), ["audio"]);

        let png = run("video-extraire-image", job("instant", "1"), &resolver);
        assert!(png[0].ends_with("clip-t1s.png"));
        let img = image::open(&png[0]).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));

        let muet = run(
            "video-supprimer-son",
            JobInput::new(vec![src.clone()]),
            &resolver,
        );
        assert_eq!(stream_types(&muet[0]), ["video"]);

        let infos = run("video-infos", JobInput::new(vec![src.clone()]), &resolver);
        let text = std::fs::read_to_string(&infos[0]).unwrap();
        assert!(
            text.contains("vidéo h264, 64×48 px") && text.contains("audio aac"),
            "{text}"
        );

        let ogg = run(
            "audio-convertir",
            JobInput::new(mp3.clone()).with("format", "ogg"),
            &resolver,
        );
        assert_eq!(stream_types(&ogg[0]), ["audio"]);
        let short = run(
            "audio-couper",
            JobInput::new(mp3).with("fin", "1"),
            &resolver,
        );
        assert!(short[0].ends_with("clip-extrait.mp3"));
        assert!(duration(&short[0]) < 1.3);
    }

    #[test]
    fn entree_invalide_erreur_typee_sans_sortie() {
        let Some((resolver, _)) = real_ffmpeg() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("faux.mp4");
        std::fs::write(&bad, b"pas une video").unwrap();
        let job = JobInput::new(vec![bad]).with("format", "mkv");
        let err = execute(
            "video-convertir",
            job,
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("ffmpeg a échoué"), "{err:#}");
        assert!(!tmp.path().join("faux.mkv").exists());
    }
}
