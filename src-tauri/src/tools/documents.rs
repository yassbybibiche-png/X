//! Conversions de documents avec pandoc, **toujours** en `--sandbox` (PLAN.md §3.4).
//!
//! En sandbox, pandoc ne lit que les fichiers passés en ligne de commande : un
//! document piégé ne peut pas inclure un fichier privé (ex. clé SSH) dans la
//! sortie. Avant l'appel, les images des entrées Markdown et HTML sont
//! pré-traitées dans une copie temporaire :
//! - distantes (http, https, `//`) → retirées, **signalées** ;
//! - locales dans le dossier du document → incorporées en `data:` URI ;
//! - locales hors du dossier (absolues, `..`, lien symbolique) → retirées, signalées.

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use regex::{Captures, Regex};

use super::per_file;
use crate::jobs::ToolContext;
use crate::output;
use crate::sidecar::{Binary, Stream};

/// Taille maximale d'une image incorporée.
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

/// Construit les arguments pandoc. **Seule** façon de lancer pandoc dans
/// l'application : `--sandbox` est toujours le premier argument.
pub fn pandoc_args(
    from: &str,
    to: &str,
    input: &Path,
    output: &Path,
    extra: &[&str],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["--sandbox", "-f", from, "-t", to]
        .iter()
        .map(OsString::from)
        .collect();
    args.extend(extra.iter().map(OsString::from));
    args.push("-o".into());
    args.push(output.into());
    args.push(input.into());
    args
}

/// Conversion déclarée dans le registre.
struct Conversion {
    to: &'static str,
    ext: &'static str,
    extra: &'static [&'static str],
}

fn conversion(tool_id: &str) -> Result<Conversion> {
    let c = |to, ext, extra| Conversion { to, ext, extra };
    Ok(match tool_id {
        "documents-md-vers-docx" | "documents-odt-vers-docx" | "documents-html-vers-docx" => {
            c("docx", "docx", &[])
        }
        "documents-md-vers-odt" | "documents-docx-vers-odt" => c("odt", "odt", &[]),
        "documents-md-vers-html" | "documents-docx-vers-html" => {
            c("html5", "html", &["--standalone"])
        }
        "documents-md-vers-epub" => c("epub3", "epub", &[]),
        "documents-docx-vers-md"
        | "documents-odt-vers-md"
        | "documents-html-vers-md"
        | "documents-epub-vers-md" => c("gfm", "md", &["--wrap=none"]),
        "documents-extraire-texte" => c("plain", "txt", &["--wrap=none"]),
        // Chaîne pandoc → typst : voir `to_pdf`.
        "documents-md-vers-pdf" | "documents-docx-vers-pdf" => c("pdf", "pdf", &[]),
        other => bail!("conversion inconnue : {other}"),
    })
}

/// Format d'entrée pandoc d'après l'extension.
pub fn input_format(path: &Path) -> Result<&'static str> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Ok(match ext.as_str() {
        "md" | "markdown" | "txt" => "markdown",
        "docx" => "docx",
        "odt" => "odt",
        "html" | "htm" => "html",
        "epub" => "epub",
        "rtf" => "rtf",
        other => bail!("format d'entrée non pris en charge : {other}"),
    })
}

/// Point d'entrée commun à tous les outils `documents-*`.
pub fn convert(ctx: &ToolContext, tool_id: &str) -> Result<Vec<PathBuf>> {
    let conv = conversion(tool_id)?;
    let pandoc = ctx.binary(Binary::Pandoc)?;
    per_file(ctx, |path| convert_one(ctx, &pandoc, &conv, path))
}

macro_rules! tools {
    ($($fn_name:ident => $id:literal),* $(,)?) => {
        $(pub fn $fn_name(ctx: &ToolContext) -> Result<Vec<PathBuf>> { convert(ctx, $id) })*
    };
}

tools! {
    md_to_pdf => "documents-md-vers-pdf",
    docx_to_pdf => "documents-docx-vers-pdf",
    md_to_docx => "documents-md-vers-docx",
    md_to_odt => "documents-md-vers-odt",
    md_to_html => "documents-md-vers-html",
    md_to_epub => "documents-md-vers-epub",
    docx_to_md => "documents-docx-vers-md",
    docx_to_odt => "documents-docx-vers-odt",
    docx_to_html => "documents-docx-vers-html",
    odt_to_docx => "documents-odt-vers-docx",
    odt_to_md => "documents-odt-vers-md",
    html_to_md => "documents-html-vers-md",
    html_to_docx => "documents-html-vers-docx",
    epub_to_md => "documents-epub-vers-md",
    extract_text => "documents-extraire-texte",
}

