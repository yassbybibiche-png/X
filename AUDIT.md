# Audit externe — Boîte à outils v0.1.0 (2026-11-08)

Relecture « auditeur qui ne connaît pas le projet », sur le dépôt tel qu'il
sera publié. Périmètre : binaires manquants sur machine vierge, chemins
Windows, CI, sécurité (shell, pandoc, exfiltration, écrasement), secrets.
Environnement : VM Linux x64 sans écran ; **Windows et macOS n'ont pas pu être
exécutés**, et **GitHub Actions n'a pas pu être lancé** depuis la VM.
Chaque correctif a son propre commit (voir `git log`).

## 1. Corrections apportées

| # | domaine | trouvaille | correction | commit |
|---|---|---|---|---|
| 1 | machine vierge (Linux) | L'ImageMagick Linux du manifeste est une **AppImage** : elle échouait sans FUSE (« No suitable fusermount binary found »), cas par défaut d'Ubuntu 24.04. Invisible jusque-là : les tests utilisaient le `convert` du système. | `APPIMAGE_EXTRACT_AND_RUN=1` passé à chaque sidecar. Tests réels ImageMagick désormais faits avec l'AppImage installée par l'app ; preuve : sans la variable, 2 tests échouent. | `47d2560` |
| 2 | machine vierge (macOS) | Une app lancée depuis le Finder reçoit un PATH minimal (`/usr/bin:/bin:/usr/sbin:/sbin`) : Ghostscript, Tesseract, ImageMagick, whisper installés par **Homebrew** (ce que l'app conseille) n'auraient **jamais** été trouvés. | `known_install_dirs` : `/opt/homebrew/bin`, `/usr/local/bin`, `/opt/local/bin` ajoutés après le PATH. | `fe7b1b0` |
| 3 | chemins Windows | Les installeurs officiels de Ghostscript (`Program Files\gs\gs10.xx\bin\gswin64c.exe`), Tesseract (`Program Files\Tesseract-OCR`), 7-Zip, ImageMagick (`ImageMagick-*`), pandoc (`%LOCALAPPDATA%\Pandoc`) n'ajoutent pas toujours leur dossier au PATH → programme déjà installé non détecté. | Ces dossiers sont parcourus après le PATH (version `gs*` la plus récente d'abord) ; testé avec une arborescence factice. | `fe7b1b0` |
| 4 | machine vierge (UI) | Le panneau d'un outil indisponible affichait seulement « Binaire manquant : X », sans téléchargement ni explication. | Bouton « Télécharger X <version> (<taille>) » dans le panneau ou, si X n'est pas téléchargeable sur cet OS, la raison du manifeste (paquet / Homebrew à installer). | `2019223` |
| 5 | exfiltration (défense en profondeur) | ffmpeg/ffprobe peuvent ouvrir des URL (HLS, concat…) si un fichier déposé les y invite. | `-protocol_whitelist file,pipe` ajouté avant chaque `-i` (et en tête pour ffprobe) dans `sidecar::run_with`, donc pour tous les outils ; test réel : `http://` refusé. Aucune exfiltration n'a été **reproduite** sans ce garde-fou (ffmpeg 6.1.1 et 8.1.2 refusent déjà une playlist déguisée en `.mp4`/`.mkv`/`.srt`). | `3a7551d` |
| 6 | CI | `release.yml` interpolait `${{ github.ref_name }}` dans des scripts shell (motif d'injection). | Passage par variables d'environnement. | `8068db7` |
| 7 | chemins Windows | `renommer-lot` acceptait `CON`, `NUL`, `COM1`… (périphériques Windows : aucun fichier créé) et les points finaux. | Noms réservés refusés, points finaux retirés, caractères de contrôle refusés. | `4f9934a` |
| 8 | licences | La licence de 7-Zip exige de reproduire ses informations de licence avec le binaire ; l'installeur ne les contenait pas. | `License.txt` copié dans l'installeur (`licenses/7zip-License.txt`). | `fcf9cd7` |
| 9 | correction | `reassembler` ne retrouvait pas une série renommée pour ne pas écraser (`film.mkv-2.001`) : `Path::with_extension` prenait `mkv-2` pour l'extension. | Noms construits par chaînes, `original_name` ; testé. | `bf7c089` |
| 10 | doc | PLAN §3.8 : `xattr -cr "/Applications/Boîte à outils.app"` — mauvais nom (le bundle s'appelle `Boite a outils.app`). | Corrigé (README correct). | `9b075c2` |

## 2. Vérifications sans anomalie

- **Jamais de shell** : `git grep -E "sh -c|cmd /C|Command::new\(\"sh\"|powershell"`
  → une seule occurrence, la phrase de PLAN.md qui l'interdit. Un seul
  `Command::new` dans le code (`sidecar::spawn`), arguments passés un par un.
- **pandoc `--sandbox` partout, Markdown compris** : un seul constructeur
  d'arguments (`documents::pandoc_args`, `--sandbox` toujours en premier, test
  unitaire), utilisé par les deux seuls points d'appel de pandoc (conversions
  directes et étape `.typ` des PDF) ; un test lit les arguments réellement reçus
  par un faux pandoc pour les 13 conversions directes (md → docx/odt/html/epub
  compris). Images distantes retirées
  + avertissement, images hors du dossier refusées (test).
