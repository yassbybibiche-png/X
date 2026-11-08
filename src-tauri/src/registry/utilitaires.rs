use super::{BASE, Category, OptionDef, OptionKind, Status, TEXT_INPUT, ToolDef, files, single};

const C: Category = Category::Utilitaires;

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        id: "utilitaires-empreinte-sha256",
        category: C,
        name: "Empreinte SHA-256",
        description: "Calcule l'empreinte SHA-256 de fichiers (format compatible sha256sum).",
        input: files(&[]),
        output: "<nom.ext>.sha256",
        status: Status::Disponible,
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
        ..BASE
    },
    ToolDef {
        id: "utilitaires-renommer-lot",
        category: C,
        name: "Renommer en lot",
        description: "Crée des copies renommées selon un motif (originaux intacts, jamais d'écrasement).",
        input: files(&[]),
        output: "<motif>-<N>.<ext>",
        status: Status::Phase3,
        ..BASE
    },
    ToolDef {
        id: "utilitaires-qrcode",
        category: C,
        name: "Générer un QR code",
        description: "Crée une image PNG de QR code à partir d'un texte ou d'un lien.",
        input: TEXT_INPUT,
        output: "Images/qrcode.png",
        status: Status::Phase3,
        ..BASE
    },
];
