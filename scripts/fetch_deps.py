#!/usr/bin/env python3
"""Télécharge les fichiers de `scripts/deps.lock.json` pour une plateforme et
vérifie taille + SHA-256 (même contrôle que l'app). Bibliothèque standard
uniquement. Les fichiers sont rangés sous `<dest>/<nom de fin d'URL>`, le
format attendu par `cargo run --example installer -- <id> <bin> --depuis <dest>`.

Un fichier déjà présent avec la bonne empreinte n'est jamais retéléchargé
(c'est ce qui rend le cache CI efficace).

    python3 scripts/fetch_deps.py --platform linux-x64 --dest deps/dl [--only 7zip,ffmpeg]
"""

import argparse
import hashlib
import json
import pathlib
import sys
import time
import urllib.request

MANIFEST = pathlib.Path(__file__).with_name("deps.lock.json")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fetch(entry, dest):
    name = entry["url"].rsplit("/", 1)[-1]
    target = dest / name
    if target.is_file() and sha256(target) == entry["sha256"]:
        print(f"  déjà en cache : {name}")
        return
    tmp = target.with_suffix(target.suffix + ".part")
    for attempt in range(1, 4):
        try:
            print(f"  téléchargement : {entry['url']} (essai {attempt})")
            with urllib.request.urlopen(entry["url"], timeout=120) as r, open(tmp, "wb") as f:
                while block := r.read(1 << 20):
                    f.write(block)
            break
        except OSError as e:
            print(f"  échec : {e}")
            if attempt == 3:
                raise
            time.sleep(10 * attempt)
    size, digest = tmp.stat().st_size, sha256(tmp)
    if size != entry["size"] or digest != entry["sha256"]:
        tmp.unlink()
        sys.exit(f"EMPREINTE INVALIDE pour {name} : {size} o / {digest}, attendu {entry['size']} o / {entry['sha256']}")
    tmp.replace(target)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--platform", required=True, choices=["linux-x64", "windows-x64", "macos-x64", "macos-arm64"])
    p.add_argument("--dest", required=True, type=pathlib.Path)
    p.add_argument("--only", default="", help="ids séparés par des virgules (défaut : tous)")
    args = p.parse_args()
    only = {s for s in args.only.split(",") if s}
    args.dest.mkdir(parents=True, exist_ok=True)
    binaries = json.loads(MANIFEST.read_text(encoding="utf-8"))["binaries"]
    for bin_id, spec in binaries.items():
        if only and bin_id not in only:
            continue
        plat = spec.get("platforms", {}).get(args.platform, {})
        if "files" not in plat:
            print(f"{bin_id} : non disponible sur {args.platform} — {plat.get('unavailable', 'absent du manifeste')}")
            continue
        print(f"{bin_id} {spec['version']} :")
        for entry in plat["files"]:
            fetch(entry, args.dest)


if __name__ == "__main__":
    main()
