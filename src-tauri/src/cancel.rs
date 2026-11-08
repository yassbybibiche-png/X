//! Annulation coopérative des tâches (drapeau partagé).
//!
//! Les boucles des outils pur Rust consultent [`CancelToken::check`] ; l'exécution
//! des sidecars ([`crate::sidecar::run_with`]) tue le processus fils dès que le
//! drapeau est levé.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Erreur renvoyée quand l'utilisateur a annulé la tâche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tâche annulée par l'utilisateur.")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` si l'annulation a été demandée.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Vrai si l'erreur (ou une de ses causes) est une annulation.
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.downcast_ref::<Cancelled>().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drapeau_partage_entre_clones() {
        let token = CancelToken::default();
        let clone = token.clone();
        assert!(token.check().is_ok());
        clone.cancel();
        assert_eq!(token.check(), Err(Cancelled));
        let err = anyhow::Error::new(Cancelled).context("pendant la conversion");
        assert!(is_cancelled(&err));
        assert!(!is_cancelled(&anyhow::anyhow!("autre")));
    }
}
