//! Archives avec 7-Zip (inclus dans l'installeur) : créer / extraire / tester
//! un 7z (et RAR, ISO, CAB… en lecture), ajouter à une archive, lister.
//!
//! - Mot de passe : **jamais** en argument (visible dans la liste des
//!   processus) ; écrit sur l'entrée standard de 7-Zip, qui le lit là.
//! - `--` avant les chemins : aucun nom ne peut être pris pour une option ;
//!   chemins absolus (jamais de `@liste`) ; noms contenant `*` ou `?` refusés
//!   (7-Zip les interpréterait comme des jokers).
//! - Création / ajout : 7-Zip écrit dans un dossier de travail du même
//!   dossier, puis le résultat remplit une sortie réservée (`create_new`).
//! - Extraction : dans un dossier neuf réservé ; 7-Zip refuse déjà les liens
//!   « dangereux » (sans `-snld`) ; tout lien symbolique restant est supprimé.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};

use super::archives_extract::{Item, archive_base, render};
use crate::jobs::ToolContext;
use crate::output::{self, OutputFile};
use crate::sidecar::{self, Binary, RunOutput, Stream};

/// Code de sortie 7-Zip « avertissement » (ex. fichier verrouillé ignoré).
const SEVEN_WARNING: i32 = 1;

fn password(ctx: &ToolContext) -> Result<Option<String>> {
    let pw = ctx.input.option("mot_de_passe");
    if pw.contains(['\n', '\r']) {
        bail!("le mot de passe ne peut pas contenir de retour à la ligne");
    }
    Ok((!pw.is_empty()).then(|| pw.to_string()))
}

fn checked_path(path: &Path) -> Result<OsString> {
    let abs = std::path::absolute(path)?;
    if abs.to_string_lossy().contains(['*', '?']) {
        bail!(
            "nom non pris en charge par 7-Zip (caractère * ou ?) : {}",
            path.display()
        );
    }
    Ok(abs.into_os_string())
}

/// « 45% 3 + fichier » → 45.
pub fn parse_progress(line: &str) -> Option<u32> {
    line.trim().split('%').next()?.trim().parse().ok()
}

/// Lance 7-Zip ; `stdin` = mot de passe (+ confirmation pour la création).
fn run(ctx: &ToolContext, args: &[OsString], stdin: &[u8], what: &str) -> Result<RunOutput> {
    let exe = ctx.binary(Binary::SevenZip)?;
    let last = AtomicU32::new(u32::MAX);
    let reporter = ctx.reporter;
    let on_line = |s: Stream, line: &str| {
        if let (Stream::Stdout, Some(p)) = (s, parse_progress(line))
            && last.swap(p, Ordering::Relaxed) != p
        {
            reporter.progress(p.min(99) as f32, what);
        }
    };
    let opts = sidecar::RunOptions {
        stdin: Some(stdin),
        ..ctx.run_options(Some(&on_line))
    };
    let run = sidecar::run_with(&exe, args, opts)?;
    check(ctx, run)
}

fn check(ctx: &ToolContext, run: RunOutput) -> Result<RunOutput> {
    let all = format!("{}\n{}", run.stdout, run.stderr);
    match run.status.code() {
        Some(0) => Ok(run),
        Some(SEVEN_WARNING) => {
            ctx.reporter.warn(&format!(
                "7-Zip : {}",
                sidecar::truncate_stderr(&run.stderr)
            ));
            Ok(run)
        }
        _ if all.contains("Wrong password") => bail!("mot de passe incorrect"),
        _ if all.contains("Can't open as archive")
            || all.contains("Cannot open the file as archive") =>
        {
            bail!("archive illisible, endommagée, ou protégée par un mot de passe incorrect")
        }
        _ => bail!(
            "7-Zip a échoué ({}) : {}",
            run.status,
            sidecar::truncate_stderr(&run.stderr)
        ),
    }
}

/// Niveau `-mx` d'une option.
pub fn level(value: &str) -> Result<&'static str> {
    Ok(match value {
        "rapide" => "-mx=1",
        "normale" => "-mx=5",
        "maximale" => "-mx=9",
        other => bail!("niveau de compression inconnu : {other}"),
    })
}

