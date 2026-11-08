//! Outils ffmpeg (suite) : GIF animé, redimensionner, pivoter, accélérer,
//! normaliser, incruster / extraire des sous-titres, concaténer, planche de
//! vignettes. Mêmes règles que `media.rs` : sortie réservée avant l'appel,
//! jamais de shell, filtres construits à partir de valeurs validées.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::media::{
    Plan, audio_codec, base_args, input_ext, media_each, parse_time, probe, probe_duration,
    run_progress, strings, video_codecs,
};
use super::per_file;
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};
use crate::sidecar::Binary;

/// Début et durée facultative (`fin` vide = jusqu'au bout).
fn span(start: &str, end: &str) -> Result<(f64, Option<f64>)> {
    let from = if start.trim().is_empty() {
        0.0
    } else {
        parse_time(start)?
    };
    if end.trim().is_empty() {
        return Ok((from, None));
    }
    let to = parse_time(end)?;
    if to <= from {
        bail!("la fin doit être après le début");
    }
    Ok((from, Some(to - from)))
}

fn number<T: std::str::FromStr + PartialOrd>(
    ctx: &ToolContext,
    key: &str,
    (min, max): (T, T),
) -> Result<T> {
    let raw = ctx.input.option(key);
    raw.trim()
        .parse::<T>()
        .ok()
        .filter(|v| *v >= min && *v <= max)
        .with_context(|| format!("valeur invalide pour « {key} » : « {raw} »"))
}

/// Filtre GIF en une passe : palette optimisée (palettegen + paletteuse).
pub fn gif_filter(fps: u32, width: u32) -> String {
    format!(
        "fps={fps},scale={width}:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"
    )
}

/// `video-vers-gif`.
pub fn to_gif(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (from, dur) = span(ctx.input.option("debut"), ctx.input.option("fin"))?;
    let fps = number(ctx, "fps", (1u32, 30))?;
    let width = number(ctx, "largeur", (32u32, 1920))?;
    let mut post = vec![
        "-vf".to_string(),
        gif_filter(fps, width),
        "-loop".into(),
        "0".into(),
    ];
    if let Some(d) = dur {
        post.splice(0..0, ["-t".to_string(), format!("{d:.3}")]);
    }
    media_each(ctx, |_| {
        Ok(Plan {
            ext: "gif".into(),
            pre: vec!["-ss".into(), format!("{from:.3}")],
            post: post.clone(),
            span: dur,
            ..Plan::default()
        })
    })
}

/// Conteneur de sortie : celui de l'entrée s'il est pris en charge, sinon MP4.
fn video_ext(path: &Path) -> String {
    let ext = input_ext(path);
    if matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "webm") {
        ext
    } else {
        "mp4".into()
    }
}

/// Réencode chaque vidéo avec le filtre vidéo `vf` (codecs selon le conteneur).
fn reencode_each(ctx: &ToolContext, suffix: String, vf: String) -> Result<Vec<PathBuf>> {
    media_each(ctx, |path| {
        let ext = video_ext(path);
        let mut post = vec!["-vf".to_string(), vf.clone()];
        post.extend(video_codecs(&ext)?);
        Ok(Plan {
            suffix: Some(suffix.clone()),
            ext,
            post,
            ..Plan::default()
        })
    })
}

/// `video-redimensionner` : hauteur cible, jamais d'agrandissement, largeur paire.
pub fn resize(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let h = number(ctx, "hauteur", (144u32, 4320))?;
    reencode_each(ctx, format!("{h}p"), format!("scale=-2:min({h}\\,ih)"))
}

pub fn rotate_filter(angle: &str) -> Result<&'static str> {
    Ok(match angle {
        "90" => "transpose=clock",
        "180" => "hflip,vflip",
        "270" => "transpose=cclock",
        other => bail!("angle invalide : {other} (90, 180 ou 270)"),
    })
}

/// `video-pivoter`.
pub fn rotate(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let vf = rotate_filter(ctx.input.option("angle"))?;
    reencode_each(ctx, "pivote".into(), vf.into())
}

/// Chaîne `atempo` (chaque filtre accepte 0,5 à 2).
pub fn atempo_chain(mut factor: f64) -> String {
    let mut parts = Vec::new();
    while factor > 2.0 {
        parts.push("atempo=2".to_string());
        factor /= 2.0;
    }
    while factor < 0.5 {
        parts.push("atempo=0.5".to_string());
        factor /= 0.5;
    }
    parts.push(format!("atempo={factor:.4}"));
    parts.join(",")
}

