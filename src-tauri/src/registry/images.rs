use super::{
    BASE, Category, Choice, IMAGE_EXT, OptionDef, OptionKind, Status, ToolDef, files, single,
};
use crate::sidecar::Binary;

const C: Category = Category::Images;
const MAGICK: &[Binary] = &[Binary::ImageMagick];

pub const FORMAT_CHOICES: &[Choice] = &[
    Choice {
        value: "png",
        label: "PNG",
    },
    Choice {
        value: "jpg",
        label: "JPEG",
    },
    Choice {
        value: "webp",
        label: "WebP (sans perte)",
    },
    Choice {
        value: "bmp",
        label: "BMP",
    },
    Choice {
        value: "tiff",
        label: "TIFF",
    },
    Choice {
        value: "gif",
        label: "GIF",
    },
    Choice {
        value: "ico",
        label: "ICO (icône)",
    },
    Choice {
        value: "tga",
        label: "TGA",
    },
    Choice {
        value: "qoi",
        label: "QOI",
    },
    Choice {
        value: "avif",
        label: "AVIF (ImageMagick, phase 2)",
    },
];

const QUALITY: OptionDef = OptionDef {
    key: "qualite",
    label: "Qualité JPEG (1-100)",
    kind: OptionKind::Number {
        min: 1.0,
        max: 100.0,
        step: 1.0,
    },
    default: "90",
};

const fn number(
    key: &'static str,
    label: &'static str,
    max: f64,
    default: &'static str,
) -> OptionDef {
    OptionDef {
        key,
        label,
        kind: OptionKind::Number {
            min: 0.0,
            max,
            step: 1.0,
        },
        default,
    }
}

const fn choice(
    key: &'static str,
    label: &'static str,
    choices: &'static [Choice],
    default: &'static str,
) -> OptionDef {
    OptionDef {
        key,
        label,
        kind: OptionKind::Choice { choices },
        default,
    }
}

const RESIZE_MODES: &[Choice] = &[
    Choice {
        value: "cote-max",
        label: "Côté le plus long (px, sans agrandir)",
    },
    Choice {
        value: "largeur",
        label: "Largeur (px)",
    },
    Choice {
        value: "hauteur",
        label: "Hauteur (px)",
    },
    Choice {
        value: "pourcent",
        label: "Pourcentage",
    },
];
const ANGLES: &[Choice] = &[
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
];
const AXES: &[Choice] = &[
    Choice {
        value: "horizontal",
        label: "Horizontal (miroir gauche-droite)",
    },
    Choice {
        value: "vertical",
        label: "Vertical (haut-bas)",
    },
];

const fn ready(tool: ToolDef, options: &'static [OptionDef]) -> ToolDef {
    ToolDef {
        options,
        status: Status::Phase1,
        ..tool
    }
}