/// Arguments `a` (création / ajout) vers `archive`.
pub fn add_args(
    archive: &Path,
    inputs: &[OsString],
    mx: &str,
    pw: bool,
    kind: &str,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["a", kind, mx, "-y", "-bsp1", "-bb0", "-ssw"]
        .iter()
        .map(OsString::from)
        .collect();
    if pw {
        args.extend(["-p", "-mhe=on"].map(OsString::from));
    }
    args.push("--".into());
    args.push(archive.into());
    args.extend(inputs.iter().cloned());
    args
}

fn pw_input(pw: &Option<String>, confirm: bool) -> Vec<u8> {
    match pw {
        Some(p) if confirm => format!("{p}\n{p}\n").into_bytes(),
        Some(p) => format!("{p}\n").into_bytes(),
        None => b"\n".to_vec(),
    }
}

fn reserve_for(inputs: &[PathBuf], ext: &str) -> Result<OutputFile> {
    match inputs {
        [single] => output::reserve(&output::parent_dir(single), &archive_base(single), ext),
        [first, ..] => output::reserve(&output::parent_dir(first), "archive", ext),
        [] => bail!("aucun fichier à archiver"),
    }
}

/// `archives-creer-7z`.
pub fn create_7z(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let mx = level(ctx.input.option("niveau"))?;
    let pw = password(ctx)?;
    let sources = inputs
        .iter()
        .map(|p| checked_path(p))
        .collect::<Result<Vec<_>>>()?;
    let out = reserve_for(inputs, "7z")?;
    let work = tempfile::Builder::new()
        .prefix(".boite-a-outils-")
        .tempdir_in(output::parent_dir(out.path()))?;
    let tmp_archive = work.path().join("archive.7z");
    let args = add_args(&tmp_archive, &sources, mx, pw.is_some(), "-t7z");
    run(ctx, &args, &pw_input(&pw, true), "Compression…")?;
    if pw.is_some() {
        ctx.reporter
            .log("Archive chiffrée (AES-256, noms de fichiers compris).");
    }
    Ok(vec![output::fill_from(out, &tmp_archive)?])
}

/// Formats que 7-Zip sait mettre à jour.
const UPDATABLE: [&str; 3] = ["7z", "zip", "tar"];

fn ext_of(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `archives-ajouter` : copie de l'archive (la première de la liste) complétée
/// des autres fichiers → `<nom>-complete.<ext>` ; l'original n'est pas modifié.
pub fn add_to_archive(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let inputs = &ctx.input.inputs;
    let archive = inputs
        .iter()
        .find(|p| p.is_file() && UPDATABLE.contains(&ext_of(p).as_str()))
        .context("ajoutez une archive .7z, .zip ou .tar à la liste (elle ne sera pas modifiée)")?;
    let others: Vec<&PathBuf> = inputs.iter().filter(|p| *p != archive).collect();
    if others.is_empty() {
        bail!("ajoutez aussi les fichiers ou dossiers à mettre dans l'archive");
    }
    let sources = others
        .iter()
        .map(|p| checked_path(p))
        .collect::<Result<Vec<_>>>()?;
    let pw = password(ctx)?;
    let ext = ext_of(archive);
    let out = output::beside(archive, Some("complete"), &ext)?;
    let work = tempfile::Builder::new()
        .prefix(".boite-a-outils-")
        .tempdir_in(output::parent_dir(archive))?;
    let copy = work.path().join(format!("archive.{ext}"));
    std::fs::copy(archive, &copy).context("copie de l'archive impossible")?;
    let args = add_args(&copy, &sources, "-mx=5", pw.is_some(), &format!("-t{ext}"));
    run(ctx, &args, &pw_input(&pw, true), "Ajout…")?;
    ctx.reporter
        .log(&format!("{} élément(s) ajouté(s)", others.len()));
    Ok(vec![output::fill_from(out, &copy)?])
}

/// Supprime les liens symboliques laissés par l'extraction (sécurité).
fn remove_links(ctx: &ToolContext, dir: &Path) -> Result<usize> {
    let mut removed = 0;
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry?;
        if entry.path_is_symlink() {
            std::fs::remove_file(entry.path())?;
            ctx.reporter.warn(&format!(
                "lien symbolique ignoré : {}",
                entry
                    .path()
                    .strip_prefix(dir)
                    .unwrap_or(entry.path())
                    .display()
            ));
            removed += 1;
        }
    }
    Ok(removed)
}