fn convert_one(
    ctx: &ToolContext,
    pandoc: &Path,
    conv: &Conversion,
    path: &Path,
) -> Result<PathBuf> {
    ctx.check_cancel()?;
    let out = output::beside(path, None, conv.ext)?;
    let from = input_format(path)?;
    let tmp = tempfile::tempdir().context("dossier temporaire impossible")?;
    let source = prepare_input(ctx, path, from, tmp.path())?;
    if conv.to == "pdf" {
        return to_pdf(ctx, pandoc, (path, &source, from), tmp.path(), out);
    }
    let mut extra: Vec<String> = conv.extra.iter().map(|s| s.to_string()).collect();
    if conv.ext == "html" {
        extra.push(format!("--metadata=pagetitle:{}", output::stem(path)));
    }
    let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
    let args = pandoc_args(from, conv.to, &source, out.path(), &extra);
    run_pandoc(ctx, pandoc, &args, None)?;
    if from == "docx" && conv.ext == "md" {
        ctx.reporter
            .log("Les images du document Word ne sont pas extraites (mode isolé).");
    }
    Ok(out.commit())
}

/// Lance pandoc (avertissements relayés) ; erreur claire pour un pandoc de
/// distribution sans fichiers de données (incompatible avec `--sandbox`).
fn run_pandoc(
    ctx: &ToolContext,
    pandoc: &Path,
    args: &[OsString],
    dir: Option<&Path>,
) -> Result<()> {
    let reporter = ctx.reporter;
    let on_line = |stream: Stream, line: &str| {
        if stream == Stream::Stderr && line.contains("WARNING") {
            reporter.warn(&format!("pandoc : {}", line.trim()));
        }
    };
    let opts = crate::sidecar::RunOptions {
        current_dir: dir,
        ..ctx.run_options(Some(&on_line))
    };
    let run = crate::sidecar::run_with(pandoc, args, opts)?;
    if !run.status.success() && run.stderr.contains("Could not find data file") {
        bail!(
            "ce pandoc ({}) n'embarque pas ses fichiers de données (paquet de distribution) : \
             il ne peut pas produire ce format en mode isolé (--sandbox, toujours actif). \
             Installez le pandoc officiel depuis l'onglet Diagnostic.",
            pandoc.display()
        );
    }
    run.check("pandoc")?;
    Ok(())
}

/// PDF en deux étapes, dans le dossier de travail `dir` :
/// 1. pandoc `--sandbox` → `document.typ` (images Markdown/HTML écrites dans
///    `dir`, images Word extraites dans `dir/media`) ;
/// 2. `typst compile --root <dir>` : typst ne peut lire que ce dossier.
///
/// En Markdown, les blocs bruts (`{=typst}`) sont désactivés : un document ne
/// peut pas injecter de code typst. Police embarquée dans typst (les gabarits
/// pandoc ≤ 3.1 passent sinon une liste de polices vide, refusée par typst).
fn to_pdf(
    ctx: &ToolContext,
    pandoc: &Path,
    (path, source, from): (&Path, &Path, &'static str),
    dir: &Path,
    out: output::OutputFile,
) -> Result<PathBuf> {
    let typst = ctx.binary(Binary::Typst)?;
    if source != path {
        materialize_data_uris(source, dir)?;
    }
    let reader = if from == "markdown" {
        "markdown-raw_attribute"
    } else {
        from
    };
    let typ = dir.join("document.typ");
    let mut extra = vec!["-s", "-V", "mainfont=Libertinus Serif"];
    if from != "markdown" && from != "html" {
        extra.push("--extract-media=media");
    }
    ctx.reporter.progress(10.0, "Mise en page (pandoc)…");
    run_pandoc(
        ctx,
        pandoc,
        &pandoc_args(reader, "typst", source, &typ, &extra),
        Some(dir),
    )?;
    ctx.reporter.progress(50.0, "Création du PDF (typst)…");
    let pdf = dir.join("document.pdf");
    let args: Vec<OsString> = vec![
        "compile".into(),
        "--root".into(),
        dir.into(),
        typ.into(),
        pdf.clone().into(),
    ];
    let opts = crate::sidecar::RunOptions {
        current_dir: Some(dir),
        ..ctx.run_options(None)
    };
    crate::sidecar::run_with(&typst, &args, opts)?.check("typst")?;
    output::fill_from(out, &pdf)
}

/// Copie pré-traitée (Markdown/HTML) dans `tmp`, ou le fichier d'origine.
fn prepare_input(ctx: &ToolContext, path: &Path, from: &str, tmp: &Path) -> Result<PathBuf> {
    if from != "markdown" && from != "html" {
        return Ok(path.to_path_buf());
    }
    let text = std::fs::read_to_string(path).context("le document n'est pas un texte UTF-8")?;
    let base = output::parent_dir(path);
    let report = rewrite_images(&text, &base, from == "html");
    for url in &report.remote {
        ctx.reporter
            .warn(&format!("Image distante retirée (mode isolé) : {url}"));
    }
    for url in &report.refused {
        ctx.reporter.warn(&format!(
            "Image hors du dossier du document retirée : {url}"
        ));
    }
    let copy = tmp.join(output::file_name(path));
    std::fs::write(&copy, report.text.as_bytes())?;
    Ok(copy)
}

static DATA_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"data:image/(?P<sub>png|jpeg|gif|webp|bmp|svg\+xml);base64,(?P<b64>[A-Za-z0-9+/=]+)",
    )
    .unwrap()
});

