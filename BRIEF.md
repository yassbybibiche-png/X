# Boîte à outils — Brief pour Claude Code (course contre Codex)

Tu es Claude Code, en production. Tu construis seul, de bout en bout, l'app de bureau
"Boîte à outils". Dossier de travail EXCLUSIF : ce repo (`~/workspace/boite-a-outils-claude`).
Ne touche à aucun autre dossier. Ne commets jamais de fichier contenant une config
perso, un e-mail, un chemin local absolu hors repo, ou un secret.

## Référence concurrente (lue par le chef d'atelier, à ne PAS copier)
Gist https://gist.github.com/baptisterajaut/b960dc696dd2c4a9c891d278e6792850
— projet "Remise" / openhangar, clone d'un wrapper payant 45 € autour de
Ghostscript/Tesseract/ffmpeg/ImageMagick/7z. Clean room : ne touche jamais au
binaire hangar, ne copie pas son site, n'utilise que les leçons ci-dessous.

## Leçons du gist — pièges à éviter (obligatoires)
1. **Bundling réel** : la v1 de Remise ne bundlait RIEN — l'utilisateur devait tout
   installer à la main, donc presque rien ne marchait sur machine vierge. Toi :
   les binaires tiers DOIVENT être inclus / téléchargés au premier usage par l'app
   elle-même (sidecars Tauri `src-tauri/binaries/` + téléchargement au premier
   usage dans un dossier modifiable de l'utilisateur, avec SHA-256 épinglés et
   vérification). Si un binaire ne peut pas être bundlé pour une raison de licence
   ou de taille, tranche EXPLICITEMENT dans PLAN.md et THIRD_PARTY.md, avec
   l'alternative (téléchargement au premier usage) — ne passe jamais sous silence.
2. **yt-dlp figé = cassé en quelques semaines** : ne fige pas yt-dlp dans le
   bundle en lecture seule. Télécharge-le au premier usage dans un dossier
   modifiable (données de l'app), propose sa mise à jour dans l'UI (`yt-dlp -U`
   équivalent : re-téléchargement de la release épinglée la plus récente),
   affiche sa version. Idem pour les modèles IA.
3. **pandoc --sandbox PARTOUT, y compris Markdown** : un .md piégé peut exfiltrer
   un fichier privé (ex. clé SSH) dans la sortie. Active `--sandbox` pour TOUS
   les formats d'entrée. Les images distantes (http/https) sont SUPPRIMÉES en
   sandbox : signale-le explicitement à l'utilisateur (avertissement visible
   dans l'UI + dans le log de l'outil). N'autorise que les images locales du
   dossier du fichier d'entrée, via pré-traitement si nécessaire.
4. **CI : pas de re-téléchargement à chaque push** : le smoke test de Remise
   retéléchargeait ~1 Go / système, ~4 Go par push, sans cache. Toi : cache
   GitHub Actions obligatoire pour cargo ET pour les binaires tiers (clé = SHA-256
   épinglés dans `scripts/deps.lock.json` / équivalent), workflow `deps.yml`
   séparé, smoke test lourd SEULEMENT en manuel (`workflow_dispatch`) ou
   pre-release, jamais à chaque push.
5. **README macOS 15** : le clic droit "Ouvrir" NE MARCHE PLUS depuis macOS 15
   (Sequoia). Indique : tenter d'ouvrir, puis Réglages Système > Confidentialité
   et sécurité > "Ouvrir quand même", secours `xattr -cr` en ligne de commande.
6. **Preuve** : ne publie JAMAIS de transcript brut (config perso / e-mail /
   chemins locaux). Preuve = historique Git horodaté + extrait filtré.
7. **Pas de LibreOffice** : conversions documents via pandoc (+ Python embarqué
   si besoin, à trancher dans PLAN.md). Pas de Chromium lourd.

## Cahier des charges
- Rust + Tauri 2, UI web légère, interface EN FRANÇAIS, glisser-déposer.
- Binaires sidecar appelés DIRECTEMENT (std::process::Command, jamais via un
  shell) : Ghostscript, Tesseract (fra+eng), ffmpeg, ImageMagick, 7-Zip,
  pandoc, yt-dlp. Résolution : 1) sidecar bundlé, 2) dossier données app
  (téléchargé au premier usage), 3) PATH système en secours, avec diagnostic
  clair dans l'UI (outil grisé + raison + bouton de téléchargement).
- Modèles IA locaux téléchargés au premier usage (dossier données app, avec
  progression + SHA-256) : whisper.cpp modèle `small` (transcription),
  Real-ESRGAN ncnn (agrandissement images). Alternative plus légère
  (ex. Whistle) à évaluer dans PLAN.md si small est trop lourd — trancher.
