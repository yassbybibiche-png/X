//! Installe un binaire du manifeste avec le vrai code de l'application
//! (téléchargement HTTPS, taille + SHA-256 vérifiés, extraction sûre) :
//! `cargo run --example installer -- <id> <dossier bin>`
//! (ex. `typst ../.scratch/appdata/bin`, utilisé par les tests réels).
//! `--depuis <dossier>` : prend les fichiers déjà téléchargés dans ce dossier
//! (même nom que la fin de l'URL) au lieu du réseau — taille et SHA-256 sont
//! vérifiés exactement de la même façon.

use std::path::PathBuf;

use std::io::Read;

use boite_a_outils_lib::download::{self, Fetch, Http, InstallHooks};
use boite_a_outils_lib::sidecar::{Binary, Resolver};

/// Fichiers locaux nommés comme la fin de l'URL.
struct LocalDir(PathBuf);

impl Fetch for LocalDir {
    fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
        let name = url.rsplit('/').next().unwrap_or_default();
        Ok(Box::new(std::fs::File::open(self.0.join(name))?))
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(id), Some(dir)) = (args.next(), args.next()) else {
        eprintln!("usage : installer <id> <dossier bin> [--depuis <dossier>]");
        std::process::exit(2);
    };
    let local = match (args.next().as_deref(), args.next()) {
        (Some("--depuis"), Some(d)) => Some(LocalDir(PathBuf::from(d))),
        _ => None,
    };
    let binary = Binary::ALL
        .into_iter()
        .find(|b| b.id() == id)
        .unwrap_or_else(|| panic!("binaire inconnu : {id}"));
    let offer = download::offer(binary).unwrap_or_else(|e| panic!("{id} : {e}"));
    let seven_zip = Resolver::from_env(None)
        .resolve(Binary::SevenZip)
        .map(|r| r.path);
    let last = std::sync::atomic::AtomicU64::new(0);
    let hooks = InstallHooks {
        progress: &|done, total| {
            let pct = done * 100 / total.max(1);
            if last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct && pct % 10 == 0 {
                eprintln!("{pct} %");
            }
        },
        log: &|m| eprintln!("{m}"),
        cancel: None,
        seven_zip,
    };
    let fetch: &dyn Fetch = match &local {
        Some(l) => l,
        None => &Http,
    };
    match download::install(&offer, &PathBuf::from(dir), fetch, &hooks) {
        Ok(exe) => println!("{}", exe.display()),
        Err(e) => {
            eprintln!("échec : {e:#}");
            std::process::exit(1);
        }
    }
}
