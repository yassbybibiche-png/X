# PLAN — Boîte à outils

Application de bureau (Windows / macOS / Linux) qui regroupe ~80 petits outils de
conversion et de traitement de fichiers, 100 % locaux, avec une interface en
français. Ce document est la référence d'architecture ; toute décision listée ici
est **tranchée** (pas de « à voir plus tard » implicite).

---

## 1. Architecture

```
boite-a-outils-claude/
├── src/                      Frontend vanilla HTML/CSS/JS (aucun bundler, aucun framework)
│   ├── index.html
│   ├── styles.css
│   └── main.js               appelle les commandes Tauri via window.__TAURI__
├── src-tauri/                Backend Rust (Tauri 2)
│   ├── Cargo.toml
│   ├── tauri.conf.json       CSP stricte, externalBin (sidecars bundlés)
│   ├── capabilities/         permissions minimales (core + dialog:open)
│   ├── binaries/             sidecars bundlés (nommés <bin>-<triple cible>)
│   ├── src/
│   │   ├── main.rs           point d'entrée (appelle lib::run)
│   │   ├── lib.rs            construction de l'app Tauri + commandes
│   │   ├── commands.rs       list_tools, check_tools, run_tool
│   │   ├── registry/         déclaration des 80 outils (un fichier par catégorie)
│   │   ├── sidecar.rs        résolution + exécution des binaires tiers
│   │   ├── output.rs         noms de sortie non écrasants
│   │   ├── jobs.rs           exécution d'un outil + événements de progression
│   │   ├── checksum.rs       SHA-256 (vérification des téléchargements, outil empreinte)
│   │   └── tools/            implémentations (pdf.rs, images.rs, archives.rs, …)
│   └── tests/fixtures/       fixtures minimales (la plupart sont générées en test)
├── scripts/deps.lock.json    (phase 2) manifeste des binaires : URL + SHA-256 épinglés
└── .github/workflows/        (phase 3) ci.yml, deps.yml, release.yml, smoke.yml
```

### 1.1 Registre d'outils (`registry/`)
- Chaque outil est une entrée statique `ToolDef` : `id`, `category`, `name` (FR),
  `description` (FR), `binaries` (sidecars requis), `inputs` (extensions acceptées,
  multi-fichiers ou non, dossiers acceptés ou non), `output` (description FR),
  `options` (liste typée : choix, nombre, texte, booléen, avec valeur par défaut),
  `status` (`Phase1` = implémenté, `Phase2`, `Phase3`).
- La commande Tauri `list_tools` renvoie le registre sérialisé ; le frontend
  construit les catégories, la liste d'outils et le panneau d'options **uniquement**
  à partir de ces données (aucune liste en dur côté JS).
- Un test vérifie : 80 outils, ids uniques, chaque id présent dans ce PLAN.md,
  chaque outil `Phase1` a une implémentation, chaque outil non implémenté renvoie
  l'erreur explicite « non implémenté en phase 1 ».

### 1.2 Résolution des sidecars (`sidecar.rs`)
Ordre de résolution, pour chaque binaire logique (ex. `ghostscript`) et chacun de
ses noms candidats (ex. `gswin64c`, `gs`) :
1. **Bundlé** : dossier de l'exécutable de l'app (là où Tauri dépose les
   `externalBin`, sans le suffixe de triple cible).
2. **Données de l'app** : `<app_data_dir>/bin/` — dossier modifiable par
   l'utilisateur où sont téléchargés au premier usage les binaires non bundlés
   (et yt-dlp, mis à jour sur demande).
3. **PATH système** en secours (recherche manuelle, `PATHEXT` sous Windows).

