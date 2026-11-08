//! Registre des outils : métadonnées en français, sérialisées pour le frontend.

mod archives;
mod documents;
mod images;
mod media;
mod pdf;
mod utilitaires;

use serde::Serialize;

use crate::sidecar::Binary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Pdf,
    Documents,
    Images,
    VideoAudio,
    Archives,
    Utilitaires,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Pdf,
        Category::Documents,
        Category::Images,
        Category::VideoAudio,
        Category::Archives,
        Category::Utilitaires,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Pdf => "PDF",
            Category::Documents => "Documents",
            Category::Images => "Images",
            Category::VideoAudio => "Vidéo et audio",
            Category::Archives => "Archives",
            Category::Utilitaires => "Utilitaires",
        }
    }
}

/// État d'un outil (voir PLAN.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Implémenté et testé : exécutable (quelle que soit la phase de livraison).
    Phase1,
    /// Prévu en phase 2.
    Phase2,
    /// Prévu en phase 3.
    Phase3,
}

impl Status {
    /// Cellule « statut » de la ligne de l'outil dans PLAN.md.
    pub fn plan_cell(self) -> &'static str {
        match self {
            Status::Phase1 => "P1",
            Status::Phase2 => "P2",
            Status::Phase3 => "P3",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputSpec {
    /// Extensions acceptées (minuscules, sans point). Vide = tout fichier.
    pub extensions: &'static [&'static str],
    pub min_files: usize,
    pub multiple: bool,
    pub directories: bool,
    /// L'outil prend une URL (yt-dlp) ou un texte, pas de fichier.
    pub text_input: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OptionKind {
    Choice { choices: &'static [Choice] },
    Number { min: f64, max: f64, step: f64 },
    Text,
    Password,
    Bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct OptionDef {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: OptionKind,
    pub default: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ToolDef {
    pub id: &'static str,
    pub category: Category,
    pub name: &'static str,
    pub description: &'static str,
    pub binaries: &'static [Binary],
    pub input: InputSpec,
    pub output: &'static str,
    pub options: &'static [OptionDef],
    pub status: Status,
}

/// Gabarit commun aux déclarations (`..BASE`).
const BASE: ToolDef = ToolDef {
    id: "",
    category: Category::Utilitaires,
    name: "",
    description: "",
    binaries: &[],
    input: InputSpec {
        extensions: &[],
        min_files: 1,
        multiple: true,
        directories: false,
        text_input: false,
    },
    output: "",
    options: &[],
    status: Status::Phase2,
};

const fn files(extensions: &'static [&'static str]) -> InputSpec {
    InputSpec {
        extensions,
        ..BASE.input
    }
}

const fn single(extensions: &'static [&'static str]) -> InputSpec {
    InputSpec {
        extensions,
        multiple: false,
        ..BASE.input
    }
}

const TEXT_INPUT: InputSpec = InputSpec {
    extensions: &[],
    min_files: 0,
    multiple: false,
    directories: false,
    text_input: true,
};

const IMAGE_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "ico", "tga", "qoi",
];
const VIDEO_EXT: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "mpg", "mpeg",
];
const AUDIO_EXT: &[&str] = &["mp3", "m4a", "wav", "flac", "ogg", "opus", "aac", "wma"];

/// Tous les outils, dans l'ordre d'affichage.
pub fn all() -> Vec<&'static ToolDef> {
    [
        pdf::TOOLS,
        documents::TOOLS,
        images::TOOLS,
        media::TOOLS,
        archives::TOOLS,
        utilitaires::TOOLS,
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn find(id: &str) -> Option<&'static ToolDef> {
    all().into_iter().find(|t| t.id == id)
}

#[derive(Debug, Serialize)]
pub struct CategoryInfo {
    pub id: Category,
    pub label: &'static str,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub categories: Vec<CategoryInfo>,
    pub tools: Vec<&'static ToolDef>,
}

pub fn catalog() -> Catalog {
    let tools = all();
    let categories = Category::ALL
        .iter()
        .map(|c| CategoryInfo {
            id: *c,
            label: c.label(),
            count: tools.iter().filter(|t| t.category == *c).count(),
        })
        .collect();
    Catalog { categories, tools }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registre_complet() {
        assert_eq!(all().len(), 82);
    }

    #[test]
    fn ids_uniques_et_prefixes_par_categorie() {
        let mut seen = HashSet::new();
        for t in all() {
            assert!(seen.insert(t.id), "id en double : {}", t.id);
            let prefix = match t.category {
                Category::Pdf => "pdf-",
                Category::Documents => "documents-",
                Category::Images => "images-",
                Category::VideoAudio => {
                    if t.id.starts_with("audio-") {
                        "audio-"
                    } else {
                        "video-"
                    }
                }
                Category::Archives => "archives-",
                Category::Utilitaires => "utilitaires-",
            };
            assert!(t.id.starts_with(prefix), "préfixe incorrect : {}", t.id);
        }
    }

    #[test]
    fn metadonnees_renseignees() {
        for t in all() {
            assert!(!t.name.is_empty() && !t.description.is_empty(), "{}", t.id);
            assert!(!t.output.is_empty(), "sortie non décrite : {}", t.id);
        }
    }

    #[test]
    fn categories_minimales_du_cahier_des_charges() {
        let count = |c| all().iter().filter(|t| t.category == c).count();
        assert!(count(Category::Pdf) >= 9);
        assert!(count(Category::Documents) >= 12);
        assert!(count(Category::Images) >= 12);
        assert!(count(Category::VideoAudio) >= 14);
        assert!(count(Category::Archives) >= 6);
    }

    #[test]
    fn chaque_outil_figure_dans_le_plan() {
        let plan = include_str!("../../../PLAN.md");
        for t in all() {
            assert!(
                plan.contains(&format!("| {} |", t.id)),
                "absent du PLAN : {}",
                t.id
            );
        }
    }

    /// Le statut de chaque outil dans le registre est celui annoncé dans PLAN.md.
    #[test]
    fn statuts_conformes_au_plan() {
        let plan = include_str!("../../../PLAN.md");
        for t in all() {
            let row = plan
                .lines()
                .find(|l| l.starts_with(&format!("| {} |", t.id)))
                .unwrap_or_else(|| panic!("ligne absente du PLAN : {}", t.id));
            let cell = row.trim_end_matches('|').rsplit('|').next().unwrap();
            assert_eq!(
                cell.trim().trim_matches('*'),
                t.status.plan_cell(),
                "{}",
                t.id
            );
        }
    }

    #[test]
    fn options_par_defaut_valides() {
        for t in all() {
            for o in t.options {
                if let OptionKind::Choice { choices } = o.kind {
                    assert!(
                        choices.iter().any(|c| c.value == o.default),
                        "{}:{}",
                        t.id,
                        o.key
                    );
                }
            }
        }
    }

    #[test]
    fn catalogue_serialisable() {
        let json = serde_json::to_string(&catalog()).unwrap();
        assert!(json.contains("\"pdf-fusionner\""));
        assert!(json.contains("\"status\":\"phase1\""));
    }
}