- **Sorties jamais écrasantes** : il n'existe pas de fonction `reserve_output` ;
  l'API est `output::reserve` / `beside` / `reserve_dir` / `reserve_exact`, toutes
  en `create_new` / `create_dir` atomiques. Les autres écritures
  (`fs::write`, `File::create`) visent uniquement des dossiers temporaires privés
  ou le dossier de données de l'app (`download.rs`). Les sidecars écrivent dans
  la réservation vide ou dans un dossier de travail puis `output::fill_from`.
  Tests de non-écrasement dans chaque famille d'outils.
- **Réseau** : un seul client HTTP (`download.rs`, HTTPS obligatoire, URL +
  taille + SHA-256 du manifeste compilé) ; yt-dlp seulement sur URL saisie par
  l'utilisateur (`--ignore-config`, URL après `--`). Le webview a une CSP
  `default-src 'self'` sans `connect-src` externe.
- **Mots de passe** : qpdf via fichier d'arguments privé (`@fichier`), 7-Zip via
  l'entrée standard — jamais sur la ligne de commande.
- **Secrets / données perso** : `git grep` d'adresses e-mail, de `/home/`,
  `/Users/`, `C:\Users`, du nom d'utilisateur de la VM et de motifs
  `token|secret|password = "…"` → rien (seul faux positif : `icons/128x128@2x.png`).
  `.scratch/` (téléchargements de vérification) est ignoré par Git.
- **CI** : cache cargo (`Swatinem/rust-cache`) dans tous les jobs ; binaires
  tiers seulement dans `deps.yml` (cache clé = plateforme +
  `hashFiles('scripts/deps.lock.json')`), restaurés en lecture seule par
  `release.yml` / `smoke.yml` ; smoke test lourd seulement en manuel ou sur tag
  `v*-rc*` ; aucun secret autre que `GITHUB_TOKEN` ; permissions minimales
  (`contents: read`, `write` seulement pour la release). actionlint 1.7.12 : 0
  trouvaille.
- **Archives** : extraction anti « zip slip », liens symboliques non suivis /
  supprimés, dossier de sortie neuf (tests existants).

## 3. Risques résiduels (non corrigés)

| gravité | risque | détail / piste |
|---|---|---|
| haute | **Windows et macOS jamais exécutés** | Aucune exécution réelle de l'app, des chemins `.exe` ni de l'extraction des installeurs Windows (Ghostscript, Tesseract, 7-Zip). Seule la compilation croisée sera vérifiée par `ci.yml` (`cargo check`). À faire : lancer `smoke.yml` manuellement avant de publier, puis un essai humain par OS. |
| haute | **CI jamais lancée** | Les 4 workflows sont validés syntaxiquement (PyYAML + actionlint) mais n'ont jamais tourné. Points incertains : label `macos-15-intel` (runner Intel GitHub, à confirmer), placement de `7z.dll` à côté de l'exécutable via `bundle.resources` dans le MSI (si ce n'est pas le cas, 7-Zip inclus ne démarre pas sous Windows → le Diagnostic le montrera absent ; repli : 7-Zip déjà installé dans `Program Files`). |
| moyenne | Transcription inutilisable sans dépôt manuel du modèle | SHA-256 des modèles whisper non épinglés (Hugging Face injoignable) : `video-sous-titres` et `audio-transcrire` affichent « modèle absent ». Ajouter `files` aux entrées `models` du manifeste. |
| moyenne | yt-dlp vieillit avec l'app | « Mettre à jour » installe la version **épinglée dans le manifeste compilé** ; quand les sites changent, il faut une nouvelle version de l'app (ou une mise à jour vers la dernière release amont, vérifiée par le `SHA2-256SUMS` publié par yt-dlp — non implémenté). Aucun téléchargement réel n'a été testé. |
| moyenne | Ghostscript / Tesseract non téléchargeables sous Linux et macOS ; ImageMagick et whisper sous macOS | Pas de binaire portable officiel : l'utilisateur doit passer par son gestionnaire de paquets (indiqué par l'app). Les builds `deps.yml` prévus au PLAN §3.3.2 ne sont pas faits. |
| moyenne | Installeurs non signés | SmartScreen / Gatekeeper avertissent (contournements dans le README). Aucun certificat disponible. |
| basse | ImageMagick 6 du système et SVG | IM 6 reconnaît un SVG à son contenu même nommé `.png` et le passe à `rsvg-convert` ; librsvg refuse les URL distantes et l'IM 7.1.2-35 du manifeste bloque déjà (« explicit coder not allowed »). Piste : `policy.xml` propre à l'app via `MAGICK_CONFIGURE_PATH`. |
| basse | typst et les paquets `@preview` | typst télécharge un paquet si le `.typ` contient `#import "@preview/…"`. Le `.typ` est produit par pandoc, qui échappe `#` et n'a pas de blocs typst bruts (désactivés) : non exploitable en l'état connu. Piste : `TYPST_PACKAGE_PATH` vers un dossier vide + mode hors ligne. |
| basse | Real-ESRGAN : voie IA jamais exécutée | Pas de GPU Vulkan sur la VM ; repli Lanczos testé et signalé. |
| basse | Licences reprises de l'amont | tessdata, yt-dlp, Real-ESRGAN (zip sans fichier de licence), modèles whisper : à relire avant release (THIRD_PARTY.md). |
| basse | AppImage de l'app elle-même | Comme toute AppImage, la nôtre a besoin de FUSE (`libfuse2`) ou de `--appimage-extract-and-run` ; indiqué dans le README. |
| basse | Frontend jamais affiché | VM sans écran : `node --check` seulement, pas de captures d'écran. |
