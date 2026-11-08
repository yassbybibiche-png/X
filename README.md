# Boîte à outils

Une application de bureau gratuite (Windows, macOS, Linux) qui regroupe
**90 petits outils** pour vos fichiers : PDF, documents, images, vidéo et son,
archives, et quelques utilitaires. Tout est en français, tout se fait par
glisser-déposer, et **tout se passe sur votre ordinateur** : vos fichiers ne
sont jamais envoyés sur Internet.

> **Version 0.1.0 — première version.** Elle a été développée et testée sous
> Linux ; les versions Windows et macOS sont construites automatiquement mais
> n'ont pas encore été essayées sur une vraie machine. Les limites connues sont
> listées plus bas, sans rien cacher.

## Ce que fait l'application

| catégorie | exemples d'outils |
|---|---|
| **PDF** (18) | fusionner, découper, extraire ou supprimer des pages, pivoter, compresser, OCR (rendre un scan cherchable), PDF → images, images → PDF, protéger / retirer un mot de passe, filigrane, numéroter les pages, réparer, retirer les métadonnées |
| **Documents** (15) | Markdown, Word (.docx), OpenDocument (.odt), HTML, EPUB, PDF : conversions dans les deux sens, extraction du texte |
| **Images** (18) | convertir (PNG, JPEG, WebP, AVIF, HEIC…), redimensionner, compresser, supprimer les données EXIF (position GPS…), pivoter, rogner, détourer, filigrane, icône .ico, GIF animé, planche contact, lire le texte d'une image (OCR), agrandir ×4 par IA |
| **Vidéo et audio** (22) | convertir, compresser, couper, extraire le son ou une image, GIF, redimensionner, accélérer, assembler, sous-titres automatiques (transcription), incruster ou extraire des sous-titres, normaliser le volume, télécharger une vidéo ou un son depuis un lien |
| **Archives** (10) | créer et extraire ZIP, 7z, TAR, TAR.GZ ; extraire RAR, ISO… ; lister, tester, ajouter des fichiers |
| **Utilitaires** (7) | empreinte SHA-256 et vérification, renommer en lot, trouver les doublons, découper / réassembler un gros fichier, QR code |

Principe commun : vous déposez vos fichiers, vous réglez les options, vous
cliquez sur **Lancer**. Le résultat est créé **à côté de l'original**, avec un
nom distinct (`photo-redim.jpg`, `rapport-compresse.pdf`…). **Un fichier
existant n'est jamais écrasé** : si le nom est pris, l'app ajoute `-2`, `-3`…

## Télécharger

Les installeurs sont sur la page **Releases** du dépôt GitHub du projet
(onglet « Releases », dernière version) :

| système | fichier |
|---|---|
| Windows 10 / 11 (64 bits) | `Boite-a-outils_v0.1.0_windows-x64.msi` |
| macOS, Mac Apple Silicon (M1, M2, M3, M4…) | `Boite-a-outils_v0.1.0_macos-arm64.dmg` |
| macOS, Mac Intel | `Boite-a-outils_v0.1.0_macos-x64.dmg` |
| Linux (64 bits) | `Boite-a-outils_v0.1.0_linux-x64.AppImage` |

Sur Mac, pour savoir quel fichier prendre : menu  › *À propos de ce Mac* :
« Puce Apple » → arm64 ; « Processeur Intel » → x64.

## Installer et premier lancement

### Windows
Double-cliquez sur le `.msi`. L'application n'étant pas signée par un
certificat payant, Windows SmartScreen peut afficher « Windows a protégé votre
ordinateur » : cliquez sur **Informations complémentaires** puis
**Exécuter quand même**.

### macOS (dont macOS 15 Sequoia)
1. Ouvrez le `.dmg` et glissez **Boite a outils** dans **Applications**.
2. Lancez l'app une première fois : macOS refuse de l'ouvrir (« Apple n'a pas
   pu confirmer… ») car elle n'est pas signée par un compte développeur Apple.
   Cliquez sur **OK** (pas sur « Placer dans la corbeille »).
