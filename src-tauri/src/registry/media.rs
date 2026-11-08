use super::{
    AUDIO_EXT, BASE, Category, Choice, OptionDef, OptionKind, Status, TEXT_INPUT, ToolDef,
    VIDEO_EXT, files,
};
use crate::sidecar::Binary;

const FFMPEG: &[Binary] = &[Binary::Ffmpeg];
const WHISPER: &[Binary] = &[Binary::Ffmpeg, Binary::Whisper];
const YTDLP: &[Binary] = &[Binary::YtDlp, Binary::Ffmpeg];

const fn ff(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    input: &'static [&'static str],
    output: &'static str,
) -> ToolDef {
    ToolDef {
        id,
        category: Category::VideoAudio,
        name,
        description,
        binaries: FFMPEG,
        input: files(input),
        output,
        ..BASE
    }
}

/// Vidéo et audio (outil « infos »).
const MEDIA_EXT: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "mpg", "mpeg", "mp3", "m4a", "wav",
    "flac", "ogg", "opus", "aac", "wma",
];

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

const fn text(key: &'static str, label: &'static str, default: &'static str) -> OptionDef {
    OptionDef {
        key,
        label,
        kind: OptionKind::Text,
        default,
    }
}

const fn c(value: &'static str, label: &'static str) -> Choice {
    Choice { value, label }
}

const VIDEO_FORMATS: &[Choice] = &[
    c("mp4", "MP4 (H.264 + AAC)"),
    c("mkv", "MKV (H.264 + AAC)"),
    c("webm", "WebM (VP9 + Opus)"),
    c("mov", "MOV (H.264 + AAC)"),
];
const EXTRACT_FORMATS: &[Choice] = &[
    c("mp3", "MP3"),
    c("m4a", "M4A (AAC)"),
    c("wav", "WAV (sans perte)"),
    c("flac", "FLAC (sans perte)"),
];
const AUDIO_FORMATS: &[Choice] = &[
    c("mp3", "MP3"),
    c("m4a", "M4A (AAC)"),
    c("ogg", "OGG (Vorbis)"),
    c("opus", "Opus"),
    c("wav", "WAV (sans perte)"),
    c("flac", "FLAC (sans perte)"),
];
const PRECISION: &[Choice] = &[
    c("precise", "Précise (réencodage)"),
    c("rapide", "Rapide (sans réencodage, coupe aux images clés)"),
];
const CUT: &[OptionDef] = &[
    text("debut", "Début (ex. 1:30 ou 90)", "0"),
    text("fin", "Fin (vide = jusqu'au bout)", ""),
    choice("precision", "Précision", PRECISION, "precise"),
];

const fn ready(tool: ToolDef, options: &'static [OptionDef]) -> ToolDef {
    ToolDef {
        options,
        status: Status::Phase1,
        ..tool
    }
}

const V: &[&str] = VIDEO_EXT;
const A: &[&str] = AUDIO_EXT;

