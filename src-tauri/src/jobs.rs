//! Exécution d'un outil : validation des entrées, options par défaut,
//! vérification des binaires, et remontée de progression via un [`Reporter`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::registry::{self, InputSpec, ToolDef};
use crate::sidecar::Resolver;
use crate::tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Progress,
    Log,
    Warning,
    Done,
    Error,
}

/// Événement envoyé au frontend (`job-event`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub job_id: u64,
    pub kind: EventKind,
    pub percent: Option<f32>,
    pub message: String,
    pub outputs: Vec<String>,
}

/// Canal de progression d'une tâche (Tauri en production, mémoire en test).
pub trait Reporter: Send + Sync {
    fn send(&self, kind: EventKind, percent: Option<f32>, message: &str);

    fn progress(&self, percent: f32, message: &str) {
        self.send(
            EventKind::Progress,
            Some(percent.clamp(0.0, 100.0)),
            message,
        );
    }

    fn log(&self, message: &str) {
        self.send(EventKind::Log, None, message);
    }

    fn warn(&self, message: &str) {
        self.send(EventKind::Warning, None, message);
    }
}

/// Entrées d'une tâche : fichiers + options (valeurs textuelles).
#[derive(Debug, Clone, Default)]
pub struct JobInput {
    pub inputs: Vec<PathBuf>,
    pub options: HashMap<String, String>,
}

impl JobInput {
    pub fn new(inputs: Vec<PathBuf>) -> Self {
        Self {
            inputs,
            options: HashMap::new(),
        }
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.options.insert(key.to_string(), value.to_string());
        self
    }

    /// Valeur d'une option ("" si absente).
    pub fn option(&self, key: &str) -> &str {
        self.options.get(key).map(String::as_str).unwrap_or("")
    }
}

/// Contexte passé aux implémentations d'outils.
pub struct ToolContext<'a> {
    pub input: &'a JobInput,
    pub reporter: &'a dyn Reporter,
    pub resolver: &'a Resolver,
}

/// Erreur renvoyée par tout outil déclaré mais pas encore implémenté.
pub fn not_implemented(tool: &ToolDef) -> anyhow::Error {
    anyhow!(
        "« {} » est déclaré mais non implémenté en phase 1 (prévu en phase {}).",
        tool.name,
        tool.status.number()
    )
}

/// Exécute l'outil `tool_id` de façon synchrone.
pub fn execute(
    tool_id: &str,
    input: JobInput,
    reporter: &dyn Reporter,
    resolver: &Resolver,
) -> Result<Vec<PathBuf>> {
    let tool = registry::find(tool_id).with_context(|| format!("outil inconnu : {tool_id}"))?;
    let run = tools::implementation(tool.id).ok_or_else(|| not_implemented(tool))?;
    check_binaries(tool, resolver)?;
    validate_inputs(&tool.input, &input.inputs)?;
    let input = with_defaults(tool, input);
    let ctx = ToolContext {
        input: &input,
        reporter,
        resolver,
    };
    run(&ctx)
}

fn check_binaries(tool: &ToolDef, resolver: &Resolver) -> Result<()> {
    let missing: Vec<&str> = tool
        .binaries
        .iter()
        .filter(|b| resolver.resolve(**b).is_none())
        .map(|b| b.label())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "Binaire(s) manquant(s) : {} — voir l'onglet Diagnostic.",
        missing.join(", ")
    )
}

fn with_defaults(tool: &ToolDef, mut input: JobInput) -> JobInput {
    for opt in tool.options {
        input
            .options
            .entry(opt.key.to_string())
            .or_insert_with(|| opt.default.to_string());
    }
    input
}

pub fn validate_inputs(spec: &InputSpec, inputs: &[PathBuf]) -> Result<()> {
    if inputs.len() < spec.min_files {
        bail!(
            "Il faut au moins {} fichier(s) pour cet outil.",
            spec.min_files
        );
    }
    if !spec.multiple && inputs.len() > 1 {
        bail!("Cet outil n'accepte qu'un seul fichier à la fois.");
    }
    inputs.iter().try_for_each(|p| validate_one(spec, p))
}

