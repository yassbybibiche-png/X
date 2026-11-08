//! Installe un binaire du manifeste avec le vrai code de l'application
//! (téléchargement HTTPS, taille + SHA-256 vérifiés, extraction sûre) :
//! `cargo run --example installer -- <id> <dossier bin>`
//! (ex. `typst ../.scratch/appdata/bin`, utilisé par les tests réels).

use std::path::PathBuf;

use boite_a_outils_lib::download::{self, Http, InstallHooks};
use boite_a_outils_lib::sidecar::{Binary, Resolver};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(id), Some(dir)) = (args.next(), args.next()) else {
        eprintln!("usage : installer <id> <dossier bin>");
        std::process::exit(2);
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
    match download::install(&offer, &PathBuf::from(dir), &Http, &hooks) {
        Ok(exe) => println!("{}", exe.display()),
        Err(e) => {
            eprintln!("échec : {e:#}");
            std::process::exit(1);
        }
    }
}
