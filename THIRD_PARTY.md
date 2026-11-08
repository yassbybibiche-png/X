# Logiciels tiers et licences

La Boîte à outils (code de ce dépôt) appelle des programmes tiers **comme des
programmes séparés** (`std::process::Command`, arguments en ligne de commande,
échange par fichiers ; jamais de shell, aucune liaison de bibliothèque). Un seul
est **inclus dans l'installeur** : 7-Zip. Tous les autres sont **téléchargés
par l'application au premier usage**, à la demande de l'utilisateur (bouton
« Télécharger » de l'onglet Diagnostic), depuis l'adresse officielle épinglée
dans [`scripts/deps.lock.json`](scripts/deps.lock.json), avec vérification de la
taille et du SHA-256 avant activation, dans le dossier de données de l'app.

Colonne « vérifié » : d'où vient la licence indiquée ici.
- **archive** : texte de licence lu dans le fichier officiel épinglé (ou sortie
  du binaire lui-même), le 2026-11-08 ;
- **amont** : licence déclarée par le dépôt officiel du projet, **non relue
  dans l'archive** (fichier de licence absent de l'archive, ou GitHub/sites
  injoignables depuis la VM de développement au moment de la rédaction) — à
  revérifier avant chaque release.

## Programmes tiers

| programme | version épinglée | licence | vérifié | livraison | source |
|---|---|---|---|---|---|
| 7-Zip (`7zz` / `7z.exe` + `7z.dll`) | 26.06 | GNU LGPL-2.1-or-later ; une partie (décompression RAR) sous LGPL + **restriction unRAR** ; quelques parties BSD-3-Clause et BSD-2-Clause | archive (`License.txt`) | **inclus dans l'installeur** (avec `License.txt`) | <https://github.com/ip7z/7zip/releases/tag/26.06> |
| Ghostscript | 10.08.0 | **GNU AGPL-3.0** (ou licence commerciale Artifex) | archive (`COPYING` du paquet officiel) | téléchargé au premier usage (Windows) ; Linux/macOS : paquet du système | <https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/tag/gs10080> |
| Tesseract OCR (build UB Mannheim) | 5.4.0.20240606 | Apache-2.0 | archive (`doc/LICENSE` de l'installeur) | téléchargé au premier usage (Windows) ; Linux/macOS : paquet du système | <https://github.com/UB-Mannheim/tesseract/releases/tag/v5.4.0.20240606> |
| Données `fra.traineddata` (tessdata_fast) | 4.1.0 | Apache-2.0 | amont | téléchargé avec Tesseract | <https://github.com/tesseract-ocr/tessdata_fast> |
| FFmpeg / ffprobe (builds statiques ffmpeg-static) | 6.1.1 | **GNU GPL-3.0-or-later** (build `--enable-gpl --enable-version3` avec libx264/libx265 ; pas de `--enable-nonfree`) | archive (`ffmpeg -L`, `-buildconf`) | téléchargé au premier usage | <https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.1.1> |
| ImageMagick | 7.1.2-35 | ImageMagick License (dérivée d'Apache-2.0) | archive (`LICENSE` de l'AppImage) | téléchargé au premier usage (Linux, Windows) ; macOS : Homebrew | <https://github.com/ImageMagick/ImageMagick/releases/tag/7.1.2-35> |
| pandoc | 3.13 | **GNU GPL-2.0-or-later** (quelques fichiers sous d'autres licences libres, voir `COPYRIGHT.txt`) | archive (`COPYRIGHT.txt`) | téléchargé au premier usage | <https://github.com/jgm/pandoc/releases/tag/3.13> |
| typst | 0.15.1 | Apache-2.0 | archive (`LICENSE`) | téléchargé au premier usage | <https://github.com/typst/typst/releases/tag/v0.15.1> |
| qpdf | 12.4.2 | Apache-2.0 | archive (manuel, section *License*) | téléchargé au premier usage | <https://github.com/qpdf/qpdf/releases/tag/v12.4.2> |
| yt-dlp | 2026.10.26 | Unlicense (domaine public) | amont | téléchargé au premier usage, **mise à jour depuis l'app** | <https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26> |
| whisper.cpp (`whisper-cli`) | 1.9.5 (b5454) | MIT | archive (`LICENSE`) | téléchargé au premier usage (Linux, Windows) ; macOS : Homebrew | <https://github.com/ggml-org/whisper.cpp/releases/tag/b5454> |
| Modèles whisper `ggml-small-q5_1` / `ggml-base-q5_1` | — | MIT (poids Whisper d'OpenAI, MIT ; conversion ggml du dépôt whisper.cpp, MIT) | amont | téléchargés au premier usage (SHA-256 pas encore épinglé, voir PLAN §3.5) | <https://huggingface.co/ggerganov/whisper.cpp> |
| Real-ESRGAN ncnn Vulkan + modèles `realesrgan-x4plus` | 0.2.5.0 (ncnn-vulkan 20220424) | Real-ESRGAN : BSD-3-Clause ; Real-ESRGAN-ncnn-vulkan : MIT ; ncnn : BSD-3-Clause | amont (le zip officiel ne contient **aucun** fichier de licence) | téléchargé au premier usage | <https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.2.5.0> |

### 7-Zip (inclus) — obligations
- LGPL-2.1 : le binaire est redistribué **non modifié** ; le code source
  correspondant est celui publié par l'auteur à l'adresse ci-dessus (lien
  repris ici et dans `License.txt`). La licence exige de reproduire les
  informations de licence avec le binaire : `License.txt` est copié dans
  l'installeur (`licenses/7zip-License.txt`, voir `release.yml`).
- **Restriction unRAR** : le code de décompression RAR ne peut pas être utilisé
  pour recréer l'algorithme de compression RAR. La Boîte à outils ne fait
  qu'**extraire** des RAR (jamais de création de RAR).

### Ghostscript (AGPL-3.0) — obligations et décision
**Décision : Ghostscript n'est jamais inclus dans l'installeur de la Boîte à
outils.** Il est téléchargé au premier usage, sur action explicite de
l'utilisateur, directement depuis les fichiers officiels d'Artifex
(installeur Windows `gs10080w64.exe` de la release `ghostpdl-downloads`
gs10080, SHA-256 épinglé), puis extrait sans installation ni droits
administrateur avec le 7-Zip inclus. Sous Linux et macOS, Artifex ne publie
pas de binaire portable utilisable : l'app indique d'installer le paquet du
système (`apt install ghostscript`, `brew install ghostscript`) et le trouve
ensuite dans le PATH.

Ce que l'AGPL-3.0 imposerait **si nous redistribuions** Ghostscript (binaire
dans l'installeur ou dans une release de ce dépôt) :
1. fournir le texte de l'AGPL-3.0 et conserver les mentions de copyright ;
2. fournir le **code source correspondant** exact (§6) : au même endroit que le
   binaire (§6(d)), ou une offre écrite valable au moins 3 ans (§6(b)) ;
3. publier sous AGPL toute **modification** de Ghostscript ; et, pour une
   version modifiée accessible par le réseau, offrir son source aux
   utilisateurs distants (§13).

Conséquences de la décision :
- **Pour la redistribution de l'app** : l'installeur ne contient aucun code
  AGPL, donc aucune des obligations ci-dessus ne s'applique à sa diffusion.
  Notre code appelle `gs` comme programme séparé (arguments + fichiers) : selon
  la FAQ GPL de la FSF, c'est une « agrégation », pas une œuvre dérivée ; la
  Boîte à outils n'a pas à être sous AGPL. Ghostscript n'est pas modifié.
- **Pour l'utilisateur** : il reçoit Ghostscript directement d'Artifex, sous
  AGPL-3.0, avec ses droits (utiliser, étudier, modifier, redistribuer sous
  AGPL) ; le code source est disponible chez Artifex
  (<https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/tag/gs10080>,
  <https://ghostscript.com/releases/gpldnld.html>). Usage purement local : la
  clause réseau §13 ne le concerne pas. Avant le téléchargement, le Diagnostic
  affiche le nom et la licence du binaire.
- **Si un jour** un binaire Ghostscript (ex. macOS) est construit par notre CI
  et publié dans une release `deps-*` de ce dépôt (option envisagée au PLAN
  §3.3.2, **non mise en œuvre**), il faudra y joindre l'archive source exacte,
  le texte de l'AGPL et ce paragraphe.
- Les opérations PDF structurelles (fusion, découpe, rotation, extraction,
  filigrane, numérotation, métadonnées, protection) n'utilisent **pas**
  Ghostscript (lopdf / qpdf) : elles marchent sans lui.

### Autres licences copyleft (GPL)
pandoc (GPL-2.0+) et les builds FFmpeg utilisés (GPL-3.0+) ne sont **pas
redistribués** par ce projet : l'app les télécharge depuis leur source
officielle, comme programmes séparés. Si on décidait de les inclure dans
l'installeur, il faudrait joindre les textes de licence et l'accès au code
source correspondant (archives sources exactes dans la même release).

## Bibliothèques Rust compilées dans l'application

Calculé avec `cargo metadata` sur `src-tauri/Cargo.lock` (516 paquets, toutes
plateformes confondues) : licences permissives — majoritairement
`MIT OR Apache-2.0`, `MIT`, `Apache-2.0`, plus `Unicode-3.0`, `Zlib`, `ISC`,
`BSD-2/3-Clause`, `CC0-1.0`, `Unlicense OR MIT`, `CDLA-Permissive-2.0`
(certificats racines Mozilla de `webpki-roots`) et `MPL-2.0` (5 paquets des
composants CSS de Tauri : `cssparser`, `selectors`… ; MPL = copyleft par
fichier, satisfait en redistribuant ces fichiers non modifiés dont le source
est public sur crates.io). Les deux paquets marqués
`MIT OR Apache-2.0 OR LGPL-2.1-or-later` sont utilisés sous MIT/Apache-2.0.
Principales : Tauri 2 (Apache-2.0 OR MIT), lopdf (MIT), image (MIT OR
Apache-2.0), zip (MIT), tar, flate2, ureq + rustls (MIT OR Apache-2.0),
lzma-rs (MIT), qrcode (MIT OR Apache-2.0), sha2 (MIT OR Apache-2.0).

Pour régénérer la liste complète :
`cd src-tauri && cargo metadata --format-version 1 | jq -r '.packages[] | "\(.name) \(.version) \(.license)"'`.

Le décodage xz + filtre BCJ x86 (`src-tauri/src/xz.rs`) est une réécriture en
Rust de l'algorithme de xz-embedded (domaine public).