/// `video-accelerer` : image (setpts) et son (atempo) au même facteur.
pub fn speed(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let factor = number(ctx, "facteur", (0.25f64, 4.0))?;
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let label = format!("x{}", format!("{factor}").replace('.', ","));
    media_each(ctx, |path| {
        let info = probe(ctx, &ffmpeg, path)?;
        let has_audio = info.as_ref().is_none_or(|j| has_stream(j, "audio"));
        let ext = video_ext(path);
        let mut post = vec!["-filter:v".to_string(), format!("setpts=PTS/{factor}")];
        if has_audio {
            post.extend(["-filter:a".to_string(), atempo_chain(factor)]);
        }
        post.extend(video_codecs(&ext)?);
        Ok(Plan {
            suffix: Some(label.clone()),
            ext,
            post,
            span: info.as_ref().and_then(probe_duration).map(|d| d / factor),
            ..Plan::default()
        })
    })
}

fn has_stream(json: &Value, kind: &str) -> bool {
    streams(json).any(|s| s["codec_type"] == kind)
}

fn streams(json: &Value) -> impl Iterator<Item = &Value> {
    json["streams"].as_array().into_iter().flatten()
}

/// `audio-normaliser` : EBU R128 (loudnorm, −16 LUFS), format conservé si possible.
pub fn normalize(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    media_each(ctx, |path| {
        let mut ext = input_ext(path);
        if audio_codec(&ext).is_err() {
            ext = "m4a".into();
        }
        let mut post = strings(&["-af", "loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000"]);
        post.extend(audio_codec(&ext)?);
        Ok(Plan {
            suffix: Some("normalise".into()),
            ext,
            post,
            ..Plan::default()
        })
    })
}

const SUB_EXT: [&str; 3] = ["srt", "vtt", "ass"];

fn is_subtitle(p: &Path) -> bool {
    SUB_EXT.contains(&input_ext(p).as_str())
}

/// `video-incruster-sous-titres` : une vidéo + un fichier .srt/.vtt/.ass.
/// Le fichier de sous-titres est copié dans un dossier temporaire et désigné
/// par un nom fixe relatif : aucun chemin utilisateur dans la chaîne de filtre
/// (dont la syntaxe d'échappement varie selon les caractères et l'OS).
pub fn burn_subtitles(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (subs, videos): (Vec<&PathBuf>, Vec<&PathBuf>) =
        ctx.input.inputs.iter().partition(|p| is_subtitle(p));
    let ([sub], [video]) = (subs.as_slice(), videos.as_slice()) else {
        bail!("choisissez exactement une vidéo et un fichier de sous-titres (.srt, .vtt ou .ass)");
    };
    let tmp = tempfile::tempdir()?;
    let name = format!("sous-titres.{}", input_ext(sub));
    std::fs::copy(sub, tmp.path().join(&name)).context("lecture des sous-titres impossible")?;
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let out = output::beside(video, Some("soustitre"), "mp4")?;
    let mut post = vec!["-vf".to_string(), format!("subtitles={name}")];
    post.extend(video_codecs("mp4")?);
    let plan = Plan {
        post,
        dir: Some(tmp.path().to_path_buf()),
        ..Plan::default()
    };
    let args = super::media::ffmpeg_args(
        &plan,
        &std::path::absolute(video)?,
        &std::path::absolute(out.path())?,
    );
    run_progress(
        ctx,
        &ffmpeg,
        &output::file_name(video),
        &args,
        (None, Some(tmp.path())),
    )?;
    Ok(vec![out.commit()])
}

/// Codec texte → (codec de sortie, extension) ; `None` = sous-titres image.
pub fn subtitle_target(codec: &str) -> Option<(&'static str, &'static str)> {
    match codec {
        "subrip" | "mov_text" | "text" | "srt" => Some(("srt", "srt")),
        "webvtt" => Some(("webvtt", "vtt")),
        "ass" | "ssa" => Some(("copy", "ass")),
        _ => None,
    }
}

/// `video-extraire-sous-titres` : chaque piste texte → `<nom>-st<N>[-langue].srt`.
pub fn extract_subtitles(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let mut all = Vec::new();
    for path in &ctx.input.inputs {
        ctx.check_cancel()?;
        let json =
            probe(ctx, &ffmpeg, path)?.context("ffprobe (fourni avec ffmpeg) est nécessaire")?;
        all.extend(extract_tracks(ctx, &ffmpeg, path, &json)?);
    }
    Ok(all)
}

