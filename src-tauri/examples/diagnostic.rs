//! Diagnostic des binaires en ligne de commande (sans GUI) :
//! `cargo run --example diagnostic`.

use boite_a_outils_lib::sidecar::{Resolver, check_all};

fn main() {
    let statuses = check_all(&Resolver::from_env(None));
    println!(
        "{}",
        serde_json::to_string_pretty(&statuses).expect("sérialisation")
    );
}
