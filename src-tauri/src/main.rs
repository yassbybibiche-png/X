// Évite une console supplémentaire sous Windows en release. NE PAS SUPPRIMER.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    boite_a_outils_lib::run();
}
