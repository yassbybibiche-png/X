use super::{AUDIO_EXT, BASE, Category, Status, TEXT_INPUT, ToolDef, VIDEO_EXT, files};
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

const V: &[&str] = VIDEO_EXT;
const A: &[&str] = AUDIO_EXT;

pub const TOOLS: &[ToolDef] = &[
    ff(
        "video-convertir",
        "Convertir une vidéo",
        "Convertit vers MP4, MKV, WebM ou MOV.",
        V,
        "<nom>.<format>",
    ),
    ff(
        "video-compresser",
        "Compresser une vidéo",
        "Réduit le poids (H.264, qualité CRF réglable).",
        V,
        "<nom>-compresse.mp4",
    ),
    ff(
        "video-couper",
        "Couper une vidéo",
        "Garde l'extrait entre un début et une fin.",
        V,
        "<nom>-extrait.<ext>",
    ),
    ff(
        "video-extraire-audio",
        "Extraire l'audio",
        "Récupère la piste son en MP3, M4A, WAV ou FLAC.",
        V,
        "<nom>.<format>",
    ),
    ff(
        "video-vers-gif",
        "Vidéo vers GIF",
        "Crée un GIF animé (palette optimisée) d'un extrait.",
        V,
        "<nom>.gif",
    ),
    ff(
        "video-extraire-image",
        "Extraire une image",
        "Capture l'image à un instant donné.",
        V,
        "<nom>-<instant>.png",
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
    ff(
        "video-supprimer-son",
        "Supprimer le son",
        "Retire toutes les pistes audio.",
        V,
        "<nom>-muet.<ext>",
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
    ff(
        "audio-convertir",
        "Convertir un audio",
        "Convertit vers MP3, M4A, OGG, Opus, WAV ou FLAC.",
        A,
        "<nom>.<format>",
    ),
    ff(
        "audio-couper",
        "Couper un audio",
        "Garde l'extrait entre un début et une fin.",
        A,
        "<nom>-extrait.<ext>",
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
];
