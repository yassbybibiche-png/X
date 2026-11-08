//! Commandes Tauri exposées au frontend.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::cancel::{self, CancelToken};
use crate::download::{self, InstallHooks};
use crate::jobs::{self, EventKind, JobEvent, JobInput, Reporter};
use crate::registry::{self, Catalog};
use crate::sidecar::{self, Binary, BinaryStatus, Resolver};

/// Nom de l'événement de progression des tâches.
pub const JOB_EVENT: &str = "job-event";
/// Nom de l'événement de progression des téléchargements.
pub const DOWNLOAD_EVENT: &str = "download-event";

#[derive(Default)]
pub struct AppState {
    next_job: AtomicU64,
    /// Jetons d'annulation des tâches en cours (tâches et téléchargements).
    running: Mutex<HashMap<u64, CancelToken>>,
}

impl AppState {
    fn start(&self) -> (u64, CancelToken) {
        let id = self.next_job.fetch_add(1, Ordering::Relaxed) + 1;
        let token = CancelToken::default();
        if let Ok(mut map) = self.running.lock() {
            map.insert(id, token.clone());
        }
        (id, token)
    }

    fn finish(&self, id: u64) {
        if let Ok(mut map) = self.running.lock() {
            map.remove(&id);
        }
    }
}

/// Reporter qui émet des événements Tauri.
struct TauriReporter {
    app: AppHandle,
    job_id: u64,
}

impl TauriReporter {
    fn emit(&self, kind: EventKind, percent: Option<f32>, message: &str, outputs: Vec<String>) {
        let event = JobEvent {
            job_id: self.job_id,
            kind,
            percent,
            message: message.to_string(),
            outputs,
        };
        // Un échec d'émission (fenêtre fermée) ne doit pas interrompre la tâche.
        let _ = self.app.emit(JOB_EVENT, event);
    }
}

impl Reporter for TauriReporter {
    fn send(&self, kind: EventKind, percent: Option<f32>, message: &str) {
        self.emit(kind, percent, message, Vec::new());
    }
}

fn bin_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("bin"))
}

/// Résolveur : ressources `binaries/` de l'installeur, dossier de l'exécutable,
/// `<données app>/bin`, PATH.
fn resolver(app: &AppHandle) -> Resolver {
    let resolver = Resolver::from_env(bin_dir(app));
    match app.path().resource_dir() {
        Ok(dir) => resolver.with_bundled_dir(dir.join("binaries")),
        Err(_) => resolver,
    }
}

#[tauri::command]
pub fn list_tools() -> Catalog {
    registry::catalog()
}

#[tauri::command]
pub async fn check_tools(app: AppHandle) -> Result<Vec<BinaryStatus>, String> {
    let resolver = resolver(&app);
    tokio::task::spawn_blocking(move || sidecar::check_all(&resolver))
        .await
        .map_err(|e| e.to_string())
}

/// Lance un outil en arrière-plan ; renvoie l'identifiant de tâche.
#[tauri::command]
pub async fn run_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    tool_id: String,
    inputs: Vec<String>,
    options: HashMap<String, String>,
) -> Result<u64, String> {
    if registry::find(&tool_id).is_none() {
        return Err(format!("outil inconnu : {tool_id}"));
    }
    let (job_id, token) = state.start();
    let input = JobInput {
        inputs: inputs.into_iter().map(PathBuf::from).collect(),
        options,
    };
    let resolver = resolver(&app);
    let reporter = TauriReporter {
        app: app.clone(),
        job_id,
    };
    tokio::task::spawn_blocking(move || {
        finish_job(&tool_id, input, &reporter, &resolver, &token);
        app.state::<AppState>().finish(job_id);
    });
    Ok(job_id)
}

/// Demande l'annulation d'une tâche (le processus fils éventuel est tué).
/// Renvoie `false` si la tâche est déjà terminée.
#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: u64) -> bool {
    let map = state.running.lock();
    match map.ok().and_then(|m| m.get(&job_id).cloned()) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

fn finish_job(
    tool_id: &str,
    input: JobInput,
    reporter: &TauriReporter,
    resolver: &Resolver,
    token: &CancelToken,
) {
    match jobs::execute_with(tool_id, input, reporter, resolver, token) {
        Ok(outputs) => {
            let outputs: Vec<String> = outputs.iter().map(|p| p.display().to_string()).collect();
            let message = format!("Terminé : {} fichier(s) créé(s).", outputs.len());
            reporter.emit(EventKind::Done, Some(100.0), &message, outputs);
        }
        Err(e) if cancel::is_cancelled(&e) => {
            let message = "Tâche annulée : les fichiers partiels ont été supprimés.";
            reporter.emit(EventKind::Cancelled, None, message, Vec::new());
        }
        Err(e) => reporter.emit(EventKind::Error, None, &format!("{e:#}"), Vec::new()),
    }
}

/// Progression d'un téléchargement de binaire.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvent {
    pub job_id: u64,
    pub binary: &'static str,
    pub kind: EventKind,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

fn emit_download(app: &AppHandle, base: &DownloadEvent, kind: EventKind, message: &str) {
    let event = DownloadEvent {
        kind,
        message: message.to_string(),
        ..base.clone()
    };
    let _ = app.emit(DOWNLOAD_EVENT, event);
}

/// Télécharge un binaire tiers (manifeste épinglé, SHA-256 vérifié) dans le
/// dossier de données de l'app. Renvoie l'identifiant de tâche (annulable via
/// `cancel_job`) ; la fin est signalée par un événement `download-event`.
#[tauri::command]
pub async fn download_binary(
    app: AppHandle,
    state: State<'_, AppState>,
    binary: String,
) -> Result<u64, String> {
    let bin = Binary::ALL
        .into_iter()
        .find(|b| b.id() == binary)
        .ok_or_else(|| format!("binaire inconnu : {binary}"))?;
    let offer = download::offer(bin)?;
    let dir = bin_dir(&app).ok_or("dossier de données de l'application introuvable")?;
    let seven_zip = resolver(&app).resolve(Binary::SevenZip).map(|r| r.path);
    let (job_id, token) = state.start();
    tokio::task::spawn_blocking(move || {
        let base = DownloadEvent {
            job_id,
            binary: bin.id(),
            kind: EventKind::Progress,
            done: 0,
            total: offer.size,
            message: String::new(),
        };
        let progress = |done, total| {
            let event = DownloadEvent {
                done,
                total,
                ..base.clone()
            };
            let _ = app.emit(DOWNLOAD_EVENT, event);
        };
        let log = |m: &str| emit_download(&app, &base, EventKind::Log, m);
        let hooks = InstallHooks {
            progress: &progress,
            log: &log,
            cancel: Some(&token),
            seven_zip,
        };
        match download::install(&offer, &dir, &download::Http, &hooks) {
            Ok(exe) => {
                let message = format!("{} installé : {}", bin.label(), exe.display());
                emit_download(&app, &base, EventKind::Done, &message);
            }
            Err(e) if cancel::is_cancelled(&e) => {
                emit_download(&app, &base, EventKind::Cancelled, "Téléchargement annulé.");
            }
            Err(e) => emit_download(&app, &base, EventKind::Error, &format!("{e:#}")),
        }
        app.state::<AppState>().finish(job_id);
    });
    Ok(job_id)
}
