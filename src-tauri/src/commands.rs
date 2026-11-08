//! Commandes Tauri exposées au frontend.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::jobs::{self, EventKind, JobEvent, JobInput, Reporter};
use crate::registry::{self, Catalog};
use crate::sidecar::{self, BinaryStatus, Resolver};

/// Nom de l'événement de progression.
pub const JOB_EVENT: &str = "job-event";

#[derive(Default)]
pub struct AppState {
    next_job: AtomicU64,
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

fn resolver(app: &AppHandle) -> Resolver {
    let data_bin = app.path().app_data_dir().ok().map(|d| d.join("bin"));
    Resolver::from_env(data_bin)
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
    let job_id = state.next_job.fetch_add(1, Ordering::Relaxed) + 1;
    let input = JobInput {
        inputs: inputs.into_iter().map(PathBuf::from).collect(),
        options,
    };
    let resolver = resolver(&app);
    let reporter = TauriReporter { app, job_id };
    tokio::task::spawn_blocking(move || finish_job(&tool_id, input, &reporter, &resolver));
    Ok(job_id)
}

fn finish_job(tool_id: &str, input: JobInput, reporter: &TauriReporter, resolver: &Resolver) {
    match jobs::execute(tool_id, input, reporter, resolver) {
        Ok(outputs) => {
            let outputs: Vec<String> = outputs.iter().map(|p| p.display().to_string()).collect();
            let message = format!("Terminé : {} fichier(s) créé(s).", outputs.len());
            reporter.emit(EventKind::Done, Some(100.0), &message, outputs);
        }
        Err(e) => reporter.emit(EventKind::Error, None, &format!("{e:#}"), Vec::new()),
    }
}
