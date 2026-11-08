# Boîte à outils

> **README provisoire (phase 1).** La version grand public (téléchargement,
> captures d'écran, macOS 15) sera rédigée en phase finale.

Application de bureau (Windows, macOS, Linux) qui regroupe ~80 outils de
conversion et de traitement de fichiers : PDF, documents, images, vidéo/audio,
archives. **Tout se passe sur votre ordinateur** : aucun fichier n'est envoyé
sur Internet (seules exceptions : le téléchargement d'une vidéo avec yt-dlp, et
le téléchargement au premier usage des outils et modèles IA).

## État actuel

- 80 outils déclarés (voir [PLAN.md](PLAN.md)) ; **4 fonctionnent** en phase 1 :
  - **Fusionner des PDF** (pur Rust, sans perte),
  - **Convertir des images** (PNG, JPEG, WebP, BMP, TIFF, GIF, ICO, TGA, QOI),
  - **Créer un ZIP** (fichiers et dossiers),
  - **Empreinte SHA-256**.
- Les autres outils apparaissent grisés avec leur phase prévue ; les lancer
  renvoie un message clair « non implémenté en phase 1 ».
- Onglet **Diagnostic** : présence, version et chemin de chaque binaire tiers.

Les fichiers produits sont créés **à côté de l'original**, avec un suffixe ; un
fichier existant n'est **jamais écrasé** (`photo.jpg` existe → `photo-2.jpg`).

## Développement

Prérequis : Rust stable (≥ 1.90), Node.js (pour la CLI Tauri), et sous Linux
les bibliothèques WebKitGTK (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libsoup-3.0-dev`, `librsvg2-dev`).

```sh
npm install                      # CLI Tauri (dev uniquement)
npx tauri dev                    # lance l'app
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run --example diagnostic   # état des binaires tiers, sans interface
```

## Licence

Code de l'application : MIT. Les binaires tiers ont leur propre licence
(Ghostscript AGPL-3.0, pandoc GPL, etc.) : voir PLAN.md §3.3 (THIRD_PARTY.md
en phase finale).