fn extract_tracks(
    ctx: &ToolContext,
    ffmpeg: &Path,
    path: &Path,
    json: &Value,
) -> Result<Vec<PathBuf>> {
    let subs: Vec<&Value> = streams(json)
        .filter(|s| s["codec_type"] == "subtitle")
        .collect();
    if subs.is_empty() {
        bail!(
            "{} ne contient aucune piste de sous-titres",
            output::file_name(path)
        );
    }
    let mut pending: Vec<OutputFile> = Vec::new();
    for (k, s) in subs.iter().enumerate() {
        let codec = s["codec_name"].as_str().unwrap_or("?");
        let Some((enc, ext)) = subtitle_target(codec) else {
            ctx.reporter.warn(&format!(
                "piste {} : sous-titres image ({codec}), non convertibles en texte sans OCR — ignorée",
                k + 1
            ));
            continue;
        };
        let lang = s["tags"]["language"]
            .as_str()
            .filter(|l| l.chars().all(|c| c.is_ascii_alphanumeric()));
        let suffix = match lang {
            Some(l) => format!("st{}-{l}", k + 1),
            None => format!("st{}", k + 1),
        };
        let out = output::beside(path, Some(&suffix), ext)?;
        let mut args = base_args();
        args.extend(["-i".into(), path.as_os_str().to_owned()]);
        args.extend([
            "-map".into(),
            format!("0:s:{k}").into(),
            "-c:s".into(),
            enc.into(),
        ]);
        args.push(out.path().into());
        run_progress(ctx, ffmpeg, &output::file_name(path), &args, (None, None))?;
        pending.push(out);
    }
    if pending.is_empty() {
        bail!(
            "aucune piste de sous-titres texte dans {}",
            output::file_name(path)
        );
    }
    Ok(pending.into_iter().map(OutputFile::commit).collect())
}

/// Caractéristiques d'une vidéo pour la concaténation.
struct Clip {
    width: u64,
    height: u64,
    audio: bool,
    duration: f64,
}

fn clip_info(ctx: &ToolContext, ffmpeg: &Path, path: &Path) -> Result<Clip> {
    let json = probe(ctx, ffmpeg, path)?.context("ffprobe (fourni avec ffmpeg) est nécessaire")?;
    let video = streams(&json)
        .find(|s| s["codec_type"] == "video")
        .with_context(|| format!("{} : aucune piste vidéo", output::file_name(path)))?;
    Ok(Clip {
        width: video["width"].as_u64().unwrap_or(1280),
        height: video["height"].as_u64().unwrap_or(720),
        audio: has_stream(&json, "audio"),
        duration: probe_duration(&json).unwrap_or(0.0),
    })
}

/// Graphe `concat` : chaque clip mis à la taille du premier (bandes noires),
/// 30 im/s, son stéréo 48 kHz (silence si le clip n'en a pas).
pub fn concat_filter(clips: &[(bool, f64)], (w, h): (u64, u64)) -> String {
    let mut graph = String::new();
    let mut pads = String::new();
    for (i, (audio, dur)) in clips.iter().enumerate() {
        graph.push_str(&format!(
            "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30,format=yuv420p[v{i}];"
        ));
        if *audio {
            graph.push_str(&format!(
                "[{i}:a]aresample=48000,aformat=channel_layouts=stereo[a{i}];"
            ));
        } else {
            graph.push_str(&format!(
                "anullsrc=r=48000:cl=stereo,atrim=duration={dur:.3}[a{i}];"
            ));
        }
        pads.push_str(&format!("[v{i}][a{i}]"));
    }
    format!("{graph}{pads}concat=n={}:v=1:a=1[v][a]", clips.len())
}