/// Pour typst (qui ne lit pas les URI `data:`) : chaque image incorporée est
/// écrite dans `dir` sous un nom fixe (`image-<n>.<ext>`) et référencée par ce nom.
fn materialize_data_uris(source: &Path, dir: &Path) -> Result<()> {
    let text = std::fs::read_to_string(source)?;
    let mut n = 0;
    let mut failure = None;
    let out = DATA_URI.replace_all(&text, |c: &Captures| {
        n += 1;
        let ext = match &c["sub"] {
            "jpeg" => "jpg",
            "svg+xml" => "svg",
            other => other,
        };
        let name = format!("image-{n}.{ext}");
        let bytes = base64::engine::general_purpose::STANDARD.decode(&c["b64"]);
        match bytes
            .map_err(anyhow::Error::from)
            .and_then(|b| Ok(std::fs::write(dir.join(&name), b)?))
        {
            Ok(()) => name,
            Err(e) => {
                failure = Some(e);
                String::new()
            }
        }
    });
    if let Some(e) = failure {
        return Err(e.context("préparation des images impossible"));
    }
    std::fs::write(source, out.as_bytes())?;
    Ok(())
}

/// Résultat du pré-traitement des images.
#[derive(Debug, Default)]
pub struct ImageReport {
    pub text: String,
    pub remote: Vec<String>,
    pub refused: Vec<String>,
    pub embedded: usize,
}

static MD_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"!\[(?P<alt>[^\]]*)\]\(\s*<?(?P<url>[^)\s>]+)>?(?:\s+"[^"]*")?\s*\)"#).unwrap()
});
static HTML_IMG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<img\b[^>]*?\bsrc\s*=\s*(?:"(?P<dq>[^"]*)"|'(?P<sq>[^']*)'|(?P<bare>[^\s>]+))[^>]*>"#)
        .unwrap()
});

/// Ce qu'il faut faire d'une référence d'image.
enum Action {
    Keep,
    Embed(String),
    Remote,
    Refused,
}

fn is_remote(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http:")
        || lower.starts_with("https:")
        || lower.starts_with("//")
        || lower.starts_with("ftp:")
}

fn classify(url: &str, base: &Path) -> Action {
    if url.starts_with("data:") {
        return Action::Keep;
    }
    if is_remote(url) {
        return Action::Remote;
    }
    match local_data_uri(url, base) {
        Some(uri) => Action::Embed(uri),
        None => Action::Refused,
    }
}