3. Ouvrez **Réglages Système › Confidentialité et sécurité**, descendez
   jusqu'à la section *Sécurité* : un message indique que « Boite a outils » a
   été bloquée. Cliquez sur **Ouvrir quand même**, confirmez avec votre mot de
   passe, puis **Ouvrir**.
4. Les lancements suivants se font normalement.

> ⚠️ **Le clic droit › « Ouvrir » ne marche plus depuis macOS 15** : passez par
> les Réglages Système comme ci-dessus.

Solution de secours (si le bouton « Ouvrir quand même » n'apparaît pas), dans
l'app **Terminal** :

```sh
xattr -cr "/Applications/Boite a outils.app"
```

Cette commande retire la marque « téléchargé depuis Internet » de l'app ;
ne l'utilisez que pour une app obtenue depuis la page Releases officielle.

### Linux
Rendez le fichier exécutable puis lancez-le :

```sh
chmod +x Boite-a-outils_v0.1.0_linux-x64.AppImage
./Boite-a-outils_v0.1.0_linux-x64.AppImage
```

(Ou clic droit › Propriétés › « Autoriser l'exécution » dans votre gestionnaire
de fichiers.) Si l'AppImage refuse de démarrer faute de FUSE, installez
`libfuse2` (Ubuntu : `sudo apt install libfuse2t64`) ou lancez-la avec
`--appimage-extract-and-run`.

### Les outils complémentaires (premier usage)
Pour rester léger, l'installeur ne contient que l'application et 7-Zip. Les
autres programmes libres sur lesquels s'appuient certains outils (ffmpeg pour
la vidéo, pandoc pour les documents, ImageMagick, Ghostscript, Tesseract pour
l'OCR, yt-dlp, whisper pour la transcription, Real-ESRGAN pour l'agrandissement
IA, qpdf, typst) sont **téléchargés au premier usage, quand vous le décidez** :

1. un outil qui a besoin d'un programme absent apparaît grisé, avec la raison ;
2. ouvrez l'onglet **Diagnostic** : chaque programme y est listé (présent ou
   non, version, emplacement, licence) avec un bouton **Télécharger** (taille
   indiquée, barre de progression, bouton Annuler) ;
