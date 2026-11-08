use super::{
    BASE, Category, Choice, IMAGE_EXT, OptionDef, OptionKind, Status, ToolDef, files, single,
};
use crate::sidecar::Binary;

const PDF: &[&str] = &["pdf"];
const GS: &[Binary] = &[Binary::Ghostscript];
const QPDF: &[Binary] = &[Binary::Qpdf];
const C: Category = Category::Pdf;

/// Langues de l'OCR (Tesseract).
pub(super) const OCR_LANG: OptionDef = OptionDef {
    key: "langue",
    label: "Langue du texte",
    kind: OptionKind::Choice {
        choices: &[
            Choice {
                value: "fra+eng",
                label: "Français + anglais",
            },
            Choice {
                value: "fra",
                label: "Français",
            },
            Choice {
                value: "eng",
                label: "Anglais",
            },
        ],
    },
    default: "fra+eng",
};

const PAGES: OptionDef = OptionDef {
    key: "pages",
    label: "Pages (ex. 1-3,5)",
    kind: OptionKind::Text,
    default: "",
};
const PASSWORD: OptionDef = OptionDef {
    key: "mot_de_passe",
    label: "Mot de passe",
    kind: OptionKind::Password,
    default: "",
};

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        id: "pdf-fusionner",
        category: C,
        name: "Fusionner des PDF",
        description: "Assemble plusieurs PDF en un seul, dans l'ordre de la liste, sans perte de qualité.",
        input: super::InputSpec {
            min_files: 2,
            ..files(PDF)
        },
        output: "<premier>-fusionne.pdf",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-decouper",
        category: C,
        name: "Découper un PDF",
        description: "Sépare un PDF en un fichier par page ou par plage de pages.",
        input: single(PDF),
        output: "<nom>-p<N>.pdf (un par page ou plage)",
        options: &[PAGES],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-extraire-pages",
        category: C,
        name: "Extraire des pages",
        description: "Copie les pages choisies dans un nouveau PDF.",
        input: single(PDF),
        output: "<nom>-extrait.pdf",
        options: &[PAGES],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-supprimer-pages",
        category: C,
        name: "Supprimer des pages",
        description: "Retire les pages choisies d'un PDF.",
        input: single(PDF),
        output: "<nom>-allege.pdf",
        options: &[PAGES],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-pivoter",
        category: C,
        name: "Pivoter des pages",
        description: "Fait pivoter tout ou partie des pages de 90, 180 ou 270 degrés.",
        input: files(PDF),
        output: "<nom>-pivote.pdf",
        options: &[
            OptionDef {
                key: "angle",
                label: "Angle",
                kind: OptionKind::Choice {
                    choices: &[
                        Choice {
                            value: "90",
                            label: "90° (horaire)",
                        },
                        Choice {
                            value: "180",
                            label: "180°",
                        },
                        Choice {
                            value: "270",
                            label: "270° (anti-horaire)",
                        },
                    ],
                },
                default: "90",
            },
            PAGES,
        ],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-compresser",
        category: C,
        name: "Compresser un PDF",
        description: "Réduit la taille d'un PDF en recompressant les images.",
        binaries: GS,
        input: files(PDF),
        output: "<nom>-compresse.pdf",
        options: &[OptionDef {
            key: "niveau",
            label: "Niveau",
            kind: OptionKind::Choice {
                choices: &[
                    Choice {
                        value: "screen",
                        label: "Écran (le plus léger)",
                    },
                    Choice {
                        value: "ebook",
                        label: "Liseuse (équilibré)",
                    },
                    Choice {
                        value: "printer",
                        label: "Impression (haute qualité)",
                    },
                ],
            },
            default: "ebook",
        }],
        status: Status::Phase1,
    },
    ToolDef {
        id: "pdf-ocr",
        category: C,
        name: "Rendre un PDF cherchable (OCR)",
        description: "Reconnaît le texte d'un PDF scanné (français + anglais) et ajoute une couche texte.",
        binaries: &[Binary::Ghostscript, Binary::Tesseract],
        input: files(PDF),
        output: "<nom>-ocr.pdf",
        options: &[OCR_LANG],
        status: Status::Phase1,
    },
    ToolDef {
        id: "pdf-vers-images",
        category: C,
        name: "PDF vers images",
        description: "Convertit chaque page en image PNG ou JPEG.",
        binaries: GS,
        input: files(PDF),
        output: "<nom>-p<N>.png / .jpg",
        options: &[
            OptionDef {
                key: "dpi",
                label: "Résolution (DPI)",
                kind: OptionKind::Number {
                    min: 36.0,
                    max: 600.0,
                    step: 1.0,
                },
                default: "150",
            },
            OptionDef {
                key: "format",
                label: "Format",
                kind: OptionKind::Choice {
                    choices: &[
                        Choice {
                            value: "png",
                            label: "PNG (sans perte)",
                        },
                        Choice {
                            value: "jpg",
                            label: "JPEG (plus léger)",
                        },
                    ],
                },
                default: "png",
            },
        ],
        status: Status::Phase1,
    },
    ToolDef {
        id: "pdf-images-vers-pdf",
        category: C,
        name: "Images vers PDF",
        description: "Assemble des images en un PDF, une image par page.",
        input: files(IMAGE_EXT),
        output: "<premiere>-images.pdf",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-proteger",
        category: C,
        name: "Protéger par mot de passe",
        description: "Chiffre un PDF (AES-256, qpdf) avec un mot de passe d'ouverture.",
        binaries: QPDF,
        input: single(PDF),
        output: "<nom>-protege.pdf",
        options: &[PASSWORD],
        status: Status::Phase1,
    },
    ToolDef {
        id: "pdf-deproteger",
        category: C,
        name: "Retirer le mot de passe",
        description: "Déchiffre un PDF dont vous connaissez le mot de passe (qpdf).",
        binaries: QPDF,
        input: single(PDF),
        output: "<nom>-deprotege.pdf",
        options: &[PASSWORD],
        status: Status::Phase1,
    },
    ToolDef {
        id: "pdf-filigrane",
        category: C,
        name: "Ajouter un filigrane",
        description: "Ajoute un texte en filigrane sur toutes les pages.",
        input: files(PDF),
        output: "<nom>-filigrane.pdf",
        options: &[
            OptionDef {
                key: "texte",
                label: "Texte du filigrane",
                kind: OptionKind::Text,
                default: "CONFIDENTIEL",
            },
            OptionDef {
                key: "opacite",
                label: "Opacité (%)",
                kind: OptionKind::Number {
                    min: 5.0,
                    max: 100.0,
                    step: 5.0,
                },
                default: "25",
            },
        ],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-vers-texte",
        category: C,
        name: "PDF vers texte",
        description: "Extrait la couche texte d'un PDF dans un fichier .txt (un PDF scanné nécessite l'OCR).",
        input: files(PDF),
        output: "<nom>.txt",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-numeroter",
        category: C,
        name: "Numéroter les pages",
        description: "Ajoute un numéro de page en pied de chaque page.",
        input: files(PDF),
        output: "<nom>-numerote.pdf",
        options: &[
            OptionDef {
                key: "format",
                label: "Format",
                kind: OptionKind::Choice {
                    choices: &[
                        Choice {
                            value: "n",
                            label: "3",
                        },
                        Choice {
                            value: "n-sur-total",
                            label: "3 / 10",
                        },
                        Choice {
                            value: "page-n",
                            label: "Page 3",
                        },
                    ],
                },
                default: "n-sur-total",
            },
            OptionDef {
                key: "position",
                label: "Position",
                kind: OptionKind::Choice {
                    choices: &[
                        Choice {
                            value: "bas-centre",
                            label: "En bas, au centre",
                        },
                        Choice {
                            value: "bas-droite",
                            label: "En bas, à droite",
                        },
                    ],
                },
                default: "bas-centre",
            },
        ],
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-niveaux-de-gris",
        category: C,
        name: "PDF en niveaux de gris",
        description: "Convertit toutes les couleurs d'un PDF en niveaux de gris.",
        binaries: GS,
        input: files(PDF),
        output: "<nom>-gris.pdf",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-reparer",
        category: C,
        name: "Réparer un PDF",
        description: "Tente de reconstruire un PDF abîmé ou mal formé.",
        binaries: GS,
        input: files(PDF),
        output: "<nom>-repare.pdf",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-supprimer-metadonnees",
        category: C,
        name: "Supprimer les métadonnées",
        description: "Efface auteur, logiciel, dates et métadonnées XMP d'un PDF.",
        input: files(PDF),
        output: "<nom>-sans-metadonnees.pdf",
        status: Status::Phase1,
        ..BASE
    },
    ToolDef {
        id: "pdf-infos",
        category: C,
        name: "Informations sur un PDF",
        description: "Rapport texte : nombre de pages, version, format, chiffrement, titre, auteur, dates.",
        input: files(PDF),
        output: "<nom>-infos.txt",
        status: Status::Phase1,
        ..BASE
    },
];
