use super::{
    BASE, Category, Choice, InputSpec, OptionDef, OptionKind, Status, TEXT_INPUT, ToolDef, files,
    single,
};

const C: Category = Category::Utilitaires;

const fn int(
    key: &'static str,
    label: &'static str,
    (min, max): (f64, f64),
    default: &'static str,
) -> OptionDef {
    OptionDef {
        key,
        label,
        kind: OptionKind::Number {
            min,
            max,
            step: 1.0,
        },
        default,
    }
}

/// Fichiers et dossiers (parcourus récursivement).
const FILES_AND_DIRS: InputSpec = InputSpec {
    directories: true,
    ..files(&[])
};

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        id: "utilitaires-empreinte-sha256",
        category: C,
        name: "Empreinte SHA-256",
        description: "Calcule l'empreinte SHA-256 de fichiers (format compatible sha256sum).",
        input: files(&[]),
        output: "<nom.ext>.sha256",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-verifier-sha256",
        category: C,
        name: "Vérifier une empreinte",
        description: "Compare l'empreinte SHA-256 d'un fichier à la valeur attendue.",
        input: single(&[]),
        output: "verdict dans le journal",
        options: &[OptionDef {
            key: "attendu",
            label: "Empreinte attendue",
            kind: OptionKind::Text,
            default: "",
        }],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-renommer-lot",
        category: C,
        name: "Renommer en lot",
        description: "Crée des copies renommées selon un motif (originaux intacts, jamais d'écrasement).",
        input: files(&[]),
        output: "<motif>.<ext> (motif : {n} numéro, {nom} nom d'origine, {date} date)",
        options: &[
            OptionDef {
                key: "motif",
                label: "Motif ({n}, {nom}, {date})",
                kind: OptionKind::Text,
                default: "{nom}-{n}",
            },
            int("debut", "Premier numéro", (0.0, 1_000_000.0), "1"),
            int("chiffres", "Chiffres du numéro", (1.0, 9.0), "3"),
        ],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-doublons",
        category: C,
        name: "Rechercher les doublons",
        description: "Liste les fichiers au contenu identique (SHA-256) dans des dossiers. Rien n'est supprimé.",
        input: FILES_AND_DIRS,
        output: "doublons.txt",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-decouper",
        category: C,
        name: "Découper un gros fichier",
        description: "Découpe un fichier en parties de taille fixe (<nom>.001, .002…), pour un envoi ou une clé USB.",
        input: single(&[]),
        output: "<nom.ext>.001, <nom.ext>.002…",
        options: &[int(
            "taille",
            "Taille d'une partie (Mo)",
            (1.0, 1_000_000.0),
            "100",
        )],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-reassembler",
        category: C,
        name: "Réassembler un fichier découpé",
        description: "Recolle les parties .001, .002… (déposez la partie .001 ou toutes les parties).",
        input: files(&[]),
        output: "<nom.ext>",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-qrcode",
        category: C,
        name: "Générer un QR code",
        description: "Crée une image PNG de QR code à partir d'un texte ou d'un lien.",
        input: TEXT_INPUT,
        output: "Images/qrcode.png",
        options: &[
            OptionDef {
                key: "texte",
                label: "Texte ou lien",
                kind: OptionKind::Text,
                default: "",
            },
            OptionDef {
                key: "correction",
                label: "Correction d'erreur",
                kind: OptionKind::Choice {
                    choices: &[
                        Choice {
                            value: "L",
                            label: "Faible (7 %)",
                        },
                        Choice {
                            value: "M",
                            label: "Moyenne (15 %)",
                        },
                        Choice {
                            value: "Q",
                            label: "Forte (25 %)",
                        },
                        Choice {
                            value: "H",
                            label: "Maximale (30 %)",
                        },
                    ],
                },
                default: "M",
            },
            int("taille", "Pixels par module", (1.0, 50.0), "10"),
            OptionDef {
                key: "dossier",
                label: "Dossier de destination (vide = Images)",
                kind: OptionKind::Text,
                default: "",
            },
        ],
        status: Status::Phase1,
        ..BASE
    },
];