/// `archives-extraire-7z` : chaque archive → dossier neuf `<nom>/`.
pub fn extract_7z(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let pw = password(ctx)?;
    let mut outs = Vec::new();
    for path in &ctx.input.inputs {
        ctx.check_cancel()?;
        let dir = output::reserve_dir(&output::parent_dir(path), &archive_base(path))?;
        let mut out_arg = OsString::from("-o");
        out_arg.push(std::path::absolute(dir.path())?);
        let args = vec![
            "x".into(),
            "-y".into(),
            "-bsp1".into(),
            "-bb0".into(),
            out_arg,
            "--".into(),
            checked_path(path)?,
        ];
        run(ctx, &args, &pw_input(&pw, false), "Extraction…")?;
        remove_links(ctx, dir.path())?;
        ctx.reporter
            .log(&format!("{} extrait", output::file_name(path)));
        outs.push(dir.commit());
    }
    Ok(outs)
}

/// `archives-tester` : intégrité vérifiée par 7-Zip, rapport dans le journal.
pub fn test_archive(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    let pw = password(ctx)?;
    let mut failures = 0;
    for path in &ctx.input.inputs {
        ctx.check_cancel()?;
        let args = vec![
            "t".into(),
            "-bsp1".into(),
            "-bb0".into(),
            "--".into(),
            checked_path(path)?,
        ];
        let name = output::file_name(path);
        match run(ctx, &args, &pw_input(&pw, false), "Vérification…") {
            Ok(_) => ctx.reporter.log(&format!("✔ {name} : archive intègre")),
            Err(e) => {
                failures += 1;
                ctx.reporter.warn(&format!("✘ {name} : {e:#}"));
            }
        }
    }
    if failures > 0 {
        bail!("{failures} archive(s) endommagée(s) ou illisible(s) (voir le journal)");
    }
    Ok(Vec::new())
}

/// Listage technique `7z l -slt` → éléments (blocs « Path = … »).
pub(crate) fn parse_slt(stdout: &str) -> Vec<Item> {
    let body = stdout.split("----------").nth(1).unwrap_or(stdout);
    body.split("\n\n")
        .filter_map(|block| {
            let get = |k: &str| {
                block
                    .lines()
                    .find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()))
            };
            let name = get("Path = ")?;
            Some(Item {
                name,
                size: get("Size = ").and_then(|s| s.parse().ok()).unwrap_or(0),
                is_dir: get("Folder = ").is_some_and(|f| f == "+")
                    || get("Attributes = ").is_some_and(|a| a.starts_with('D')),
            })
        })
        .collect()
}

