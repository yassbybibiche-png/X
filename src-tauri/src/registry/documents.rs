use super::{BASE, Category, Status, ToolDef, files};
use crate::sidecar::Binary;

const PANDOC: &[Binary] = &[Binary::Pandoc];
const PANDOC_PDF: &[Binary] = &[Binary::Pandoc, Binary::Typst];

/// Conversion pandoc (toujours lancée avec `--sandbox`, voir PLAN.md §3.4).
const fn conv(
    id: &'static str,
    name: &'static str,
    input: &'static [&'static str],
    output: &'static str,
    binaries: &'static [Binary],
) -> ToolDef {
    ToolDef {
        id,
        category: Category::Documents,
        name,
        description: "Conversion locale avec pandoc en mode isolé (--sandbox) ; \
                      les images distantes sont retirées et signalées.",
        binaries,
        input: files(input),
        output,
        status: Status::Phase1,
        ..BASE
    }
}

/// Sortie PDF : nécessite typst en plus (pas encore implémenté).
const fn conv_pdf(id: &'static str, name: &'static str, input: &'static [&'static str]) -> ToolDef {
    ToolDef {
        status: Status::Phase1,
        ..conv(id, name, input, "<nom>.pdf", PANDOC_PDF)
    }
}

const MD: &[&str] = &["md", "markdown", "txt"];
const DOCX: &[&str] = &["docx"];
const ODT: &[&str] = &["odt"];
const HTML: &[&str] = &["html", "htm"];

pub const TOOLS: &[ToolDef] = &[
    conv(
        "documents-md-vers-docx",
        "Markdown vers Word",
        MD,
        "<nom>.docx",
        PANDOC,
    ),
    conv(
        "documents-md-vers-odt",
        "Markdown vers ODT",
        MD,
        "<nom>.odt",
        PANDOC,
    ),
    conv(
        "documents-md-vers-html",
        "Markdown vers HTML",
        MD,
        "<nom>.html (autonome)",
        PANDOC,
    ),
    conv(
        "documents-md-vers-epub",
        "Markdown vers EPUB",
        MD,
        "<nom>.epub",
        PANDOC,
    ),
    conv_pdf("documents-md-vers-pdf", "Markdown vers PDF", MD),
    conv(
        "documents-docx-vers-md",
        "Word vers Markdown",
        DOCX,
        "<nom>.md (images non extraites)",
        PANDOC,
    ),
    conv(
        "documents-docx-vers-odt",
        "Word vers ODT",
        DOCX,
        "<nom>.odt",
        PANDOC,
    ),
    conv(
        "documents-docx-vers-html",
        "Word vers HTML",
        DOCX,
        "<nom>.html",
        PANDOC,
    ),
    conv_pdf("documents-docx-vers-pdf", "Word vers PDF", DOCX),
    conv(
        "documents-odt-vers-docx",
        "ODT vers Word",
        ODT,
        "<nom>.docx",
        PANDOC,
    ),
    conv(
        "documents-odt-vers-md",
        "ODT vers Markdown",
        ODT,
        "<nom>.md",
        PANDOC,
    ),
    conv(
        "documents-html-vers-md",
        "HTML vers Markdown",
        HTML,
        "<nom>.md",
        PANDOC,
    ),
    conv(
        "documents-html-vers-docx",
        "HTML vers Word",
        HTML,
        "<nom>.docx",
        PANDOC,
    ),
    conv(
        "documents-epub-vers-md",
        "EPUB vers Markdown",
        &["epub"],
        "<nom>.md",
        PANDOC,
    ),
    ToolDef {
        id: "documents-extraire-texte",
        category: Category::Documents,
        name: "Extraire le texte",
        description: "Extrait le texte brut d'un document (Word, ODT, HTML, EPUB, Markdown, RTF).",
        binaries: PANDOC,
        input: files(&["docx", "odt", "html", "htm", "epub", "md", "rtf"]),
        output: "<nom>.txt",
        status: Status::Phase1,
        ..BASE
    },
];