fn validate_one(spec: &InputSpec, path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("Fichier introuvable : {}", path.display()))?;
    if meta.is_dir() {
        if spec.directories {
            return Ok(());
        }
        bail!("Les dossiers ne sont pas acceptés : {}", path.display());
    }
    if spec.extensions.is_empty() {
        return Ok(());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if spec.extensions.contains(&ext.as_str()) {
        return Ok(());
    }
    bail!(
        "Format non accepté : {} (attendu : {})",
        path.display(),
        spec.extensions.join(", ")
    )
}

/// Reporter de test : garde les événements en mémoire.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryReporter {
    pub events: std::sync::Mutex<Vec<(EventKind, String)>>,
}

#[cfg(test)]
impl Reporter for MemoryReporter {
    fn send(&self, kind: EventKind, _percent: Option<f32>, message: &str) {
        self.events
            .lock()
            .unwrap()
            .push((kind, message.to_string()));
    }
}

#[cfg(test)]
impl MemoryReporter {
    pub fn count(&self, kind: EventKind) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| *k == kind)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Status;

    fn run(id: &str, input: JobInput) -> Result<Vec<PathBuf>> {
        execute(id, input, &MemoryReporter::default(), &Resolver::default())
    }

    #[test]
    fn outil_inconnu() {
        assert!(run("nexiste-pas", JobInput::default()).is_err());
    }

    #[test]
    fn implementations_conformes_au_statut() {
        for t in registry::all() {
            let implemented = tools::implementation(t.id).is_some();
            assert_eq!(implemented, t.status == Status::Phase1, "{}", t.id);
        }
    }

    #[test]
    fn outils_non_implementes_erreur_explicite() {
        for t in registry::all()
            .into_iter()
            .filter(|t| t.status != Status::Phase1)
        {
            let err = run(t.id, JobInput::default()).unwrap_err().to_string();
            assert!(
                err.contains("non implémenté en phase 1"),
                "{} : {err}",
                t.id
            );
        }
    }

    fn spec(id: &str) -> InputSpec {
        registry::find(id).unwrap().input
    }

    #[test]
    fn validation_nombre_et_format() {
        let tmp = tempfile::tempdir().unwrap();
        let txt = tmp.path().join("a.txt");
        std::fs::write(&txt, b"x").unwrap();
        let err = validate_inputs(&spec("pdf-fusionner"), std::slice::from_ref(&txt)).unwrap_err();
        assert!(err.to_string().contains("au moins 2"));
        let err = validate_inputs(&spec("pdf-fusionner"), &[txt.clone(), txt]).unwrap_err();
        assert!(err.to_string().contains("Format non accepté"));
    }

    #[test]
    fn validation_un_seul_fichier() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.pdf");
        std::fs::write(&p, b"x").unwrap();
        let err = validate_inputs(&spec("pdf-decouper"), &[p.clone(), p]).unwrap_err();
        assert!(err.to_string().contains("un seul fichier"));
    }

    #[test]
    fn validation_fichier_absent_et_dossier() {
        let tmp = tempfile::tempdir().unwrap();
        let s = spec("images-convertir");
        let err = validate_inputs(&s, &[tmp.path().join("x.png")]).unwrap_err();
        assert!(err.to_string().contains("introuvable"));
        let err = validate_inputs(&s, &[tmp.path().to_path_buf()]).unwrap_err();
        assert!(err.to_string().contains("dossiers"));
        assert!(validate_inputs(&spec("archives-creer-zip"), &[tmp.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn options_par_defaut_completees() {
        let tool = registry::find("images-convertir").unwrap();
        let input = with_defaults(tool, JobInput::default().with("format", "jpg"));
        assert_eq!(input.option("format"), "jpg");
        assert_eq!(input.option("qualite"), "90");
        assert_eq!(input.option("inconnue"), "");
    }

    #[test]
    fn extension_insensible_a_la_casse() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("SCAN.PDF");
        std::fs::write(&p, b"x").unwrap();
        assert!(validate_inputs(&spec("pdf-compresser"), &[p]).is_ok());
    }
}