const fn simple(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    output: &'static str,
) -> ToolDef {
    ToolDef {
        id,
        category: C,
        name,
        description,
        input: files(IMAGE_EXT),
        output,
        ..BASE
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        id: "images-convertir",
        category: C,
        name: "Convertir des images",
        description: "Convertit des images vers PNG, JPEG, WebP, BMP, TIFF, GIF, ICO, TGA ou QOI (moteur intégré).",
        input: files(IMAGE_EXT),
        output: "<nom>.<format>",
        options: &[
            OptionDef {
                key: "format",
                label: "Format de sortie",
                kind: OptionKind::Choice {
                    choices: FORMAT_CHOICES,
                },
                default: "png",
            },
            QUALITY,
        ],
        status: Status::Phase1,
        ..BASE
    },
    ready(
        simple(
            "images-redimensionner",
            "Redimensionner",
            "Change la taille (pixels ou pourcentage), proportions conservées.",
            "<nom>-redim.<ext>",
        ),
        &[
            choice("mode", "Mode", RESIZE_MODES, "cote-max"),
            number("valeur", "Valeur", 20000.0, "1600"),
        ],
    ),
    ready(
        simple(
            "images-compresser",
            "Compresser",
            "Réduit le poids des images en JPEG (qualité réglable, transparence sur fond blanc).",
            "<nom>-compresse.jpg",
        ),
        &[number("qualite", "Qualité JPEG (1-100)", 100.0, "75")],
    ),
    ready(
        simple(
            "images-supprimer-exif",
            "Supprimer les données EXIF",
            "Retire position GPS, appareil et autres métadonnées (JPEG : sans réencodage).",
            "<nom>-sans-exif.<ext>",
        ),
        &[],
    ),
    ToolDef {
        id: "images-agrandir-ia",
        category: C,
        name: "Agrandir par IA (x4)",
        description: "Agrandit une image avec Real-ESRGAN (modèle local téléchargé au premier usage).",
        binaries: &[Binary::RealEsrgan],
        input: files(IMAGE_EXT),
        output: "<nom>-x4.png",
        status: Status::Phase3,
        ..BASE
    },
    ToolDef {
        id: "images-detourer",
        category: C,
        name: "Détourage simple",
        description: "Rend transparent un fond uni (couleur et tolérance réglables).",
        binaries: MAGICK,
        input: files(IMAGE_EXT),
        output: "<nom>-detoure.png",
        status: Status::Phase3,
        ..BASE
    },
    ready(
        simple(
            "images-pivoter",
            "Pivoter",
            "Fait pivoter de 90, 180 ou 270 degrés.",
            "<nom>-pivote.<ext>",
        ),
        &[choice("angle", "Angle", ANGLES, "90")],
    ),
    ready(
        simple(
            "images-retourner",
            "Retourner (miroir)",
            "Retourne horizontalement ou verticalement.",
            "<nom>-retourne.<ext>",
        ),
        &[choice("axe", "Axe", AXES, "horizontal")],
    ),
    ready(
        ToolDef {
            input: single(IMAGE_EXT),
            ..simple(
                "images-rogner",
                "Rogner",
                "Recadre l'image sur un rectangle.",
                "<nom>-rogne.<ext>",
            )
        },
        &[
            number("x", "Gauche (px)", 20000.0, "0"),
            number("y", "Haut (px)", 20000.0, "0"),
            number("largeur", "Largeur (px)", 20000.0, "100"),
            number("hauteur", "Hauteur (px)", 20000.0, "100"),
        ],
    ),
    ready(
        simple(
            "images-niveaux-de-gris",
            "Niveaux de gris",
            "Convertit en noir et blanc (niveaux de gris).",
            "<nom>-gris.<ext>",
        ),
        &[],
    ),
    ToolDef {
        binaries: MAGICK,
        status: Status::Phase3,
        ..simple(
            "images-filigrane",
            "Filigrane texte",
            "Ajoute un texte en filigrane.",
            "<nom>-filigrane.<ext>",
        )
    },
    ToolDef {
        input: single(IMAGE_EXT),
        ..simple(
            "images-vers-ico",
            "Créer une icône",
            "Crée un fichier .ico multi-tailles (16 à 256 px).",
            "<nom>.ico",
        )
    },
    ToolDef {
        binaries: MAGICK,
        status: Status::Phase3,
        ..simple(
            "images-planche-contact",
            "Planche contact",
            "Assemble des vignettes sur une seule image.",
            "planche.jpg",
        )
    },
    simple(
        "images-bordure",
        "Ajouter une bordure",
        "Ajoute un cadre de couleur et d'épaisseur choisies.",
        "<nom>-bordure.<ext>",
    ),
    simple(
        "images-optimiser-web",
        "Optimiser pour le web",
        "Redimensionne (côté max.) et recompresse pour le web.",
        "<nom>-web.jpg / .webp",
    ),
    ToolDef {
        binaries: &[Binary::Tesseract],
        status: Status::Phase3,
        ..simple(
            "images-ocr-texte",
            "Image vers texte (OCR)",
            "Reconnaît le texte d'une image (français + anglais).",
            "<nom>.txt",
        )
    },
    simple(
        "images-assembler-gif",
        "Créer un GIF animé",
        "Assemble des images en GIF animé (délai réglable).",
        "<premiere>-anime.gif",
    ),
];