/// `video-concatener` : à la suite, dans l'ordre (réencodage H.264/AAC).
pub fn concat(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    let inputs = &ctx.input.inputs;
    let clips = inputs
        .iter()
        .map(|p| clip_info(ctx, &ffmpeg, p))
        .collect::<Result<Vec<_>>>()?;
    let size = (
        clips[0].width - clips[0].width % 2,
        clips[0].height - clips[0].height % 2,
    );
    let shape: Vec<(bool, f64)> = clips.iter().map(|c| (c.audio, c.duration)).collect();
    let out = output::beside(&inputs[0], Some("assemble"), "mp4")?;
    let mut args = base_args();
    for p in inputs {
        args.extend(["-i".into(), p.as_os_str().to_owned()]);
    }
    args.extend(["-filter_complex".into(), concat_filter(&shape, size).into()]);
    args.extend(
        strings(&["-map", "[v]", "-map", "[a]"])
            .into_iter()
            .map(OsString::from),
    );
    args.extend(video_codecs("mp4")?.into_iter().map(OsString::from));
    args.push(out.path().into());
    let total: f64 = clips.iter().map(|c| c.duration).sum();
    run_progress(
        ctx,
        &ffmpeg,
        &output::file_name(&inputs[0]),
        &args,
        (Some(total), None),
    )?;
    ctx.reporter.log(&format!(
        "{} vidéo(s) assemblée(s), {total:.1} s",
        inputs.len()
    ));
    Ok(vec![out.commit()])
}

/// Grille « 4x3 » → (4, 3).
pub fn parse_grid(value: &str) -> Result<(u32, u32)> {
    let (c, r) = value.split_once('x').context("grille invalide (ex. 4x3)")?;
    let (c, r): (u32, u32) = (c.trim().parse()?, r.trim().parse()?);
    if !(1..=10).contains(&c) || !(1..=10).contains(&r) {
        bail!("grille invalide : 1 à 10 colonnes et lignes");
    }
    Ok((c, r))
}

/// `video-vignettes` : planche d'images réparties sur toute la durée.
pub fn thumbnail_sheet(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let (cols, rows) = parse_grid(ctx.input.option("grille"))?;
    let ffmpeg = ctx.binary(Binary::Ffmpeg)?;
    per_file(ctx, |path| {
        let json =
            probe(ctx, &ffmpeg, path)?.context("ffprobe (fourni avec ffmpeg) est nécessaire")?;
        let dur = probe_duration(&json)
            .filter(|d| *d > 0.0)
            .context("durée de la vidéo inconnue")?;
        let n = cols * rows;
        let out = output::beside(path, Some("vignettes"), "jpg")?;
        let vf = format!("fps={n}/{dur:.3},scale=320:-2,tile={cols}x{rows}:padding=4:margin=4");
        let mut args = base_args();
        args.extend([
            "-i".into(),
            path.as_os_str().to_owned(),
            "-vf".into(),
            vf.into(),
        ]);
        args.extend(
            strings(&["-frames:v", "1", "-update", "1", "-q:v", "3"])
                .into_iter()
                .map(OsString::from),
        );
        args.push(out.path().into());
        run_progress(
            ctx,
            &ffmpeg,
            &output::file_name(path),
            &args,
            (Some(dur), None),
        )?;
        Ok(out.commit())
    })
}

