//! Modèles IA téléchargés au premier usage (whisper.cpp).
//!
//! Emplacement : `<données app>/models/<id>/<fichier>` (le dossier `bin` du
//! résolveur a pour parent `<données app>`). Un dépôt manuel est aussi accepté
//! dans `<dossier de whisper-cli>/models/<fichier>`.
//! Les modèles Real-ESRGAN sont livrés dans le zip du binaire (`models/`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::download;
use crate::sidecar::{Delivery, Resolver, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Model {
    WhisperSmall,
    WhisperBase,
}

impl Model {
    pub const ALL: [Model; 2] = [Model::WhisperSmall, Model::WhisperBase];

    /// Clé du manifeste (`models`) et identifiant du Diagnostic.
    pub fn id(self) -> &'static str {
        match self {
            Model::WhisperSmall => "whisper-small",
            Model::WhisperBase => "whisper-base",
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Model::WhisperSmall => "ggml-small-q5_1.bin",
            Model::WhisperBase => "ggml-base-q5_1.bin",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::WhisperSmall => "Modèle whisper « small » (q5_1, ≈ 190 Mo)",
            Model::WhisperBase => "Modèle whisper « base » (q5_1, ≈ 60 Mo, plus rapide)",
        }
    }

    pub fn from_option(value: &str) -> Option<Model> {
        match value {
            "small" => Some(Model::WhisperSmall),
            "base" => Some(Model::WhisperBase),
            _ => None,
        }
    }

    pub fn from_id(id: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// `<données app>/models` d'après le dossier `bin` du résolveur.
pub fn models_dir(resolver: &Resolver) -> Option<PathBuf> {
    resolver.data_dir()?.parent().map(|d| d.join("models"))
}

/// Fichier du modèle s'il est présent (téléchargé ou déposé à la main).
pub fn locate(resolver: &Resolver, model: Model, whisper_exe: Option<&Path>) -> Option<PathBuf> {
    let downloaded = models_dir(resolver).map(|d| d.join(model.id()).join(model.file()));
    let manual = whisper_exe
        .and_then(Path::parent)
        .map(|d| d.join("models").join(model.file()));
    [downloaded, manual]
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
}

/// Message d'erreur quand le modèle manque (avec la raison du manifeste).
pub fn missing_message(resolver: &Resolver, model: Model) -> String {
    let place = models_dir(resolver)
        .map(|d| d.join(model.id()).display().to_string())
        .unwrap_or_else(|| "<données de l'app>/models".into());
    match download::model_offer(model.id()) {
        Ok(_) => format!(
            "{} absent : téléchargez-le depuis l'onglet Diagnostic (ou placez {} dans {place}).",
            model.label(),
            model.file()
        ),
        Err(reason) => format!("{} absent. {reason}", model.label()),
    }
}

/// Ligne « modèle » de l'onglet Diagnostic (même forme qu'un binaire).
pub fn status(resolver: &Resolver, model: Model) -> crate::sidecar::BinaryStatus {
    let found = locate(resolver, model, None);
    let offer = download::model_offer(model.id());
    let state = match (&found, &offer) {
        (Some(_), _) => State::Present,
        (None, Ok(_)) => State::Telechargeable,
        (None, Err(_)) => State::Absent,
    };
    crate::sidecar::BinaryStatus {
        id: model.id(),
        label: model.label(),
        license: "MIT",
        delivery: Delivery::FirstUse,
        present: found.is_some(),
        state,
        source: found.as_ref().map(|_| crate::sidecar::Source::AppData),
        path: found.map(|p| p.display().to_string()),
        version: None,
        pinned_version: offer.as_ref().ok().map(|o| o.version.clone()),
        download_size: offer.as_ref().ok().map(|o| o.size),
        unavailable_reason: offer.err(),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emplacements_du_modele() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver = Resolver::new(None, Some(tmp.path().join("bin")), None);
        assert_eq!(locate(&resolver, Model::WhisperSmall, None), None);
        let msg = missing_message(&resolver, Model::WhisperSmall);
        assert!(msg.contains("ggml-small-q5_1.bin"), "{msg}");
        let st = status(&resolver, Model::WhisperSmall);
        assert_eq!(st.state, State::Absent);
        assert!(st.unavailable_reason.unwrap().contains("SHA-256"));

        let dir = tmp.path().join("models/whisper-small");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ggml-small-q5_1.bin"), b"x").unwrap();
        assert_eq!(
            locate(&resolver, Model::WhisperSmall, None),
            Some(dir.join("ggml-small-q5_1.bin"))
        );
        assert_eq!(status(&resolver, Model::WhisperSmall).state, State::Present);
    }
}