Exécution : `std::process::Command` **directement**, jamais via un shell (pas de
`sh -c`, pas de `cmd /C`) ; arguments passés un par un (`OsStr`), donc aucun
problème d'échappement ni d'injection, y compris pour les chemins Windows avec
espaces. Timeout (processus tué au-delà), capture stdout/stderr dans des threads
séparés (pas d'interblocage sur tampon plein), code de retour vérifié.

Diagnostic (`check_tools`) : pour chaque binaire → présent/absent, source
(bundlé / données app / PATH), chemin résolu, version (première ligne de
`--version` ou équivalent). L'UI grise les outils dont un binaire manque, affiche
la raison et (phase 2) un bouton « Télécharger ».

### 1.3 Sorties non écrasantes (`output.rs`)
- Sortie **à côté de l'original** (même dossier), nom = `<base>-<suffixe>.<ext>`
  (ou `<base>.<ext>` si l'extension change et que le nom est libre).
- Collision → `<nom>-2.<ext>`, `<nom>-3.<ext>`, …
- La réservation se fait par `OpenOptions::create_new(true)` (atomique côté
  système de fichiers) : même en cas de course entre deux tâches, **aucun fichier
  existant n'est jamais écrasé**. En cas d'échec de l'outil, le fichier réservé
  est supprimé.

### 1.4 Tâches et progression (`jobs.rs`)
- `run_tool(tool_id, inputs, options)` crée un `job_id`, lance le travail dans
  `spawn_blocking`, et renvoie immédiatement l'id.
- Les implémentations reçoivent un `Reporter` (trait) : `progress(pourcentage,
  message)`, `log(message)`, `warn(message)`. L'implémentation Tauri émet
  l'événement `job-event` (`{jobId, kind, percent, message, outputs}`) ;
  l'implémentation de test collecte en mémoire (testable sans GUI).
- Fin de tâche : événement `done` (liste des fichiers produits) ou `error`
  (message FR).
- Phase 2 : annulation (drapeau atomique + kill du sidecar), file d'attente.

### 1.5 Frontend
HTML/CSS/JS vanilla, ~3 fichiers, aucune dépendance npm à l'exécution
(`@tauri-apps/cli` seulement en dev). `withGlobalTauri` activé. Écrans :
catégories → outils → zone de dépôt (glisser-déposer natif Tauri +
bouton « Choisir des fichiers » via `tauri-plugin-dialog`) → options générées →
« Lancer » → barre de progression + journal d'exécution. Onglet « Diagnostic »
listant les binaires. CSP stricte (`default-src 'self'`), aucun appel réseau
depuis le webview.

### 1.6 Sécurité / réseau
- Aucun fichier ne quitte la machine. Seuls appels réseau autorisés :
  téléchargements de binaires/modèles (module `download`, phase 2, URL + SHA-256
  épinglés dans `scripts/deps.lock.json`, vérification avant activation, écriture
  atomique) et yt-dlp (à la demande explicite de l'utilisateur).
- pandoc toujours lancé avec `--sandbox` (voir §3.4).
- Archives : extraction protégée contre le « zip slip » (chemins `..` ou absolus
  refusés), liens symboliques non suivis.

---

## 2. Liste des outils (80)

Statut : **P1** = implémenté, testé et exécutable (quelle que soit la phase où
il a été livré ; `Status::Phase1` dans le registre) ; **P2** = phase 2 (outils purs
Rust + sidecars simples) ; **P3** = phase 3 (IA, OCR, téléchargement, cas
complexes). Binaire « — » = pur Rust, aucun sidecar.

### PDF (18)
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| pdf-fusionner | — (lopdf) | ≥2 PDF → `<1er>-fusionne.pdf` | **P1** |
| pdf-decouper | — (lopdf) | 1 PDF → 1 PDF par page / par plage | **P1** |
| pdf-extraire-pages | — (lopdf) | 1 PDF + plages → `-extrait.pdf` | **P1** |
| pdf-supprimer-pages | — (lopdf) | 1 PDF + plages → `-allege.pdf` | **P1** |
| pdf-pivoter | — (lopdf) | PDF + angle (+ pages) → `-pivote.pdf` | **P1** |
| pdf-compresser | ghostscript | PDF + niveau (écran/ebook/impression) → `-compresse.pdf` | **P1** |
| pdf-ocr | ghostscript + tesseract | PDF scanné → pages rastérisées 300 DPI → Tesseract (fra+eng, image + couche texte) → réassemblage lopdf `-ocr.pdf` | **P1** |
| pdf-vers-images | ghostscript | PDF + DPI + format → `<nom>-p<N>.png/.jpg` par page | **P1** |
| pdf-images-vers-pdf | — (image + lopdf) | images → `<1re>-images.pdf` | **P1** |
| pdf-proteger | qpdf (AES-256) | PDF + mot de passe → `-protege.pdf` | **P1** |
| pdf-deproteger | qpdf | PDF + mot de passe → `-deprotege.pdf` | **P1** |
| pdf-filigrane | — (lopdf) | PDF + texte + opacité → `-filigrane.pdf` (diagonale, Helvetica WinAnsi) | **P1** |
| pdf-vers-texte | — (lopdf) | PDF → `.txt` (couche texte ; PDF scanné → OCR) | **P1** |
| pdf-numeroter | — (lopdf) | PDF + format + position → `-numerote.pdf` | **P1** |
| pdf-niveaux-de-gris | ghostscript | PDF → `-gris.pdf` | **P1** |
| pdf-reparer | ghostscript | PDF abîmé → `-repare.pdf` (table des objets reconstruite) | **P1** |
| pdf-supprimer-metadonnees | — (lopdf) | PDF → `-sans-metadonnees.pdf` (/Info, XMP, PieceInfo) | **P1** |
| pdf-infos | — (lopdf) | PDF → `-infos.txt` (pages, version, format, métadonnées) | **P1** |

### Documents (15) — pandoc, toujours `--sandbox`
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| documents-md-vers-docx | pandoc | .md → .docx | **P1** |
| documents-md-vers-odt | pandoc | .md → .odt | **P1** |
| documents-md-vers-html | pandoc | .md → .html autonome | **P1** |
| documents-md-vers-epub | pandoc | .md → .epub | **P1** |
| documents-md-vers-pdf | pandoc + typst | .md → .pdf | P2 |
| documents-docx-vers-md | pandoc | .docx → .md (images non extraites : `--extract-media` écrirait hors sandbox) | **P1** |
| documents-docx-vers-odt | pandoc | .docx → .odt | **P1** |
| documents-docx-vers-html | pandoc | .docx → .html | **P1** |
| documents-docx-vers-pdf | pandoc + typst | .docx → .pdf | P2 |
| documents-odt-vers-docx | pandoc | .odt → .docx | **P1** |
| documents-odt-vers-md | pandoc | .odt → .md | **P1** |
| documents-html-vers-md | pandoc | .html → .md | **P1** |
| documents-html-vers-docx | pandoc | .html → .docx | **P1** |
| documents-epub-vers-md | pandoc | .epub → .md | **P1** |
| documents-extraire-texte | pandoc | .docx/.odt/.html/.epub/.md/.rtf → .txt | **P1** |

### Images (18)
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| images-convertir | — (crate image) ; imagemagick pour AVIF/HEIC | images (+ .heic/.heif/.avif) → png/jpg/webp/bmp/tiff/gif/ico/tga/qoi/avif ; erreur claire si la build IM ne lit/n'écrit pas le format | **P1** |
| images-redimensionner | — (crate image) | images + taille/% → `-redim` | **P1** |
| images-compresser | — (crate image) | images + qualité → `-compresse.jpg` | **P1** |
| images-supprimer-exif | — (JPEG : segments retirés sans réencodage ; autres : réencodage crate image) | images → `-sans-exif` | **P1** |
| images-agrandir-ia | realesrgan-ncnn-vulkan + modèles (zip officiel) | image → `-x4.png` ; sans GPU Vulkan : repli Lanczos **signalé** `-x4-lanczos.png` | **P1** |
| images-detourer | imagemagick | image + couleur de fond (auto = coin) / tolérance → PNG transparent (remplissage depuis les bords ; rembg = option future) | **P1** |
| images-pivoter | — (crate image) | images + angle → `-pivote` | **P1** |
| images-retourner | — (crate image) | images + axe → `-retourne` | **P1** |
| images-rogner | — (crate image) | image + rectangle → `-rogne` | **P1** |
| images-niveaux-de-gris | — (crate image) | images → `-gris` | **P1** |
| images-filigrane | imagemagick | images + texte + position + opacité → `-filigrane` | **P1** |
| images-vers-ico | — (crate image) | image → .ico 16 à 256 px (carré transparent) | **P1** |
| images-planche-contact | imagemagick (montage) | images → `planche.jpg` | **P1** |
| images-bordure | — (crate image) | images + épaisseur/couleur → `-bordure` | **P1** |
| images-optimiser-web | — (crate image) ; imagemagick pour WebP avec perte | images → JPEG q80 / WebP q80 redimensionné ≤ N px, sans métadonnées | **P1** |
| images-ocr-texte | tesseract (fra+eng) | image → .txt | **P1** |
| images-assembler-gif | — (crate image) | ≥2 images → GIF animé en boucle | **P1** |
| images-vignettes | — (crate image) | images + côté max. → `-vignette.<ext>` (ajouté en phase 3a) | **P1** |

### Vidéo / audio (22)
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| video-convertir | ffmpeg | vidéo → mp4/mkv/webm/mov | **P1** |
| video-compresser | ffmpeg | vidéo + CRF → `-compresse.mp4` | **P1** |
| video-couper | ffmpeg | vidéo + début/fin → `-extrait` | **P1** |
| video-extraire-audio | ffmpeg | vidéo → mp3/m4a/wav/flac | **P1** |
| video-vers-gif | ffmpeg | vidéo + plage/fps/largeur → .gif (palette optimisée, une passe) | **P1** |
| video-extraire-image | ffmpeg | vidéo + horodatage → .png | **P1** |
| video-sous-titres | ffmpeg + whisper-cli + modèle | vidéo/audio → .srt et/ou .vtt | **P1** |
| video-incruster-sous-titres | ffmpeg (libass) | vidéo + .srt/.vtt/.ass → `-soustitre.mp4` | **P1** |
| video-redimensionner | ffmpeg | vidéo + hauteur → `-720p` … (jamais d'agrandissement) | **P1** |
| video-pivoter | ffmpeg | vidéo + angle → `-pivote` | **P1** |
| video-supprimer-son | ffmpeg | vidéo → `-muet` | **P1** |
| video-accelerer | ffmpeg | vidéo + facteur (×0,25 à ×4) → `-x2` … | **P1** |
| video-telecharger | yt-dlp (+ ffmpeg) | URL + qualité → Téléchargements/`<titre> [<id>].<ext>` | **P1** |
| audio-telecharger | yt-dlp (+ ffmpeg) | URL → mp3/m4a/opus dans Téléchargements | **P1** |
| audio-convertir | ffmpeg | audio → mp3/m4a/ogg/opus/wav/flac | **P1** |
| audio-couper | ffmpeg | audio + début/fin → `-extrait` | **P1** |
| audio-normaliser | ffmpeg (loudnorm) | audio → `-normalise` | **P1** |
| audio-transcrire | ffmpeg + whisper-cli + modèle | audio/vidéo → .txt | **P1** |
| video-infos | ffmpeg (ffprobe) | vidéo/audio → `-infos.txt` (durée, pistes, codecs) | **P1** |
| video-concatener | ffmpeg | ≥2 vidéos → `<1re>-assemble.mp4` (taille de la 1re, réencodage ; ajouté en phase 3a) | **P1** |
| video-vignettes | ffmpeg | vidéo + grille → `-vignettes.jpg` (captures réparties ; ajouté en phase 3a) | **P1** |
| video-extraire-sous-titres | ffmpeg (ffprobe) | vidéo → `-st<N>[-langue].srt/.vtt/.ass` (pistes texte ; ajouté en phase 3a) | **P1** |

### Archives (9)
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| archives-creer-zip | — (crate zip) | fichiers/dossiers → .zip | **P1** |
| archives-extraire-zip | — (crate zip) | .zip → dossier `<nom>/` (anti zip-slip) | **P1** |
| archives-creer-7z | 7zip | fichiers/dossiers → .7z | P2 |
| archives-extraire-7z | 7zip | .7z/.rar/.iso/… → dossier | P2 |
| archives-creer-tar | — (crate tar) | fichiers/dossiers → .tar | **P1** |
| archives-creer-targz | — (tar + flate2) | fichiers/dossiers → .tar.gz | **P1** |
| archives-extraire-tar | — (tar + flate2) | .tar/.tar.gz → dossier | **P1** |
| archives-lister | — (zip/tar) ; 7zip pour les autres formats (P2) | .zip/.tar/.tar.gz → `-contenu.txt` | **P1** |
| archives-tester | 7zip | archive → rapport d'intégrité | P2 |

### Utilitaires (4)
| id | binaire | entrées → sortie | statut |
|---|---|---|---|
| utilitaires-empreinte-sha256 | — (sha2) | fichiers → `<nom>.sha256` | **P1** |
| utilitaires-verifier-sha256 | — (sha2) | fichier + empreinte attendue → verdict | P2 |
| utilitaires-renommer-lot | — | fichiers + motif → copies renommées (jamais d'écrasement) | P3 |
| utilitaires-qrcode | — (crate qrcode) | texte → .png | P3 |

Total : 18 + 15 + 17 + 19 + 9 + 4 = **82** (80 prévus + `pdf-infos` et `video-infos`
ajoutés en phase 2b).

---

## 3. Décisions tranchées

### 3.1 pdf-fusionner : lopdf (pur Rust), pas Ghostscript
- Fusion **structurelle** (copie des objets, renumérotation, nouvel arbre de
  pages) : sans perte, texte et images intacts, rapide. Ghostscript (`pdfwrite`)
  **re-distille** le document : polices ré-embarquées, liens/formulaires parfois
  perdus, et taille qui varie.
- Fonctionne sur machine vierge sans aucun sidecar ; Ghostscript est AGPL et
  absent de la VM de dev (et de beaucoup de machines).
- Limites connues (documentées dans l'UI) : signets et formulaires AcroForm des
  documents sources non fusionnés en phase 1 ; PDF chiffrés refusés avec un
  message clair.

### 3.2 images-convertir : crate `image` (pur Rust), ImageMagick en complément
- Formats en pur Rust : PNG, JPEG (qualité réglable), WebP (sans perte — l'encodeur
  WebP du crate `image` ne fait que du lossless), BMP, TIFF, GIF, ICO, TGA, QOI.
- Fonctionne partout sans sidecar, testable en CI sans réseau.
- AVIF/HEIC (encodeur AVIF pur Rust = rav1e, compilation très lourde) et WebP
  avec perte → ImageMagick en phase 2. En phase 1, demander AVIF renvoie une
  erreur explicite.

### 3.3 Bundling vs téléchargement au premier usage (par binaire)
Principe : **rien ne doit exiger une installation manuelle**. Chaque binaire est
soit bundlé (sidecar Tauri), soit téléchargé par l'app au premier usage dans
`<app_data_dir>/bin/` depuis une URL **épinglée** avec **SHA-256 épinglé**
(`scripts/deps.lock.json`), vérifié avant activation. Le PATH système n'est qu'un
secours.

| binaire | licence | taille approx. | décision | raison |
|---|---|---|---|---|
| 7-Zip (`7zz`/`7za`) | LGPL-2.1 (+ restriction unRAR) | 2–3 Mo | **bundlé** | petit, redistribuable (binaire + licence + lien source) ; sert aussi à extraire les installeurs Windows de Ghostscript/Tesseract |
| Ghostscript | **AGPL-3.0** (ou licence commerciale Artifex) | 40–60 Mo | **téléchargé au premier usage** | voir §3.3.1 |
| pandoc | GPL-2.0+ | 150–200 Mo décompressé | téléchargé au premier usage | taille ; binaire officiel par OS ; GPL : en téléchargeant depuis l'amont on ne redistribue pas |
| typst (moteur PDF de pandoc) | Apache-2.0 | ~40 Mo | téléchargé au premier usage | seulement pour les sorties PDF ; évite LaTeX (Go) et Chromium |
| ffmpeg | LGPL/GPL selon build (builds GPL avec x264) | 80–130 Mo | téléchargé au premier usage | taille ; builds statiques épinglés (BtbN Windows/Linux, build macOS épinglé) |
| ImageMagick | licence ImageMagick (type Apache-2.0) | 30–50 Mo | téléchargé au premier usage | taille ; seulement AVIF/HEIC/détourage/montage/filigrane — le reste est pur Rust |
| Tesseract + `fra`/`eng` traineddata | Apache-2.0 | 30–60 Mo + 2×~4 Mo (`tessdata_fast`) | téléchargé au premier usage | taille ; pas de binaire officiel portable sur macOS/Linux → build CI (§3.3.2) |
| yt-dlp | Unlicense | ~15–35 Mo | **téléchargé au premier usage, jamais figé** | leçon 2 : dossier modifiable, bouton « Mettre à jour », version affichée ; binaires autonomes officiels (`yt-dlp.exe`, `yt-dlp_macos`, `yt-dlp_linux`) → pas de Python requis |
| whisper.cpp (`whisper-cli`) | MIT | 2–10 Mo | téléchargé au premier usage | binaire Windows officiel ; macOS/Linux build CI |
| Real-ESRGAN ncnn Vulkan | BSD-3 | 20–50 Mo (avec modèles) | téléchargé au premier usage | zips officiels par OS ; nécessite Vulkan → message clair + repli redimensionnement Lanczos si absent |
| qpdf | Apache-2.0 | 3–28 Mo (zip officiel) | téléchargé au premier usage (binaires officiels Linux/macOS/Windows, SHA-256 publiés par l'amont) | **sidecar du projet depuis la phase 3a** : voir §3.9 |

Les sources de téléchargement sont, par ordre de préférence : release officielle
amont ; sinon binaire construit par notre workflow `deps.yml` à partir des
sources officielles et publié dans une release GitHub dédiée `deps-AAAAMMJJ` de
ce dépôt (**avec les archives sources exactes attachées à la même release**, ce
qui satisfait l'obligation GPL/AGPL de fournir le code source correspondant).

#### 3.3.1 Ghostscript (AGPL-3.0) — détail
- Ghostscript est distribué par Artifex sous **AGPL-3.0** (ou licence commerciale).
- Notre app appelle `gs` comme **programme séparé** (exécution d'un processus avec
  des arguments en ligne de commande, échange via fichiers) : selon la FAQ de la
  FSF il s'agit d'une « agrégation », notre code n'est pas une œuvre dérivée et
  n'a pas à être sous AGPL.
- **Si nous redistribuons le binaire** (bundle ou release `deps-*`), l'AGPL §4–6
  impose : fournir le texte de la licence, conserver les mentions de copyright,
  et fournir le **code source correspondant** (le mettre à disposition au même
  endroit que le binaire — §6(d) —, ou joindre une offre écrite valable 3 ans,
  §6(b)) ; si nous modifions Ghostscript, publier les modifications. La clause
  réseau §13 ne s'applique qu'à une version **modifiée** offerte à des
  utilisateurs via un réseau : non applicable (pas de modification, usage local).
- **Décision** : Ghostscript n'est **pas** inclus dans les installeurs.
  - Linux : téléchargement au premier usage du binaire officiel Artifex
    (`ghostpdl-downloads`, tarball x86_64) — l'utilisateur obtient le binaire
    directement de l'amont, nous ne redistribuons rien.
  - Windows : téléchargement au premier usage de l'installeur officiel
    `gs…w64.exe`, extrait sans installation (ni droits admin) avec le 7-Zip bundlé.
  - macOS : Artifex ne publie pas de binaire → build par `deps.yml` depuis le
    tarball source officiel, publié dans la release `deps-*` **avec le tarball
    source attaché** et la licence ; THIRD_PARTY.md contient l'offre / le lien.
  - Avant le premier téléchargement, l'UI affiche « Ghostscript — AGPL-3.0 » avec
    lien vers la licence et les sources.
  - Les opérations structurelles PDF (fusion, découpe, rotation, extraction,
    filigrane, métadonnées) sont en **pur Rust (lopdf)** : Ghostscript n'est
    nécessaire que pour compresser, rastériser, niveaux de gris, texte, réparer, OCR.

#### 3.3.2 Binaires sans build officiel portable
Tesseract (macOS/Linux), whisper.cpp (macOS/Linux), Ghostscript (macOS),
ImageMagick (macOS) : construits dans `deps.yml`, publiés dans `deps-*` avec
sources attachées. C'est le point le plus coûteux du projet : il est planifié en
phase 3 et, tant qu'il n'est pas fait, l'outil concerné reste grisé avec une
raison explicite (jamais de faux succès).

### 3.4 pandoc : `--sandbox` partout + images distantes signalées
- **Tous** les appels pandoc ont `--sandbox`, quel que soit le format d'entrée
  (Markdown compris : un `.md` piégé ne doit pas pouvoir inclure `~/.ssh/id_rsa`).
  Un test vérifiera que le constructeur d'arguments pandoc ajoute toujours
  `--sandbox` (aucune autre façon de lancer pandoc dans le code).
- En sandbox, pandoc ne lit que les fichiers passés en ligne de commande : les
  images distantes (http/https) **et** locales disparaissent de la sortie.
- Pré-traitement (Markdown et HTML) : avant l'appel, l'app analyse les références
  d'images :
  - **distantes** → supprimées ; **avertissement visible** dans l'UI (bandeau
    jaune) et dans le journal de l'outil, avec la liste des URL ;
  - **locales** situées dans le dossier du fichier d'entrée (chemin canonique
    contenu dans ce dossier, pas de `..` qui en sort, pas de chemin absolu, pas
    de lien symbolique vers l'extérieur) → incorporées en `data:` URI dans une
    copie temporaire du document, donc lisibles sans I/O par pandoc ;
  - locales hors du dossier → refusées + avertissement.
- DOCX/ODT/EPUB : les images sont dans le conteneur, lues sans I/O externe ;
  les relations `TargetMode="External"` sont signalées de la même façon.

### 3.5 Transcription : whisper.cpp, modèle `small` quantifié par défaut
- Modèle par défaut : `ggml-small-q5_1.bin` (~190 Mo au lieu de 466 Mo pour
  `small` en f16), qualité quasi identique en français.
- Alternative légère proposée dans les options : `ggml-base-q5_1.bin` (~60 Mo),
  plus rapide, moins précise.
- « Whistle » et autres alternatives : **non retenues** — pas de projet maintenu
  comparable en qualité multilingue / français, alors que whisper.cpp est MIT,
  sans dépendance, CPU uniquement.
- Téléchargement au premier usage depuis le dépôt Hugging Face officiel
  `ggerganov/whisper.cpp`, SHA-256 épinglé, progression affichée ; bouton
  « Supprimer le modèle » pour libérer la place (phase 3b).
- **État phase 3a** : `whisper-cli` 1.9.5 (release `b5454`, empreintes publiées
  par GitHub) est épinglé pour linux-x64 et windows-x64 (macOS : build
  `deps.yml`). Les modèles sont déclarés dans la section `models` du manifeste,
  rangés dans `<données>/models/<id>/`, affichés dans le Diagnostic (bouton
  Télécharger via `download_binary("whisper-small")`). Leur SHA-256 n'est **pas
  encore épinglé** : Hugging Face était injoignable depuis la VM de
  développement (aucune empreinte inventée) ; l'entrée est donc `unavailable`
  avec cette raison, et l'outil affiche « Modèle whisper … absent » + où le
  déposer. Il suffira d'ajouter `files` (url, sha256, size) pour activer le
  téléchargement au premier usage, déjà câblé et testé (même code que les
  binaires).

### 3.6 Python embarqué : **non**
- pandoc couvre toutes les conversions de documents ; typst fournit le PDF ;
  yt-dlp est utilisé via ses exécutables autonomes officiels (Python déjà inclus
  par PyInstaller).
- Un Python embarqué (python-build-standalone) coûterait 30–80 Mo par OS, une
  surface d'attaque (pip, paquets) et une chaîne de mise à jour de plus, sans
  outil qui l'exige. Pas de LibreOffice, pas de Chromium (leçon 7).

### 3.7 CI (phase 3)
- `ci.yml` (push/PR) : fmt `--check`, clippy `-D warnings`, tests — cache cargo
  (`Swatinem/rust-cache`). **Aucun binaire tiers téléchargé** : les tests n'en
  dépendent pas (fixtures générées, outils sidecar testés seulement si le binaire
  est présent).
- `deps.yml` séparé (manuel + changement de `scripts/deps.lock.json`) : télécharge
  ou construit les binaires tiers, vérifie les SHA-256, les met en cache
  `actions/cache` avec clé = `hashFiles('scripts/deps.lock.json')` + OS.
- `release.yml` (tag `v*`) : restaure le cache des binaires (jamais de
  re-téléchargement s'il est chaud), build Windows x64 `.msi`, macOS Intel +
  Apple Silicon `.dmg`, Linux `.AppImage`, attache les 4 installeurs à la release.
- `smoke.yml` : smoke test lourd (exécute un outil de chaque binaire réel)
  **uniquement** en `workflow_dispatch` ou sur tag de pré-release (`v*-rc*`),
  jamais à chaque push.

### 3.9 qpdf devient un sidecar (phase 3a) — protection PDF
- Chiffrement/déchiffrement par mot de passe : **qpdf**, pas lopdf. lopdf 0.47
  sait lire certains PDF chiffrés mais n'offre pas d'API d'écriture AES-256
  fiable ; Ghostscript ne chiffre qu'en RC4 40/128 bits (obsolète, cassable).
  qpdf (Apache-2.0) est la référence pour AES-256 (R6, PDF 2.0).
- Distribution : zips officiels qpdf 12.4.2 pour linux-x64, macos-x64,
  macos-arm64 et windows-x64 (msvc64), SHA-256 épinglés dans le manifeste
  (repris des empreintes publiées par l'amont et revérifiés localement). Les
  liens symboliques des bibliothèques (`libqpdf.so.30`, `libqpdf.30.dylib`)
  ne sont jamais extraits (sécurité) : le manifeste les déclare en `aliases`,
  recréés en **copies** après extraction.
- Le mot de passe ne passe **jamais** sur la ligne de commande (visible par
  `ps` / le Gestionnaire des tâches) : les arguments sont écrits dans un
  fichier temporaire privé et passés par `qpdf @fichier`.
- Mot de passe propriétaire = mot de passe d'ouverture (pas de restrictions
  d'usage factices : elles ne protègent rien une fois le PDF ouvert).

### 3.8 Divers
- Complexité : `clippy::cognitive_complexity` activé avec seuil 15
  (`clippy.toml`) comme garde-fou automatique ; fonctions courtes par principe.
- Sorties multi-fichiers (fusion, zip) : placées dans le dossier du premier
  fichier d'entrée.
- yt-dlp : sortie dans le dossier Téléchargements de l'utilisateur (pas de
  fichier d'entrée), même règle de non-écrasement.
- README macOS 15 : clic droit « Ouvrir » ne suffit plus → tenter d'ouvrir, puis
  Réglages Système › Confidentialité et sécurité › « Ouvrir quand même » ; secours
  `xattr -cr "/Applications/Boîte à outils.app"`.

---

## 4. Phases

| phase | contenu |
|---|---|
| **1** | PLAN, squelette Tauri 2, registre des 80 outils, sidecar/output/jobs, diagnostic, UI FR, 4 outils fonctionnels testés |
| **2** | module `download` (manifeste SHA-256, progression, écriture atomique), bouton Télécharger dans le diagnostic, outils pur Rust restants (PDF lopdf, images, archives), outils ffmpeg/pandoc/ghostscript/7-Zip simples, pré-traitement pandoc sandbox, annulation |
| **3** | whisper, Real-ESRGAN, Tesseract/OCR, yt-dlp + mise à jour, outils complexes restants, CI complète (ci/deps/release/smoke), THIRD_PARTY.md, README grand public, audit externe (machine vierge, chemins Windows, sécurité) et corrections |