#[cfg(test)]
fn write_text(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::{self, Resolver};
    use crate::tools::testutil::real;

    #[test]
    fn filtres() {
        assert_eq!(atempo_chain(4.0), "atempo=2,atempo=2.0000");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5000");
        assert!(rotate_filter("45").is_err());
        assert_eq!(parse_grid("4x3").unwrap(), (4, 3));
        assert!(parse_grid("0x3").is_err() && parse_grid("abc").is_err());
        let f = concat_filter(&[(true, 2.0), (false, 1.5)], (64, 48));
        assert!(
            f.ends_with("[v0][a0][v1][a1]concat=n=2:v=1:a=1[v][a]"),
            "{f}"
        );
        assert!(f.contains("atrim=duration=1.500[a1]"));
        assert_eq!(subtitle_target("mov_text"), Some(("srt", "srt")));
        assert_eq!(subtitle_target("hdmv_pgs_subtitle"), None);
    }

    fn ff(r: &Resolver, args: &[&str], out: &Path) {
        let ffmpeg = r.resolve(Binary::Ffmpeg).unwrap().path;
        let mut all: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-y"]
            .iter()
            .map(OsString::from)
            .collect();
        all.extend(args.iter().map(OsString::from));
        all.push(out.into());
        sidecar::run(&ffmpeg, &all, std::time::Duration::from_secs(60))
            .unwrap()
            .check("ffmpeg")
            .unwrap();
    }

    fn clip(r: &Resolver, dir: &Path, name: &str, audio: bool) -> PathBuf {
        let out = dir.join(name);
        let mut args = vec!["-f", "lavfi", "-i", "testsrc=size=64x48:rate=10:duration=2"];
        if audio {
            args.extend([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:a",
                "aac",
            ]);
        }
        args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-shortest"]);
        ff(r, &args, &out);
        out
    }

    fn run(id: &str, job: JobInput, r: &Resolver) -> Vec<PathBuf> {
        execute(id, job, &MemoryReporter::default(), r).unwrap_or_else(|e| panic!("{id} : {e:#}"))
    }

    fn dims(path: &Path) -> (u32, u32) {
        let img = image::open(path).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn outils_video_avec_ffmpeg() {
        let Some(r) = real(&[Binary::Ffmpeg]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = clip(&r, tmp.path(), "clip.mp4", true);
        let job = |k: &str, v: &str| JobInput::new(vec![src.clone()]).with(k, v);

        let gif = run("video-vers-gif", job("fin", "1").with("largeur", "32"), &r);
        assert!(gif[0].ends_with("clip.gif"));
        assert_eq!(dims(&gif[0]).0, 32);

        let small = run("video-redimensionner", job("hauteur", "240"), &r);
        assert!(small[0].ends_with("clip-240p.mp4"), "{small:?}");
        let rot = run("video-pivoter", job("angle", "90"), &r);
        let frame = run(
            "video-extraire-image",
            JobInput::new(rot).with("instant", "0.5"),
            &r,
        );
        assert_eq!(dims(&frame[0]), (48, 64), "pivoté");

        let fast = run("video-accelerer", job("facteur", "2"), &r);
        assert!(fast[0].ends_with("clip-x2.mp4"), "{fast:?}");
        let info = run("video-infos", JobInput::new(fast), &r);
        let text = std::fs::read_to_string(&info[0]).unwrap();
        assert!(text.contains("(1."), "durée ÷ 2 attendue : {text}");

        let sheet = run("video-vignettes", job("grille", "2x2"), &r);
        assert!(sheet[0].ends_with("clip-vignettes.jpg"));
        assert_eq!(dims(&sheet[0]).0, 2 * 320 + 3 * 4);
    }

    #[test]
    fn sous_titres_concat_normaliser_avec_ffmpeg() {
        let Some(r) = real(&[Binary::Ffmpeg]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let a = clip(&r, tmp.path(), "a.mp4", true);
        let b = clip(&r, tmp.path(), "b.mp4", false);
        let srt = tmp.path().join("texte l'été.srt");
        write_text(&srt, "1\n00:00:00,000 --> 00:00:01,500\nBonjour\n\n");

        let burned = run(
            "video-incruster-sous-titres",
            JobInput::new(vec![a.clone(), srt.clone()]),
            &r,
        );
        assert!(burned[0].ends_with("a-soustitre.mp4"));

        let mkv = tmp.path().join("avec-st.mkv");
        let (a_s, srt_s) = (
            a.to_string_lossy().to_string(),
            srt.to_string_lossy().to_string(),
        );
        ff(
            &r,
            &[
                "-i",
                &a_s,
                "-i",
                &srt_s,
                "-map",
                "0",
                "-map",
                "1",
                "-c",
                "copy",
                "-c:s",
                "srt",
                "-metadata:s:s:0",
                "language=fra",
            ],
            &mkv,
        );
        let subs = run("video-extraire-sous-titres", JobInput::new(vec![mkv]), &r);
        assert!(subs[0].ends_with("avec-st-st1-fra.srt"), "{subs:?}");
        assert!(
            std::fs::read_to_string(&subs[0])
                .unwrap()
                .contains("Bonjour")
        );
        let none = execute(
            "video-extraire-sous-titres",
            JobInput::new(vec![a.clone()]),
            &MemoryReporter::default(),
            &r,
        );
        assert!(none.unwrap_err().to_string().contains("aucune piste"));

        let joined = run("video-concatener", JobInput::new(vec![a.clone(), b]), &r);
        assert!(joined[0].ends_with("a-assemble.mp4"));
        let info = run("video-infos", JobInput::new(joined), &r);
        let text = std::fs::read_to_string(&info[0]).unwrap();
        assert!(
            text.contains("(4.0") && text.contains("audio aac"),
            "{text}"
        );

        let mp3 = run(
            "video-extraire-audio",
            JobInput::new(vec![a]).with("format", "mp3"),
            &r,
        );
        let norm = run("audio-normaliser", JobInput::new(mp3), &r);
        assert!(norm[0].ends_with("a-normalise.mp3"));
    }
}
