# Journal

## 2026-11-08 — Phase 1

### Fait
- **PLAN.md** : architecture, 80 outils (id, binaire, entrées → sortie, statut),
  décisions tranchées : bundling / premier usage par binaire, Ghostscript AGPL,
  whisper `small` quantifié (q5_1) + `base` en option légère, pas de Python
  embarqué, pandoc `--sandbox` partout + images distantes signalées, cache CI,
  smoke test lourd hors push.
- **Squelette Tauri 2** (`npx tauri init`, CLI 2.12.4) : Rust `src-tauri/`,
  frontend vanilla `src/` en français (catégories, liste générée depuis
  `list_tools`, recherche, glisser-déposer natif + sélecteur de fichiers,
  options générées, barre de progression, journal, onglet Diagnostic
  `check_tools`). CSP stricte, permissions minimales (`core:default`,
  `dialog:allow-open`).
- **Backend** : `sidecar` (bundlé → données app → PATH, `Command` direct sans
  shell, délai, capture stdout/stderr, masquage console Windows), `output`
  (à côté de l'original, suffixe, réservation `create_new`, nettoyage en cas
  d'échec), `registry` (80 outils, métadonnées FR), `jobs` (validation,
  options par défaut, binaires requis, événements `job-event`), `checksum`.
- **Outils fonctionnels (4)** : `pdf-fusionner` (lopdf), `images-convertir`
  (crate image), `archives-creer-zip` (crate zip), `utilitaires-empreinte-sha256`.
  Les 76 autres renvoient « … non implémenté en phase 1 (prévu en phase N) ».
- Exemple `cargo run --example diagnostic` (diagnostic sans GUI).

### Commandes et résultats (dans `src-tauri/`)
- `cargo fmt --check` → code de sortie 0.
- `cargo clippy --all-targets -- -D warnings` → code de sortie 0, aucun
  avertissement (`clippy::cognitive_complexity` actif, seuil 15 ; vérifié
  actif en abaissant temporairement le seuil à 4 → 9 fonctions signalées).
- `cargo test` → **55 tests passés, 0 échec** (registre = 80 outils, statuts
  conformes au PLAN, noms non écrasants, résolution sidecar, exécution sans
  shell, délai dépassé, 4 outils de bout en bout avec fixtures générées).
- `npx tauri build --debug --no-bundle` → binaire Linux debug construit.
- App non lancée graphiquement (VM sans GUI).

### Diagnostic des binaires sur cette VM
FFmpeg 8.1.2 présent (PATH système, `/usr/bin/ffmpeg`). **Absents** :
Ghostscript, Tesseract, ImageMagick, 7-Zip, pandoc, typst, yt-dlp, whisper.cpp,
Real-ESRGAN. Non bloquant : aucun outil de phase 1 n'en dépend, et les tests
n'en dépendent pas (ils utilisent des répertoires et exécutables factices).

### Notes d'environnement
- `libzstd-dev` était bloqué par un `libzstd1` patché de la VM (même version
  amont 1.5.5) : paquet `libzstd-dev` reconditionné localement avec une
  dépendance `>= 1.5.5` (et mis en `hold`) pour pouvoir installer
  `libwebkit2gtk-4.1-dev`. Concerne uniquement cette VM de dev.

### Reste à faire
- **Phase 2** : module `download` (manifeste `scripts/deps.lock.json`, SHA-256
  épinglés, progression, écriture atomique) + bouton « Télécharger » dans le
  Diagnostic ; sidecar 7-Zip bundlé (`externalBin`) ; outils pur Rust restants
  (PDF lopdf, images, archives tar/zip) ; outils ffmpeg / pandoc (sandbox +
  pré-traitement des images) / Ghostscript ; annulation des tâches ;
  mtime des fichiers dans les ZIP.
- **Phase 3** : whisper, Real-ESRGAN, Tesseract/OCR, yt-dlp + mise à jour,
  builds `deps.yml` (Ghostscript/ImageMagick/Tesseract macOS, avec sources
  attachées), CI (`ci.yml`, `deps.yml`, `release.yml`, `smoke.yml`),
  THIRD_PARTY.md, README grand public, audit externe et corrections.