pub const TOOLS: &[ToolDef] = &[
    ready(
        ff(
            "video-convertir",
            "Convertir une vidéo",
            "Convertit vers MP4, MKV, WebM ou MOV.",
            V,
            "<nom>.<format>",
        ),
        &[choice("format", "Format", VIDEO_FORMATS, "mp4")],
    ),
    ready(
        ff(
            "video-compresser",
            "Compresser une vidéo",
            "Réduit le poids (H.264, qualité CRF réglable).",
            V,
            "<nom>-compresse.mp4",
        ),
        &[OptionDef {
            key: "crf",
            label: "Qualité CRF (18 = haute, 35 = très légère)",
            kind: OptionKind::Number {
                min: 18.0,
                max: 40.0,
                step: 1.0,
            },
            default: "28",
        }],
    ),
    ready(
        ff(
            "video-couper",
            "Couper une vidéo",
            "Garde l'extrait entre un début et une fin.",
            V,
            "<nom>-extrait.<ext>",
        ),
        CUT,
    ),
    ready(
        ff(
            "video-extraire-audio",
            "Extraire l'audio",
            "Récupère la piste son en MP3, M4A, WAV ou FLAC.",
            V,
            "<nom>.<format>",
        ),
        &[choice("format", "Format", EXTRACT_FORMATS, "mp3")],
    ),
    ff(
        "video-vers-gif",
        "Vidéo vers GIF",
        "Crée un GIF animé (palette optimisée) d'un extrait.",
        V,
        "<nom>.gif",
    ),
    ready(
        ff(
            "video-extraire-image",
            "Extraire une image",
            "Capture l'image à un instant donné.",
            V,
            "<nom>-<instant>.png",
        ),
        &[text("instant", "Instant (ex. 1:30 ou 90)", "0")],
    ),
    ToolDef {
        binaries: WHISPER,
        status: Status::Phase3,
        ..ff(
            "video-sous-titres",
            "Sous-titres automatiques",
            "Transcrit la parole en sous-titres .srt avec whisper (local).",
            V,
            "<nom>.srt",
        )
    },
    ToolDef {
        status: Status::Phase3,
        ..ff(
            "video-incruster-sous-titres",
            "Incruster des sous-titres",
            "Grave un fichier .srt dans l'image.",
            V,
            "<nom>-soustitre.mp4",
        )
    },
    ff(
        "video-redimensionner",
        "Redimensionner une vidéo",
        "Change la définition (1080p, 720p, 480p…).",
        V,
        "<nom>-<hauteur>p.<ext>",
    ),
    ff(
        "video-pivoter",
        "Pivoter une vidéo",
        "Fait pivoter de 90, 180 ou 270 degrés.",
        V,
        "<nom>-pivote.<ext>",
    ),
    ready(
        ff(
            "video-supprimer-son",
            "Supprimer le son",
            "Retire toutes les pistes audio.",
            V,
            "<nom>-muet.<ext>",
        ),
        &[],
    ),
    ToolDef {
        status: Status::Phase3,
        ..ff(
            "video-accelerer",
            "Accélérer / ralentir",
            "Change la vitesse de lecture (image et son).",
            V,
            "<nom>-x<facteur>.<ext>",
        )
    },
    ToolDef {
        binaries: YTDLP,
        input: TEXT_INPUT,
        status: Status::Phase3,
        ..ff(
            "video-telecharger",
            "Télécharger une vidéo",
            "Télécharge une vidéo depuis une URL (yt-dlp), qualité au choix.",
            &[],
            "Téléchargements/<titre>.<ext>",
        )
    },
    ToolDef {
        binaries: YTDLP,
        input: TEXT_INPUT,
        status: Status::Phase3,
        ..ff(
            "audio-telecharger",
            "Télécharger l'audio",
            "Télécharge uniquement le son depuis une URL (yt-dlp).",
            &[],
            "Téléchargements/<titre>.mp3",
        )
    },
    ready(
        ff(
            "audio-convertir",
            "Convertir un audio",
            "Convertit vers MP3, M4A, OGG, Opus, WAV ou FLAC.",
            A,
            "<nom>.<format>",
        ),
        &[choice("format", "Format", AUDIO_FORMATS, "mp3")],
    ),
    ready(
        ff(
            "audio-couper",
            "Couper un audio",
            "Garde l'extrait entre un début et une fin.",
            A,
            "<nom>-extrait.<ext>",
        ),
        CUT,
    ),
    ff(
        "audio-normaliser",
        "Normaliser le volume",
        "Égalise le volume perçu (EBU R128, filtre loudnorm).",
        A,
        "<nom>-normalise.<ext>",
    ),
    ToolDef {
        binaries: WHISPER,
        status: Status::Phase3,
        ..ff(
            "audio-transcrire",
            "Transcrire un audio",
            "Transcrit la parole en texte avec whisper (local).",
            A,
            "<nom>.txt",
        )
    },
    ToolDef {
        input: files(MEDIA_EXT),
        ..ready(
            ff(
                "video-infos",
                "Informations sur un média",
                "Rapport texte : durée, conteneur, débit, pistes vidéo et audio (ffprobe).",
                V,
                "<nom>-infos.txt",
            ),
            &[],
        )
    },
];
