use super::{BASE, Category, Choice, InputSpec, OptionDef, OptionKind, Status, ToolDef, files};
use crate::sidecar::Binary;

const C: Category = Category::Archives;
const SEVEN: &[Binary] = &[Binary::SevenZip];

/// Fichiers et dossiers de tout type.
const ANY: InputSpec = InputSpec {
    directories: true,
    ..files(&[])
};

const ARCHIVE_EXT: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "iso", "cab",
];

pub const ZIP_LEVELS: &[Choice] = &[
    Choice {
        value: "aucune",
        label: "Aucune (stockage)",
    },
    Choice {
        value: "rapide",
        label: "Rapide",
    },
    Choice {
        value: "normale",
        label: "Normale",
    },
    Choice {
        value: "maximale",
        label: "Maximale",
    },
];

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        id: "archives-creer-zip",
        category: C,
        name: "Créer un ZIP",
        description: "Compresse des fichiers et dossiers dans une archive .zip (moteur intégré).",
        input: ANY,
        output: "<nom>.zip (un élément) ou archive.zip (plusieurs)",
        options: &[OptionDef {
            key: "compression",
            label: "Compression",
            kind: OptionKind::Choice {
                choices: ZIP_LEVELS,
            },
            default: "normale",
        }],
        status: Status::Disponible,
        ..BASE
    },
    ToolDef {
        id: "archives-extraire-zip",
        category: C,
        name: "Extraire un ZIP",
        description: "Extrait une archive .zip dans un nouveau dossier (protection anti « zip slip »).",
        input: files(&["zip"]),
        output: "dossier <nom>/",
        ..BASE
    },
    ToolDef {
        id: "archives-creer-7z",
        category: C,
        name: "Créer un 7z",
        description: "Compresse dans une archive .7z (forte compression, mot de passe possible).",
        binaries: SEVEN,
        input: ANY,
        output: "<nom>.7z",
        ..BASE
    },
    ToolDef {
        id: "archives-extraire-7z",
        category: C,
        name: "Extraire 7z / RAR / ISO…",
        description: "Extrait la plupart des formats d'archive avec 7-Zip.",
        binaries: SEVEN,
        input: files(ARCHIVE_EXT),
        output: "dossier <nom>/",
        ..BASE
    },
    ToolDef {
        id: "archives-creer-tar",
        category: C,
        name: "Créer un TAR",
        description: "Regroupe des fichiers dans une archive .tar (sans compression).",
        input: ANY,
        output: "<nom>.tar",
        ..BASE
    },
    ToolDef {
        id: "archives-creer-targz",
        category: C,
        name: "Créer un TAR.GZ",
        description: "Regroupe et compresse dans une archive .tar.gz.",
        input: ANY,
        output: "<nom>.tar.gz",
        ..BASE
    },
    ToolDef {
        id: "archives-extraire-tar",
        category: C,
        name: "Extraire un TAR / TAR.GZ",
        description: "Extrait une archive .tar ou .tar.gz dans un nouveau dossier.",
        input: files(&["tar", "gz", "tgz"]),
        output: "dossier <nom>/",
        ..BASE
    },
    ToolDef {
        id: "archives-lister",
        category: C,
        name: "Lister le contenu",
        description: "Écrit la liste des fichiers d'une archive (noms, tailles) dans un .txt.",
        binaries: SEVEN,
        input: files(ARCHIVE_EXT),
        output: "<nom>-contenu.txt",
        ..BASE
    },
    ToolDef {
        id: "archives-tester",
        category: C,
        name: "Tester une archive",
        description: "Vérifie l'intégrité d'une archive sans l'extraire.",
        binaries: SEVEN,
        input: files(ARCHIVE_EXT),
        output: "rapport dans le journal",
        ..BASE
    },
];