- ~80 outils, regroupés par catégorie. Liste cible (à compléter dans PLAN.md) :
  - PDF (9+) : fusionner, découper, compresser, pivoter, OCR, PDF→images,
    images→PDF, protéger, déprotéger, filigrane.
  - Documents (12+) : conversions Markdown/DOCX/HTML/ODT/EPUB/PDF via pandoc
    dans les deux sens quand pandoc le permet, + extraction de texte.
  - Images (12+) : convertir (png/jpg/webp/avif...), redimensionner,
    compresser, supprimer EXIF, agrandir par IA, détourage simple,
    pivoter/retourner, rogner, convertir en niveaux de gris, etc.
  - Vidéo/audio (14+) : convertir, compresser, couper, extraire l'audio,
    GIF animé, extraire une image, sous-titres par transcription (whisper),
    téléchargement yt-dlp (vidéo/audio, choix qualité), convertir audio.
  - Archives (6+) : créer/extraire zip, 7z, tar (+ tar.gz), lister contenu.
  - Compléter jusqu'à ~80 avec des variantes utiles (ex. PDF→texte, images
    en lot, renommage de sortie, etc.) — chaque outil compte dans le registre.
- Chaque outil : entrée (fichiers + drag & drop) → options → barre de
  progression (événements Tauri) → fichier de sortie À CÔTÉ de l'original,
  nom non collisionnant (suffixe), JAMAIS d'écrasement.
- Sécurité : aucune donnée ne quitte la machine, sauf yt-dlp et le
  téléchargement des modèles/binaires. Aucun appel réseau autre que ceux-là.
- Qualité : `cargo clippy -- -D warnings` strict, `cargo fmt --check`,
  complexité cyclomatique ≤ 15 par fonction (vise petit, découpe), tests
  unitaires pour chaque outil avec fichiers d'exemple (générés en test,
  fixtures minimales commitées sous `src-tauri/tests/fixtures/` ou générées).
- Mise à jour yt-dlp proposée dans l'app (bouton + version affichée).
- CI / release (GitHub Actions) :
  - Builds : Windows x64 (.msi), macOS Intel + Apple Silicon (.dmg),
    Linux (.AppImage).
  - Cache cargo + binaires tiers obligatoire.
  - Workflow `deps.yml` séparé (récupère + met en cache les binaires tiers).
  - Release v0.1.0 déclenchée par un tag `v*`, avec les 4 installeurs attachés.
  - README grand public en français (téléchargement, Gatekeeper macOS 15,
    captures d'écran — placeholder honnête si pas de capture réelle).
  - THIRD_PARTY.md : licence de chaque outil inclus. Ghostscript = AGPL :
    vérifie et indique EXPLICITEMENT les obligations de redistribution
    (source disponible, offre écrite, etc.) et la décision prise (bundlé ?
    téléchargé au premier usage ? avec quelles conséquences AGPL ?).
- Méthode : PLAN.md d'abord (architecture, liste des 80 outils, phases,
  décisions/tranches), puis exécution par phases, `git commit` après chaque
  fonctionnalité qui marche. À la fin, relecture d'auditeur externe
  (binaires manquants sur machine vierge, chemins Windows, CI, sécurité) et
  corrections.

## Environnement de cette VM (Linux, pour le dev — la CI fera les autres OS)
- Rust en cours d'installation via rustup (vérifie `~/.cargo/bin/cargo --version`,
  sinon installe-le toi-même). Node 24 disponible. Pas de GUI ici : tu ne peux
  pas lancer l'app graphiquement — vérifie par `cargo test`, `cargo clippy`,
  `cargo fmt --check`, `cargo tauri build` si possible (build Linux au moins
  en mode debug, sinon `cargo check`).
- Si un téléchargement de binaire tiers échoue ici (réseau), ne bloque pas :
  implémente le mécanisme (manifeste SHA-256, téléchargement, vérification)
  et documente-le ; les fixtures de test ne doivent pas dépendre du réseau.

## Règles de travail
- Travaille par petites étapes, commit fréquemment, messages de commit clairs.
- Ne jamais inventer un succès : `cargo test` / clippy / fmt doivent être
  réellement lancés et leurs résultats rapportés.
- Si une exigence est irréaliste, tranche explicitement dans PLAN.md au lieu
  de la passer sous silence.
- À la fin de chaque phase, écris un court bilan dans `JOURNAL.md`
  (date, phase, ce qui a été fait, tests lancés et résultats, reste à faire).
