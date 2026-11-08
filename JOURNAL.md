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
