// Suivi de la tâche lancée depuis le panneau (sans dépendance au DOM, testé
// par tests/frontend/taches.test.js).
//
// Le backend émet ses événements dès le démarrage du thread de travail : pour
// une tâche de quelques millisecondes, ils arrivent tous (y compris « done »)
// AVANT que `run_tool` ait rendu l'identifiant de tâche. Ils sont donc gardés
// en tampon entre `demarrer()` et `associer()`, puis rejoués.

export function creerSuivi() {
  let tache = null;
  let ticket = 0;
  let attente = false;
  let tampon = [];
  return {
    /** Identifiant de la tâche en cours (null si aucune ou pas encore connu). */
    get tache() {
      return tache;
    },
    /** Avant `invoke("run_tool")` : ouvre le tampon ; renvoie un ticket. */
    demarrer() {
      tache = null;
      attente = true;
      tampon = [];
      return ++ticket;
    },
    /** Après `invoke("run_tool")` : renvoie les événements reçus entre-temps
     *  pour cette tâche, ou null si le lancement a été abandonné. */
    associer(t, jobId) {
      if (t !== ticket || !attente) return null;
      attente = false;
      tache = jobId;
      const recus = tampon.filter((ev) => ev.jobId === jobId);
      tampon = [];
      return recus;
    },
    /** true si l'événement est à afficher maintenant (sinon tamponné ou ignoré). */
    recevoir(ev) {
      if (attente) {
        tampon.push(ev);
        return false;
      }
      return tache !== null && ev.jobId === tache;
    },
    /** Fin de tâche, échec du lancement ou changement d'outil. */
    terminer() {
      tache = null;
      attente = false;
      tampon = [];
      ticket++;
    },
  };
}

export const FINALES = new Set(["done", "error", "cancelled"]);