- Limites connues de `pdf-fusionner` : signets, formulaires et étiquettes de
  page des sources non repris.

## 2026-11-08 — Phase 2 : reprise après interruption

La session précédente a été tuée (redémarrage de la VM) sans commit. État trouvé
à la reprise (`git status`, `git diff --stat`, lecture de chaque fichier) :

### Déjà fait (non commité, récupéré tel quel puis complété)
- `scripts/deps.lock.json` (schéma 1) : URL + SHA-256 + taille épinglés par
  plateforme pour 7-Zip 26.06, Ghostscript 10.08.0 (Windows seulement), Tesseract
  5.4.0 + `fra` (Windows seulement), ffmpeg/ffprobe 6.1.1 statiques, ImageMagick
  7.1.2-35 (Linux AppImage, Windows portable), pandoc 3.13, typst 0.15.1,
  yt-dlp 2026.10.26 ; entrées `unavailable` avec raison pour les plateformes
  sans binaire officiel portable. Types d'archive : raw, gz, zip, tar.gz,
  tar.xz, 7z.
- `src-tauri/src/cancel.rs` : jeton d'annulation partagé + test.
- `src-tauri/src/extract.rs` : extraction sûre ZIP/TAR (anti zip-slip, liens
  ignorés, jamais d'écrasement, dates restaurées, pas de setuid) + 3 tests.
- `src-tauri/src/timeutil.rs` : fuseau local, conversion date → format DOS ZIP,
  secondes Unix pour le champ « extended timestamp » + test.
- `src-tauri/src/sidecar.rs` : `run_with` (annulation qui tue le fils, lecture
  ligne à ligne de stdout/stderr, capture bornée), erreur typée
  `SidecarError` (stderr tronqué), états Diagnostic `present / telechargeable /
  absent`, résolution dans `<données>/bin/<id>/` via le manifeste.
- `Cargo.toml` : dépendances ureq (rustls), tar, flate2, regex, base64, lzma-rs,
  time, feature `time` de zip ; `tempfile` passé en dépendance normale.
- `.gitignore` : `.scratch/` (≈ 750 Mo de téléchargements ayant servi à
  calculer/vérifier les SHA-256 du manifeste, et paquets Ghostscript Ubuntu).

### Manquait
- `src-tauri/src/download.rs` (référencé par `sidecar.rs` → **le code ne
  compilait pas** à la reprise), commandes Tauri `download_binary` et
  `cancel_job`, bouton « Télécharger » du Diagnostic, déclaration des modules
  dans `lib.rs`, ressources 7-Zip dans `tauri.conf.json`.
- Tous les outils Phase 2, la conservation des dates dans les ZIP, les tests
  associés, la mise à jour des statuts du registre et de PLAN.md.

## 2026-11-08 — Phase 2b

Progression (une ligne par groupe) :
- Groupe 0 (remise en vert) : convention de statut unique `Status::Phase1` ↔
  « P1 » dans PLAN.md (= implémenté, testé, exécutable) ; corrige aussi le
  frontend qui attendait déjà `"phase1"` (le backend envoyait `"disponible"` →
  tous les outils auraient été grisés). fmt OK, clippy OK, `cargo test` 73/73.
- Groupe PDF : `pdf-decouper`, `pdf-extraire-pages`, `pdf-supprimer-pages`,
  `pdf-pivoter` (rotation héritée cumulée), `pdf-vers-texte` (couche texte
  lopdf, avertissement si PDF scanné — Ghostscript n'est plus requis),
  `pdf-images-vers-pdf` (A4 ajusté, transparence sur blanc), nouvel outil
  `pdf-infos` (registre : 81 outils). Découpe : sorties validées seulement à la
  fin (annulation/erreur → rien ne reste). fmt OK, clippy OK, test 87/87.
- Groupe Images : `images-redimensionner` (largeur/hauteur/%/côté max, Lanczos3),
  `images-compresser` (JPEG, qualité, alerte si pas plus léger),
  `images-supprimer-exif` (JPEG : EXIF/XMP/IPTC/commentaires retirés **sans
  réencodage**, ICC conservé ; orientation EXIF appliquée si nécessaire),
  `images-pivoter`, `images-retourner`, `images-rogner` (bornes vérifiées),
  `images-niveaux-de-gris`. Orientation EXIF appliquée à la lecture. La
  conversion en lot était déjà couverte par `images-convertir` (multi-fichiers,
  test `lot_avec_un_fichier_invalide`). fmt OK, clippy OK, test 96/96.
- Groupe Archives : `archives-extraire-zip`, `archives-creer-tar`,
  `archives-creer-targz`, `archives-extraire-tar` (gzip détecté au contenu),
  `archives-lister` (zip/tar/tar.gz en pur Rust, 7-Zip n'est plus requis).
  Extraction via `extract.rs` dans un dossier réservé atomiquement
  (`output::reserve_dir`, `<nom>-2/` si besoin, supprimé en cas d'échec ou
  d'annulation). Dates de modification désormais conservées dans les ZIP créés.
  fmt OK, clippy OK, test 104/104.
- Groupe Documents (pandoc) : 13 conversions (md→docx/odt/html/epub,
  docx→md/odt/html, odt→docx/md, html→md/docx, epub→md, extraction de texte).
  Un seul constructeur d'arguments (`pandoc_args`) avec `--sandbox` en tête.
  Pré-traitement Markdown/HTML dans une copie temporaire : images distantes et
  hors dossier (absolu, `..`, lien symbolique) retirées + avertissement ;
  locales incorporées en `data:` URI. Avertissements `[WARNING]` de pandoc
  relayés au journal. Tests avec faux pandoc (args enregistrés, `--sandbox`
  vérifié pour les 13 outils, échec → erreur typée sans sortie). md/docx→PDF
  restent P2 (typst). pandoc absent de la VM : aucune conversion réelle testée.
  fmt OK, clippy OK, test 110/110.
- Groupe Vidéo/audio (ffmpeg) : `video-convertir` (mp4/mkv/mov H.264+AAC,
  webm VP9+Opus), `video-compresser` (CRF), `video-couper` / `audio-couper`
  (précis ou rapide sans réencodage), `video-extraire-audio`, `audio-convertir`,
  `video-extraire-image`, `video-supprimer-son`, nouvel outil `video-infos`
  (ffprobe, secours en-tête ffmpeg ; registre : 82 outils). Progression réelle
  (`-progress pipe:1` / `Duration:`). Sortie réservée avant l'appel (le `-y`
  ne touche que notre réservation). Tests : arguments purs + bout en bout avec
  le **vrai** ffmpeg 8.1.2 de la VM sur un clip généré (lavfi), ignorés
  proprement si ffmpeg est absent (CI). fmt OK, clippy OK, test 115/115.
- Groupe Frontend : Diagnostic avec colonne « Action » — bouton « Télécharger
  <version> (<taille>) » pour les binaires téléchargeables (commande
  `download_binary`, événements `download-event` : barre de progression,
  bouton « Annuler » via `cancel_job`, diagnostic relancé à la fin), raison
  affichée si non disponible. Panneau outil : bouton « Annuler » (`cancel_job`)
  pendant une tâche, gestion de l'événement `cancelled`, bandeau jaune
  « Avertissements » (images distantes retirées, etc.). `node --check` OK ;
  non testé graphiquement (VM sans GUI).

### Bilan phase 2b (2026-11-08)
Outils réellement exécutables (statut P1 / `Status::Phase1`, implémentés et
testés) : **45 sur 82** — PDF 8/18, Documents 13/15, Images 8/17,
Vidéo/audio 9/19, Archives 6/9, Utilitaires 1/4. Restent P2 : 21, P3 : 16.

Commandes finales (dans `src-tauri/`) :
- `cargo fmt --check` → code 0.
- `cargo clippy --all-targets -- -D warnings` → code 0, aucun avertissement.
- `cargo test` → 115 passés, 0 échec (lib) ; 0 test dans main et doctests.
  Le test de bout en bout ffmpeg a réellement tourné (ffmpeg 8.1.2 présent).

Aucune commande n'a dépassé 10 minutes (clippy/test < 1 min après le premier
build). Aucun binaire commité ; aucun test dépendant du réseau.

Limites restantes :
- pandoc absent de la VM : les 13 outils Documents sont testés avec un faux
  pandoc (arguments, `--sandbox`, avertissements, erreurs), pas sur de vraies
  conversions. docx→md n'extrait pas les images. md/docx→PDF (typst) : P2.
- Références d'images Markdown de type « définition » (`![x][id]` + `[id]: url`)
  non réécrites : pandoc en sandbox les supprime quand même, et son
  `[WARNING]` est relayé, mais sans la liste explicite des URL.
- Retrait EXIF JPEG : métadonnées placées après le premier SOS (rarissime)
  non retirées. Compression image = JPEG uniquement.
- Outils ffmpeg : progression par fichier (un lot repart de 0 à chaque
  fichier). Codecs fixes par format (H.264/AAC, VP9/Opus).
- `pdf-vers-texte` : qualité dépendante de l'encodage des polices (lopdf) ;
  pas d'OCR. Extraction de pages : ordre du document conservé (pas de
  réordonnancement).
- Frontend vérifié par `node --check` seulement (pas de GUI sur la VM).
- Ghostscript, 7-Zip, ImageMagick, typst, yt-dlp, whisper, Real-ESRGAN :
  outils correspondants toujours P2/P3.

## 2026-11-08 — Phase 3a (vrais sidecars installés sur la VM)

Binaires présents : Ghostscript 10.02.1, Tesseract 5.3.4 (eng+fra), ImageMagick
6.9.12 (`convert`, pas de `magick`), 7-Zip 23.01 (`7z`), pandoc 3.1.3, qpdf
11.9.0, ffmpeg 8.1.2, yt-dlp 2024.04.09. Installés **par le code de l'app**
(`cargo run --example installer -- <id> ../.scratch/appdata/bin` : HTTPS,
taille + SHA-256 du manifeste vérifiés) : typst 0.15.1 et qpdf 12.4.2. Les tests
« réels » utilisent PATH + `.scratch/appdata/bin` et sont ignorés proprement
si le binaire manque (CI).

Progression (une ligne par groupe) :
- Groupe PDF : `pdf-compresser`, `pdf-niveaux-de-gris`, `pdf-reparer`,
  `pdf-vers-images` (PNG/JPEG, `%` du chemin échappé, tout ou rien),
  `pdf-ocr` (gs 300 DPI → Tesseract fra+eng par page → réassemblage lopdf),
  `pdf-proteger` / `pdf-deproteger` (**qpdf devient un sidecar**, AES-256, mot
  de passe jamais sur la ligne de commande : `qpdf @fichier`), `pdf-filigrane`,
  `pdf-numeroter`, `pdf-supprimer-metadonnees` (lopdf). Manifeste : qpdf 12.4.2
  (4 plateformes, `aliases` pour les liens symboliques des bibliothèques).
  Téléchargements : ureq utilise désormais les racines de confiance du système
  (`platform-verifier`) — l'installeur échouait derrière le proxy TLS de la VM
  (« UnknownIssuer »), comme il l'aurait fait derrière un proxy d'entreprise.
  Tests **réels** : gs (compression plus légère, gris vérifié pixel par pixel,
  réparation d'une xref cassée, rastérisation), Tesseract (texte « BONJOUR »
  retrouvé dans un PDF image), qpdf (chiffrement, mauvais mot de passe refusé
  sans sortie, déchiffrement). PDF 18/18. fmt OK, clippy OK, test 131/131.
- Groupe Images : ImageMagick accepté sous `magick` (v7) **et** `convert` (v6,
  jamais sous Windows où `convert.exe` est un outil système) ; `montage` =
  `magick montage` ou binaire voisin. `images-convertir` lit HEIC/HEIF/AVIF et
  écrit AVIF via ImageMagick, avec erreur claire si la build ne gère pas le
  format (ici IM 6.9.12 : AVIF/HEIC en lecture seule → écriture AVIF refusée
  proprement, lecture AVIF testée sur un fichier produit par ffmpeg).
  `images-filigrane` (texte échappé : `%`, `@fichier`), `images-detourer` (fond
  uni depuis les bords, tolérance ; rembg = option future), `images-planche-contact`,
  `images-optimiser-web` (JPEG intégré / WebP avec perte IM), `images-vers-ico`
  (7 tailles), `images-bordure`, `images-assembler-gif`, nouvel outil
  `images-vignettes` (registre : 83), `images-ocr-texte` (Tesseract réel),
  `images-agrandir-ia` (Real-ESRGAN : binaire + modèles du zip officiel ajoutés
  au manifeste, SHA-256 calculés au téléchargement faute d'empreinte amont ;
  installé dans `.scratch` par l'installeur de l'app). Sans GPU Vulkan (cas de
  la VM : « vkCreateInstance failed »), repli Lanczos ×4 **signalé**
  (`-x4-lanczos.png`) ; chemin IA testé avec un faux binaire (arguments,
  modèles, progression). Images 18/18. fmt OK, clippy OK, test 145/145.
- Groupe Vidéo/audio : `video-vers-gif` (palette une passe), `video-redimensionner`
  (jamais d'agrandissement), `video-pivoter`, `video-accelerer` (setpts + atempo
  chaîné), `audio-normaliser` (loudnorm), `video-incruster-sous-titres` (libass ;
  sous-titres copiés sous un nom fixe dans un dossier de travail → aucun chemin
  utilisateur dans le filtre), nouveaux outils `video-concatener` (taille de la
  1re, silence ajouté si une vidéo n'a pas de son), `video-vignettes` (planche),
  `video-extraire-sous-titres` (pistes texte ; image → avertissement) — registre :
  86. Tous testés avec le **vrai** ffmpeg 8.1.2. whisper : `video-sous-titres`
  (SRT/VTT) et `audio-transcrire` (TXT), WAV 16 kHz par ffmpeg, progression `-pp`.
  `whisper-cli` 1.9.5 épinglé (linux/windows, empreintes GitHub) et installé dans
  `.scratch` : il tourne, mais le **modèle** manque (Hugging Face injoignable
  depuis la VM : SHA-256 non épinglé, entrée `models` « unavailable » avec la
  raison ; message clair testé avec le vrai binaire). Câblage complet testé avec
  un faux whisper-cli + modèle factice. yt-dlp : `video-telecharger` /
  `audio-telecharger` (URL http/https validée et passée après `--`,
  `--ignore-config`, `--no-playlist` par défaut, dossier de travail dans
  Téléchargements puis noms réservés → jamais d'écrasement), testés avec un faux
  yt-dlp (aucun téléchargement réel). Nouvelle commande `check_ytdlp_update` +
  bouton « Vérifier la mise à jour » / « Mettre à jour vers … » dans le
  Diagnostic : testée avec le vrai yt-dlp 2024.04.09 (mise à jour 2026.10.26
  proposée). Modèles whisper visibles dans le Diagnostic. Vidéo/audio 22/22.
  fmt OK, clippy OK, test 157/157.
- Groupe Archives : `archives-creer-7z` (niveau, mot de passe facultatif →
  AES-256 + noms chiffrés), `archives-extraire-7z` (7z/RAR/ISO/CAB/XZ… dans un
  dossier neuf réservé, liens symboliques restants supprimés), `archives-tester`
  (rapport dans le journal), nouvel outil `archives-ajouter` (copie complétée
  `<nom>-complete.<ext>`, original intact ; registre : 87), `archives-lister`
  étendu aux formats 7-Zip (`7z l -slt`). Mot de passe **jamais en argument** :
  écrit sur l'entrée standard de 7-Zip (nouvelle option `stdin` de
  `sidecar::run_with`) ; `--` avant les chemins, `*`/`?` refusés (jokers 7-Zip).
  Création dans un dossier de travail puis `output::fill_from` (remplit la
  réservation, jamais d'écrasement). Tests **réels** avec 7-Zip 23.01 (système) :
  création chiffrée sans écraser `photos.7z`, mauvais mot de passe refusé sans
  dossier résiduel, extraction, test d'intégrité, listage, ajout à un ZIP,
  archive abîmée détectée. Installation de 7-Zip 26.06 depuis le manifeste :
  échec réseau ponctuel (GitHub 504 puis délai de connexion), à retenter.
  Archives 10/10. fmt OK, clippy OK, test 161/161.