3. chaque fichier téléchargé est **vérifié** (taille et empreinte SHA-256
   fixées à l'avance) avant d'être utilisé, puis rangé dans le dossier de
   données de l'application.

Selon votre système, quelques programmes n'ont pas de version officielle
téléchargeable ; le Diagnostic vous indique alors quoi installer :

| programme | Windows | macOS | Linux |
|---|---|---|---|
| Ghostscript (compresser / OCR / rastériser un PDF) | bouton Télécharger | `brew install ghostscript` | `sudo apt install ghostscript` |
| Tesseract (OCR) | bouton Télécharger | `brew install tesseract tesseract-lang` | `sudo apt install tesseract-ocr tesseract-ocr-fra` |
| ImageMagick (détourer, filigrane, planche, AVIF/HEIC) | bouton Télécharger | `brew install imagemagick` | bouton Télécharger |
| whisper (transcription) | bouton Télécharger | `brew install whisper-cpp` | bouton Télécharger |

(`brew` = [Homebrew](https://brew.sh).) Les modèles de transcription whisper ne
sont **pas encore téléchargeables depuis l'app** dans cette version (voir
*Limites*).

## Mettre à jour yt-dlp
Les sites vidéo changent souvent, et yt-dlp doit suivre. Dans l'onglet
**Diagnostic**, la ligne yt-dlp affiche la version installée et un bouton
**Vérifier la mise à jour** ; si une version plus récente est connue de l'app,
le bouton **Mettre à jour vers …** la télécharge (vérifiée par SHA-256) dans le
dossier de données de l'app, à la place de l'ancienne. La version proposée est
celle épinglée par votre version de l'app : si un téléchargement vidéo ne
marche plus, installez aussi la dernière version de la Boîte à outils.

## Vie privée
- **100 % local** : aucun fichier ne quitte votre ordinateur, aucun compte,
  aucune publicité, aucune statistique d'usage.
- L'application n'accède à Internet que dans trois cas, **toujours à votre
  demande** :
  1. **Télécharger une vidéo ou un son** (yt-dlp) : l'adresse que vous collez
     est contactée, comme le ferait votre navigateur ;
  2. **Télécharger un programme complémentaire** (bouton du Diagnostic) depuis
     son site officiel (GitHub, Artifex…) ;
  3. **Télécharger un modèle d'IA** (transcription, agrandissement).
- Le QR code est généré localement : le lien encodé n'est jamais ouvert.
- Conversions de documents : pandoc tourne toujours en mode **sandbox** ; les
  images distantes (http/https) d'un document sont **retirées** et signalées
  par un bandeau d'avertissement, pour qu'un document piégé ne puisse ni lire
  vos fichiers privés ni contacter un serveur.
- Mots de passe (PDF, 7z) : jamais écrits sur la ligne de commande (invisibles
  pour les autres programmes).

## Captures d'écran

> **Capture à ajouter.** L'application a été développée sur une machine sans
> écran : aucune capture réelle n'existe encore, et aucune image n'est fournie
> à la place. Emplacements prévus :
> - `docs/captures/outils.png` — *capture à ajouter* : écran principal
>   (catégories, liste des outils, zone de dépôt) ;
> - `docs/captures/options.png` — *capture à ajouter* : options d'un outil et
>   barre de progression ;
> - `docs/captures/diagnostic.png` — *capture à ajouter* : onglet Diagnostic
>   avec les boutons Télécharger.

## Limites connues (v0.1.0)
- Testée uniquement sous **Linux** ; Windows et macOS n'ont pas encore été
  essayés sur une vraie machine.
- **Transcription / sous-titres automatiques** : les modèles whisper ne sont
  pas encore téléchargeables depuis l'app. Vous pouvez déposer vous-même
  `ggml-small-q5_1.bin` (ou `ggml-base-q5_1.bin`), obtenu sur la page
  Hugging Face officielle de whisper.cpp, dans le dossier
  `models/whisper-small/` (ou `models/whisper-base/`) du dossier de données
  de l'app.
- **Agrandissement IA** : nécessite une carte graphique compatible Vulkan ; sans
  elle, l'image est agrandie de façon classique (Lanczos) et l'app le signale
  (`-x4-lanczos.png`).
- Applications non signées (avertissements Windows / macOS ci-dessus).
- Détail complet : [PLAN.md](PLAN.md) §5 et [AUDIT.md](AUDIT.md).

Dossier de données de l'app (programmes téléchargés, modèles) :
Windows `%APPDATA%\fr.boiteaoutils.app`, macOS
`~/Library/Application Support/fr.boiteaoutils.app`, Linux
`~/.local/share/fr.boiteaoutils.app`.

## Licences
Code de l'application : MIT. Les programmes complémentaires gardent leur propre
licence (Ghostscript AGPL-3.0, pandoc GPL-2.0+, ffmpeg GPL-3.0+, 7-Zip LGPL…) :
voir [THIRD_PARTY.md](THIRD_PARTY.md), qui explique aussi pourquoi Ghostscript
n'est jamais inclus dans l'installeur.

## Développement

Prérequis : Rust stable (≥ 1.90), Node.js (CLI Tauri), et sous Linux
`libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev`.

```sh
npm install                      # CLI Tauri (dev uniquement)
npx tauri dev                    # lance l'app
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run --example diagnostic   # état des binaires tiers, sans interface
```

CI : `.github/workflows/` — `ci.yml` (chaque push), `deps.yml` (binaires tiers
en cache), `release.yml` (tag `v*` → 4 installeurs), `smoke.yml` (test lourd,
manuel). Publier une version : `git tag v0.1.0 && git push origin v0.1.0`.
