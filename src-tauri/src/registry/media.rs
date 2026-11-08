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

const fn num(
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
const SUB_FORMATS: &[Choice] = &[
    c("srt", "SRT"),
    c("vtt", "WebVTT"),
    c("srt+vtt", "SRT + WebVTT"),
];
const LANGS: &[Choice] = &[
    c("auto", "Détection automatique"),
    c("fr", "Français"),
    c("en", "Anglais"),
];
const WHISPER_MODELS: &[Choice] = &[
    c("small", "small (précis, ≈ 190 Mo)"),
    c("base", "base (rapide, ≈ 60 Mo)"),
];
const HEIGHTS: &[Choice] = &[
    c("2160", "2160p (4K)"),
    c("1080", "1080p"),
    c("720", "720p"),
    c("480", "480p"),
    c("360", "360p"),
];
const ANGLES: &[Choice] = &[
    c("90", "90° (horaire)"),
    c("180", "180°"),
    c("270", "270° (anti-horaire)"),
];
const SPEEDS: &[Choice] = &[
    c("0.25", "×0,25"),
    c("0.5", "×0,5"),
    c("0.75", "×0,75"),
    c("1.25", "×1,25"),
    c("1.5", "×1,5"),
    c("2", "×2"),
    c("3", "×3"),
    c("4", "×4"),
];
const QUALITIES: &[Choice] = &[
    c("meilleure", "La meilleure disponible"),
    c("1080", "1080p max."),
    c("720", "720p max."),
    c("480", "480p max."),
    c("360", "360p max."),
];
const DL_AUDIO: &[Choice] = &[c("mp3", "MP3"), c("m4a", "M4A (AAC)"), c("opus", "Opus")];
const GRIDS: &[Choice] = &[c("3x2", "3 × 2"), c("4x3", "4 × 3"), c("5x4", "5 × 4")];
const PLAYLIST: OptionDef = OptionDef {
    key: "playlist",
    label: "Toute la playlist (sinon la vidéo seule)",
    kind: OptionKind::Bool,
    default: "false",
};
const DEST: OptionDef = text("dossier", "Dossier (vide = Téléchargements)", "");
const V_AND_SUBS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "mpg", "mpeg", "srt", "vtt", "ass",
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
    ready(
        ff(
            "video-vers-gif",
            "Vidéo vers GIF",
            "Crée un GIF animé (palette optimisée) d'un extrait.",
            V,
            "<nom>.gif",
        ),
        &[
            text("debut", "Début (ex. 1:30 ou 90)", "0"),
            text("fin", "Fin (vide = jusqu'au bout)", "10"),
            num("fps", "Images par seconde", (1.0, 30.0), "10"),
            num("largeur", "Largeur (px)", (32.0, 1920.0), "480"),
        ],
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
        input: files(MEDIA_EXT),
        ..ready(
            ff(
                "video-sous-titres",
                "Sous-titres automatiques",
                "Transcrit la parole en sous-titres SRT et/ou WebVTT avec whisper (local, rien ne quitte la machine).",
                V,
                "<nom>.srt / <nom>.vtt",
            ),
            &[
                choice("format", "Format", SUB_FORMATS, "srt"),
                choice("langue", "Langue parlée", LANGS, "auto"),
                choice("modele", "Modèle", WHISPER_MODELS, "small"),
            ],
        )
    },
    ToolDef {
        input: super::InputSpec {
            min_files: 2,
            ..files(V_AND_SUBS)
        },
        ..ready(
            ff(
                "video-incruster-sous-titres",
                "Incruster des sous-titres",
                "Grave un fichier .srt/.vtt/.ass dans l'image (choisissez la vidéo ET le fichier de sous-titres).",
                V,
                "<nom>-soustitre.mp4",
            ),
            &[],
        )
    },
    ready(
        ff(
            "video-redimensionner",
            "Redimensionner une vidéo",
            "Change la définition (1080p, 720p, 480p…), sans jamais agrandir.",
            V,
            "<nom>-<hauteur>p.<ext>",
        ),
        &[choice("hauteur", "Hauteur", HEIGHTS, "720")],
    ),
    ready(
        ff(
            "video-pivoter",
            "Pivoter une vidéo",
            "Fait pivoter de 90, 180 ou 270 degrés.",
            V,
            "<nom>-pivote.<ext>",
        ),
        &[choice("angle", "Angle", ANGLES, "90")],
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
    ready(
        ff(
            "video-accelerer",
            "Accélérer / ralentir",
            "Change la vitesse de lecture (image et son).",
            V,
            "<nom>-x<facteur>.<ext>",
        ),
        &[choice("facteur", "Vitesse", SPEEDS, "2")],
    ),
    ToolDef {
        binaries: YTDLP,
        input: TEXT_INPUT,
        ..ready(
            ff(
                "video-telecharger",
                "Télécharger une vidéo",
                "Télécharge une vidéo depuis une URL (yt-dlp), qualité au choix. Seul outil qui utilise Internet.",
                &[],
                "Téléchargements/<titre> [<id>].<ext>",
            ),
            &[
                text("url", "Adresse (https://…)", ""),
                choice("qualite", "Qualité", QUALITIES, "1080"),
                PLAYLIST,
                DEST,
            ],
        )
    },
    ToolDef {
        binaries: YTDLP,
        input: TEXT_INPUT,
        ..ready(
            ff(
                "audio-telecharger",
                "Télécharger l'audio",
                "Télécharge uniquement le son depuis une URL (yt-dlp). Seul outil qui utilise Internet.",
                &[],
                "Téléchargements/<titre> [<id>].mp3",
            ),
            &[
                text("url", "Adresse (https://…)", ""),
                choice("format", "Format", DL_AUDIO, "mp3"),
                PLAYLIST,
                DEST,
            ],
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
    ready(
        ff(
            "audio-normaliser",
            "Normaliser le volume",
            "Égalise le volume perçu (EBU R128, −16 LUFS, filtre loudnorm).",
            A,
            "<nom>-normalise.<ext>",
        ),
        &[],
    ),
    ToolDef {
        binaries: WHISPER,
        input: files(MEDIA_EXT),
        ..ready(
            ff(
                "audio-transcrire",
                "Transcrire un audio",
                "Transcrit la parole en texte avec whisper (local).",
                A,
                "<nom>.txt",
            ),
            &[
                choice("langue", "Langue parlée", LANGS, "auto"),
                choice("modele", "Modèle", WHISPER_MODELS, "small"),
            ],
        )
    },
    ToolDef {
        input: super::InputSpec {
            min_files: 2,
            ..files(V)
        },
        ..ready(
            ff(
                "video-concatener",
                "Assembler des vidéos",
                "Met des vidéos bout à bout, dans l'ordre (taille de la première, réencodage H.264/AAC).",
                V,
                "<premiere>-assemble.mp4",
            ),
            &[],
        )
    },
    ready(
        ff(
            "video-vignettes",
            "Planche de vignettes",
            "Image regroupant des captures réparties sur toute la durée de la vidéo.",
            V,
            "<nom>-vignettes.jpg",
        ),
        &[choice("grille", "Grille", GRIDS, "4x3")],
    ),
    ready(
        ff(
            "video-extraire-sous-titres",
            "Extraire les sous-titres",
            "Enregistre les pistes de sous-titres texte contenues dans la vidéo (.srt, .vtt, .ass).",
            V,
            "<nom>-st<N>[-langue].srt",
        ),
        &[],
    ),
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