/// Décodage minimal des `%XX` d'un chemin d'URL.
fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') {
        return Cow::Borrowed(s);
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// `data:` URI d'une image locale **contenue** dans `base` (chemin canonique,
/// liens symboliques résolus) ; `None` sinon.
fn local_data_uri(url: &str, base: &Path) -> Option<String> {
    let decoded = percent_decode(url.split(['?', '#']).next()?);
    let rel = Path::new(decoded.as_ref());
    if rel.is_absolute() || decoded.contains(':') || decoded.starts_with('\\') {
        return None;
    }
    let root = base.canonicalize().ok()?;
    let target = root.join(rel).canonicalize().ok()?;
    let meta = target.metadata().ok()?;
    if !target.starts_with(&root) || !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mime = image_mime(&target)?;
    let data = std::fs::read(&target).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    Some(format!("data:{mime};base64,{b64}"))
}

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

/// Réécrit les références d'images (Markdown inline + balises `<img>`).
pub fn rewrite_images(text: &str, base: &Path, html_only: bool) -> ImageReport {
    let mut report = ImageReport::default();
    let apply = |url: &str, report: &mut ImageReport| {
        let action = classify(url, base);
        match &action {
            Action::Remote => report.remote.push(url.to_string()),
            Action::Refused => report.refused.push(url.to_string()),
            Action::Embed(_) => report.embedded += 1,
            Action::Keep => {}
        }
        action
    };
    let mut text = HTML_IMG
        .replace_all(text, |c: &Captures| {
            let url = ["dq", "sq", "bare"]
                .iter()
                .find_map(|k| c.name(k))
                .map_or("", |m| m.as_str());
            match apply(url, &mut report) {
                Action::Keep => c[0].to_string(),
                Action::Embed(uri) => c[0].replacen(url, &uri, 1),
                Action::Remote | Action::Refused => String::new(),
            }
        })
        .into_owned();
    if !html_only {
        text = MD_IMAGE
            .replace_all(&text, |c: &Captures| match apply(&c["url"], &mut report) {
                Action::Keep => c[0].to_string(),
                Action::Embed(uri) => format!("![{}]({uri})", &c["alt"]),
                Action::Remote | Action::Refused => c["alt"].to_string(),
            })
            .into_owned();
    }
    report.text = text;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{EventKind, JobInput, MemoryReporter, execute};
    use crate::registry::{self, Category};
    use crate::sidecar::Resolver;
    use std::fs;

    #[test]
    fn sandbox_toujours_en_tete() {
        let args = pandoc_args(
            "markdown",
            "docx",
            Path::new("a.md"),
            Path::new("a.docx"),
            &[],
        );
        assert_eq!(args[0], "--sandbox");
        assert_eq!(args.last().unwrap(), "a.md");
    }

    #[test]
    fn images_distantes_et_hors_dossier_retirees() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = tmp.path().join("doc");
        fs::create_dir(&doc).unwrap();
        fs::write(doc.join("logo.png"), b"\x89PNG").unwrap();
        fs::write(tmp.path().join("secret.png"), b"secret").unwrap();
        let md = "![a](https://pistage.example/p.png) ![b](logo.png) ![c](../secret.png) \
                  ![d](/etc/passwd) <img src=\"http://x.example/i.gif\"> ![e](data:image/png;base64,AA)";
        let r = rewrite_images(md, &doc, false);
        assert_eq!(
            r.remote,
            ["http://x.example/i.gif", "https://pistage.example/p.png"]
        );
        assert_eq!(r.refused, ["../secret.png", "/etc/passwd"]);
        assert_eq!(r.embedded, 1);
        assert!(
            r.text.contains("![b](data:image/png;base64,iVBORw==)"),
            "{}",
            r.text
        );
        assert!(!r.text.contains("pistage") && !r.text.contains("secret.png"));
        assert!(r.text.contains("data:image/png;base64,AA"));
    }

    #[cfg(unix)]
    #[test]
    fn lien_symbolique_vers_l_exterieur_refuse() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = tmp.path().join("doc");
        fs::create_dir(&doc).unwrap();
        fs::write(tmp.path().join("cle.png"), b"prive").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("cle.png"), doc.join("lien.png")).unwrap();
        let r = rewrite_images("![x](lien.png)", &doc, false);
        assert_eq!(r.refused, ["lien.png"]);
    }

    /// Faux pandoc : enregistre ses arguments et copie l'entrée vers `-o`.
    #[cfg(unix)]
    fn fake_pandoc(dir: &Path) -> Resolver {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("pandoc");
        fs::write(
            &script,
            "#!/bin/sh\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/args.txt\"\n\
             out=\"\"; prev=\"\"\nfor a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\n\
             for last; do :; done\ncat \"$last\" > \"$out\"\necho '[WARNING] avertissement factice' >&2\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        Resolver::new(Some(dir.to_path_buf()), None, None)
    }

    #[cfg(unix)]
    fn sample_for(tool: &registry::ToolDef, dir: &Path) -> PathBuf {
        let ext = tool.input.extensions[0];
        let path = dir.join(format!("entree.{ext}"));
        let body = if ext.starts_with("htm") {
            "<h1>Titre</h1><img alt=\"img\" src=\"https://exemple.invalid/a.png\">"
        } else {
            "# Titre\n\n![img](https://exemple.invalid/a.png)\n"
        };
        fs::write(&path, body).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn tous_les_outils_documents_en_sandbox() {
        let bin = tempfile::tempdir().unwrap();
        let resolver = fake_pandoc(bin.path());
        let tools = registry::all().into_iter().filter(|t| {
            t.category == Category::Documents && t.binaries == [crate::sidecar::Binary::Pandoc]
        });
        let mut count = 0;
        for tool in tools {
            let work = tempfile::tempdir().unwrap();
            let input = sample_for(tool, work.path());
            let rep = MemoryReporter::default();
            let outs = execute(tool.id, JobInput::new(vec![input.clone()]), &rep, &resolver)
                .unwrap_or_else(|e| panic!("{} : {e:#}", tool.id));
            let args = fs::read_to_string(bin.path().join("args.txt")).unwrap();
            assert_eq!(args.lines().next(), Some("--sandbox"), "{}", tool.id);
            assert!(outs[0].exists() && outs[0] != input, "{}", tool.id);
            let text_input = matches!(input_format(&input).unwrap(), "markdown" | "html");
            if text_input {
                // L'image distante a été retirée avant pandoc et signalée.
                assert!(
                    !fs::read_to_string(&outs[0])
                        .unwrap()
                        .contains("exemple.invalid")
                );
                let warnings = rep
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(k, m)| *k == EventKind::Warning && m.contains("Image distante"))
                    .count();
                assert_eq!(warnings, 1, "{}", tool.id);
            }
            assert!(
                rep.events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, m)| m.contains("avertissement factice"))
            );
            count += 1;
        }
        assert_eq!(count, 13);
    }

    #[cfg(unix)]
    #[test]
    fn echec_de_pandoc_erreur_typee_sans_sortie() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let script = bin.path().join("pandoc");
        fs::write(
            &script,
            "#!/bin/sh\necho 'pandoc: format inconnu' >&2\nexit 3\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let resolver = Resolver::new(Some(bin.path().to_path_buf()), None, None);
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("a.md");
        fs::write(&input, "x").unwrap();
        let err = execute(
            "documents-md-vers-docx",
            JobInput::new(vec![input]),
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("format inconnu"), "{err:#}");
        assert!(!work.path().join("a.docx").exists());
    }

    /// Vrais pandoc + typst (typst téléchargé et vérifié par l'installeur de
    /// l'app dans `.scratch/appdata/bin`) : Markdown (image locale incorporée)
    /// et Word → PDF ; le texte est relu dans le PDF.
    #[test]
    fn markdown_et_word_vers_pdf_avec_typst() {
        use crate::sidecar::Binary;
        let Some(r) =
            crate::tools::testutil::real(&[Binary::Pandoc, Binary::Typst, Binary::Ghostscript])
        else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        image::RgbImage::from_pixel(20, 10, image::Rgb([200, 0, 0]))
            .save(work.path().join("logo.png"))
            .unwrap();
        let md = work.path().join("note.md");
        let body = "# Compte rendu\n\nBonjour, voici l'été.\n\n![logo](logo.png)\n\n```{=typst}\n#read(\"note.md\")\n```\n";
        fs::write(&md, body).unwrap();
        let run = |id: &str, input: &Path| {
            execute(
                id,
                JobInput::new(vec![input.to_path_buf()]),
                &MemoryReporter::default(),
                &r,
            )
            .unwrap_or_else(|e| panic!("{id} : {e:#}"))
        };
        let pdf = run("documents-md-vers-pdf", &md);
        assert_eq!(pdf, vec![work.path().join("note.pdf")]);
        let text = crate::tools::testutil::gs_text(&r, &pdf[0]);
        assert!(
            text.contains("Compte rendu") && text.contains("été"),
            "{text}"
        );
        assert!(
            !text.contains("# Compte"),
            "bloc typst brut exécuté : {text}"
        );
        let raw = fs::read(&pdf[0]).unwrap();
        assert!(
            raw.windows(6).any(|w| w == b"/Image"),
            "image locale absente du PDF"
        );

        let docx = run("documents-md-vers-docx", &md);
        let pdf2 = run("documents-docx-vers-pdf", &docx[0]);
        assert_eq!(pdf2, vec![work.path().join("note-2.pdf")]);
        assert!(crate::tools::testutil::gs_text(&r, &pdf2[0]).contains("Compte rendu"));
        let raw = fs::read(&pdf2[0]).unwrap();
        assert!(
            raw.windows(6).any(|w| w == b"/Image"),
            "image du .docx absente du PDF"
        );
    }

    /// Les 13 autres conversions avec le vrai pandoc (chaînées depuis un .md).
    #[test]
    fn conversions_reelles_avec_pandoc() {
        use crate::sidecar::Binary;
        let Some(r) = crate::tools::testutil::real(&[Binary::Pandoc]) else {
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let md = work.path().join("doc.md");
        fs::write(&md, "# Titre\n\nPremier paragraphe, accentué.\n").unwrap();
        let probe = execute(
            "documents-md-vers-docx",
            JobInput::new(vec![md.clone()]),
            &MemoryReporter::default(),
            &r,
        );
        if let Err(e) = probe {
            assert!(e.to_string().contains("fichiers de données"), "{e:#}");
            eprintln!("pandoc de distribution sans fichiers de données : test réel ignoré");
            return;
        }
        let run = |id: &str, input: &Path| -> PathBuf {
            let outs = execute(
                id,
                JobInput::new(vec![input.to_path_buf()]),
                &MemoryReporter::default(),
                &r,
            )
            .unwrap_or_else(|e| panic!("{id} : {e:#}"));
            outs[0].clone()
        };
        let docx = run("documents-md-vers-docx", &md);
        let odt = run("documents-md-vers-odt", &md);
        let html = run("documents-md-vers-html", &md);
        let epub = run("documents-md-vers-epub", &md);
        let derived = [
            run("documents-docx-vers-md", &docx),
            run("documents-docx-vers-odt", &docx),
            run("documents-docx-vers-html", &docx),
            run("documents-odt-vers-docx", &odt),
            run("documents-odt-vers-md", &odt),
            run("documents-html-vers-md", &html),
            run("documents-html-vers-docx", &html),
            run("documents-epub-vers-md", &epub),
            run("documents-extraire-texte", &docx),
        ];
        let text = |p: &Path| match p.extension().and_then(|e| e.to_str()) {
            Some("txt") => p.to_path_buf(),
            _ => run("documents-extraire-texte", p),
        };
        for out in derived.iter().chain([&docx, &odt, &html, &epub]) {
            let t = fs::read_to_string(text(out)).unwrap();
            assert!(
                t.contains("Premier paragraphe, accentué."),
                "{} : {t}",
                out.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn pandoc_de_distribution_message_clair() {
        let bin = tempfile::tempdir().unwrap();
        crate::tools::testutil::fake_bin(
            bin.path(),
            "pandoc",
            "echo 'Could not find data file data/data/docx/[Content_Types].xml' >&2; exit 97",
        );
        let resolver = Resolver::new(Some(bin.path().to_path_buf()), None, None);
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("a.md");
        fs::write(&input, "x").unwrap();
        let err = execute(
            "documents-md-vers-docx",
            JobInput::new(vec![input]),
            &MemoryReporter::default(),
            &resolver,
        )
        .unwrap_err();
        assert!(err.to_string().contains("fichiers de données"), "{err}");
        assert!(!work.path().join("a.docx").exists());
    }

    #[test]
    fn pandoc_absent_message_clair() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("a.md");
        fs::write(&input, "x").unwrap();
        let err = execute(
            "documents-md-vers-html",
            JobInput::new(vec![input]),
            &MemoryReporter::default(),
            &Resolver::new(None, None, None),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Pandoc"), "{err}");
    }
}
