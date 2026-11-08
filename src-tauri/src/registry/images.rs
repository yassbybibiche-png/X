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
        status: Status::Disponible,
        ..BASE
    },
    simple(
        "images-redimensionner",
        "Redimensionner",
        "Change la taille (pixels ou pourcentage), proportions conservées.",
        "<nom>-redim.<ext>",
    ),
    simple(
        "images-compresser",
        "Compresser",
        "Réduit le poids des images (qualité réglable).",
        "<nom>-compresse.<ext>",
    ),
    simple(
        "images-supprimer-exif",
        "Supprimer les données EXIF",
        "Retire position GPS, appareil et autres métadonnées.",
        "<nom>-sans-exif.<ext>",
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
    simple(
        "images-pivoter",
        "Pivoter",
        "Fait pivoter de 90, 180 ou 270 degrés.",
        "<nom>-pivote.<ext>",
    ),
    simple(
        "images-retourner",
        "Retourner (miroir)",
        "Retourne horizontalement ou verticalement.",
        "<nom>-retourne.<ext>",
    ),
    ToolDef {
        input: single(IMAGE_EXT),
        ..simple(
            "images-rogner",
            "Rogner",
            "Recadre l'image sur un rectangle.",
            "<nom>-rogne.<ext>",
        )
    },
    simple(
        "images-niveaux-de-gris",
        "Niveaux de gris",
        "Convertit en noir et blanc (niveaux de gris).",
        "<nom>-gris.<ext>",
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
