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
