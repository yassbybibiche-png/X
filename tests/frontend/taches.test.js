// Tests du suivi de tâche du frontend : `node --test tests/frontend/*.test.js`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { creerSuivi } from "../../src/taches.js";

const ev = (jobId, kind, message = "") => ({ jobId, kind, message, percent: null, outputs: [] });

// Régression v0.1.0 (fusion PDF, ZIP) : tous les événements, « done » compris,
// arrivent avant que `run_tool` ait rendu l'identifiant de tâche.
test("tâche rapide : événements reçus avant l'identifiant rejoués à l'association", () => {
  const s = creerSuivi();
  const ticket = s.demarrer();
  assert.equal(s.recevoir(ev(7, "progress")), false);
  assert.equal(s.recevoir(ev(7, "done")), false);
  const recus = s.associer(ticket, 7);
  assert.deepEqual(
    recus.map((e) => e.kind),
    ["progress", "done"],
  );
  assert.equal(s.tache, 7);
});

test("tâche lente : événements reçus après l'identifiant transmis directement", () => {
  const s = creerSuivi();
  const ticket = s.demarrer();
  assert.deepEqual(s.associer(ticket, 3), []);
  assert.equal(s.recevoir(ev(3, "log")), true);
  assert.equal(s.recevoir(ev(4, "log")), false, "autre tâche ignorée");
});

test("événements d'une autre tâche écartés du tampon", () => {
  const s = creerSuivi();
  const ticket = s.demarrer();
  s.recevoir(ev(1, "done"));
  s.recevoir(ev(2, "log"));
  assert.deepEqual(
    s.associer(ticket, 2).map((e) => e.jobId),
    [2],
  );
});

test("après la fin, plus rien n'est transmis (Lancer réactivable)", () => {
  const s = creerSuivi();
  s.associer(s.demarrer(), 5);
  s.terminer();
  assert.equal(s.tache, null);
  assert.equal(s.recevoir(ev(5, "log")), false);
});

test("changement d'outil pendant le lancement : association abandonnée", () => {
  const s = creerSuivi();
  const ticket = s.demarrer();
  s.recevoir(ev(9, "done"));
  s.terminer();
  assert.equal(s.associer(ticket, 9), null);
  assert.equal(s.tache, null);
});

test("relance : le tampon du lancement précédent ne fuit pas", () => {
  const s = creerSuivi();
  s.demarrer();
  s.recevoir(ev(1, "log"));
  const t2 = s.demarrer();
  assert.deepEqual(s.associer(t2, 1), []);
});