/// `archives-lister` pour les formats non intégrés (7z, RAR, ISO…).
pub fn list_with_7z(ctx: &ToolContext, path: &Path) -> Result<String> {
    let pw = password(ctx)?;
    let args = vec!["l".into(), "-slt".into(), "--".into(), checked_path(path)?];
    let run = run(ctx, &args, &pw_input(&pw, false), "Lecture…")?;
    render(&output::file_name(path), &parse_slt(&run.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use crate::tools::testutil::real;
    use std::fs;

    #[test]
    fn arguments_et_analyse() {
        let a = add_args(
            Path::new("/t/x.7z"),
            &["/d/a".into()],
            "-mx=9",
            true,
            "-t7z",
        );
        let pos = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(a[pos + 1], "/t/x.7z");
        assert!(
            !a.iter()
                .any(|x| x.to_string_lossy().starts_with("-psecret"))
        );
        assert_eq!(parse_progress(" 45% 3 + a.txt"), Some(45));
        let slt = "--\nPath = x.7z\nType = 7z\n\n----------\nPath = d\nFolder = +\nSize = 0\n\nPath = d/a.txt\nFolder = -\nSize = 12\n\n";
        let items = parse_slt(slt);
        assert_eq!(items.len(), 2);
        assert!(items[0].is_dir && items[1].size == 12);
        assert!(checked_path(Path::new("a*b")).is_err());
    }

    fn run_tool(id: &str, job: JobInput, r: &Resolver) -> Result<Vec<PathBuf>> {
        execute(id, job, &MemoryReporter::default(), r)
    }

    fn tree(dir: &Path) -> PathBuf {
        let src = dir.join("photos");
        fs::create_dir_all(src.join("sous")).unwrap();
        fs::write(src.join("a.txt"), b"contenu A").unwrap();
        fs::write(src.join("sous/b.txt"), "contenu B ".repeat(50)).unwrap();
        src
    }

    #[test]
    fn creer_extraire_tester_avec_7zip() {
        let Some(r) = real(&[Binary::SevenZip]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = tree(tmp.path());
        fs::write(tmp.path().join("photos.7z"), b"existant").unwrap();
        let job = JobInput::new(vec![src.clone()]).with("mot_de_passe", "s3cret");
        let outs = run_tool("archives-creer-7z", job, &r).unwrap();
        assert_eq!(outs, vec![tmp.path().join("photos-2.7z")]);
        assert_eq!(fs::read(tmp.path().join("photos.7z")).unwrap(), b"existant");
        let leftovers = fs::read_dir(tmp.path()).unwrap().filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".boite")
        });
        assert_eq!(leftovers.count(), 0, "dossier de travail supprimé");

        let wrong = JobInput::new(outs.clone()).with("mot_de_passe", "faux");
        let err = run_tool("archives-extraire-7z", wrong, &r).unwrap_err();
        assert!(format!("{err:#}").contains("mot de passe"), "{err:#}");
        assert!(!tmp.path().join("photos-3").exists() && !tmp.path().join("photos-2").exists());

        let good = || JobInput::new(outs.clone()).with("mot_de_passe", "s3cret");
        let dirs = run_tool("archives-extraire-7z", good(), &r).unwrap();
        assert_eq!(dirs, vec![tmp.path().join("photos-2")]);
        let b = fs::read_to_string(dirs[0].join("photos/sous/b.txt")).unwrap();
        assert_eq!(b, "contenu B ".repeat(50));
        assert!(run_tool("archives-tester", good(), &r).unwrap().is_empty());

        let list = run_tool("archives-lister", good(), &r).unwrap();
        let text = fs::read_to_string(&list[0]).unwrap();
        assert!(
            text.contains("photos/sous/b.txt") && text.contains("2 fichier(s)"),
            "{text}"
        );
    }

    #[test]
    fn ajouter_et_archive_abimee_avec_7zip() {
        let Some(r) = real(&[Binary::SevenZip]) else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let src = tree(tmp.path());
        let zip = run_tool("archives-creer-zip", JobInput::new(vec![src]), &r).unwrap();
        let before = fs::read(&zip[0]).unwrap();
        let extra = tmp.path().join("note.txt");
        fs::write(&extra, b"ajout").unwrap();
        let outs = run_tool(
            "archives-ajouter",
            JobInput::new(vec![zip[0].clone(), extra]),
            &r,
        )
        .unwrap();
        assert_eq!(outs, vec![tmp.path().join("photos-complete.zip")]);
        assert_eq!(fs::read(&zip[0]).unwrap(), before, "original intact");
        let mut z = zip::ZipArchive::new(fs::File::open(&outs[0]).unwrap()).unwrap();
        assert!(z.by_name("note.txt").is_ok() && z.by_name("photos/a.txt").is_ok());

        let bad = tmp.path().join("abime.7z");
        fs::write(&bad, b"7z\xbc\xaf\x27\x1c pas vraiment").unwrap();
        let err = run_tool("archives-tester", JobInput::new(vec![bad]), &r).unwrap_err();
        assert!(err.to_string().contains("endommagée"), "{err}");
    }
}
