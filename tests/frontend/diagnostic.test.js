// Tests des libellés du Diagnostic : `node --test tests/frontend/*.test.js`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { modeBinaire } from "../../src/diagnostic.js";

// Régression v0.1.0 : FFmpeg trouvé dans le PATH affiché « téléchargé au premier usage ».
test("mode d'un binaire trouvé : d'après sa source réelle", () => {
  const b = { delivery: "first-use", state: "present" };
  assert.equal(modeBinaire({ ...b, source: "system-path" }), "installé sur le système (PATH)");
  assert.equal(modeBinaire({ ...b, source: "app-data" }), "téléchargé au premier usage");
  assert.equal(modeBinaire({ ...b, delivery: "bundled", source: "bundled" }), "inclus dans l'installeur");
  // Cassé mais trouvé : la source reste celle du binaire trouvé.
  assert.equal(modeBinaire({ ...b, state: "casse", source: "system-path" }), "installé sur le système (PATH)");
});

test("mode d'un binaire absent : d'après ce qui est possible ici", () => {
  assert.equal(modeBinaire({ delivery: "first-use", state: "telechargeable" }), "téléchargé au premier usage");
  assert.equal(modeBinaire({ delivery: "first-use", state: "absent" }), "à installer sur le système");
  assert.equal(modeBinaire({ delivery: "bundled", state: "absent" }), "inclus dans l'installeur");
});
