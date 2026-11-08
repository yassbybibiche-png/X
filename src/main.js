// Boîte à outils — interface (vanilla JS, aucune dépendance).
// Toutes les données affichées passent par textContent (jamais innerHTML).

const tauri = window.__TAURI__;
const invoke = tauri.core.invoke;
const listen = tauri.event.listen;

const etat = {
  catalogue: { categories: [], tools: [] },
  binaires: new Map(), // id -> BinaryStatus
  categorie: null,
  outil: null,
  fichiers: [],
  tache: null,
  recherche: "",
  telechargements: new Map(), // jobId -> { binaire, termine, total, message }
};

const PHASES = { phase1: "Disponible", phase2: "Prévu en phase 2", phase3: "Prévu en phase 3" };
const ORIGINES = { bundled: "incluse", "app-data": "données de l'app", "system-path": "PATH système" };
const MODES = { bundled: "inclus dans l'installeur", "first-use": "téléchargé au premier usage" };
const ETATS = { present: "Présent", telechargeable: "Absent — téléchargeable", absent: "Absent" };

function taille(octets) {
  if (!octets) return "";
  return octets >= 1e6 ? (octets / 1e6).toFixed(0) + " Mo" : Math.ceil(octets / 1e3) + " ko";
}

const $ = (sel, racine = document) => racine.querySelector(sel);

function el(tag, texte, classe) {
  const n = document.createElement(tag);
  if (texte !== undefined) n.textContent = texte;
  if (classe) n.className = classe;
  return n;
}

function nomFichier(chemin) {
  return chemin.split(/[\\/]/).pop();
}

// ---------- Disponibilité ----------

function binairesManquants(outil) {
  return outil.binaries.filter((b) => !etat.binaires.get(b)?.present);
}

function raisonIndisponible(outil) {
  if (outil.status !== "phase1") return PHASES[outil.status];
  const manquants = binairesManquants(outil);
  if (etat.binaires.size && manquants.length) {
    return "Binaire manquant : " + manquants.map((b) => etat.binaires.get(b)?.label ?? b).join(", ");
  }
  return null;
}

// ---------- Catégories et liste ----------

function afficherCategories() {
  const ul = $("#categories");
  ul.replaceChildren();
  for (const c of etat.catalogue.categories) {
    const li = el("li");
    const b = el("button", `${c.label} (${c.count})`, c.id === etat.categorie ? "active" : "");
    b.type = "button";
    b.addEventListener("click", () => {
      etat.categorie = c.id;
      afficherCategories();
      afficherOutils();
    });
    li.append(b);
    ul.append(li);
  }
}

function outilsVisibles() {
  const q = etat.recherche.trim().toLowerCase();
  return etat.catalogue.tools.filter((t) =>
    q ? (t.name + " " + t.description + " " + t.id).toLowerCase().includes(q) : t.category === etat.categorie,
  );
}

function afficherOutils() {
  const ul = $("#outils");
  ul.replaceChildren();
  for (const t of outilsVisibles()) {
    const raison = raisonIndisponible(t);
    const li = el("li");
    const b = el("button", undefined, "outil" + (raison ? " grise" : "") + (etat.outil?.id === t.id ? " active" : ""));
    b.type = "button";
    b.append(el("span", t.name, "nom"));
    if (raison) b.append(el("span", raison, "badge"));
    b.addEventListener("click", () => choisirOutil(t));
    li.append(b);
    ul.append(li);
  }
}

// ---------- Panneau outil ----------

function choisirOutil(outil) {
  etat.outil = outil;
  etat.fichiers = [];
  etat.tache = null;
  afficherOutils();
  const panneau = $("#panneau");
  panneau.replaceChildren($("#tpl-panneau").content.cloneNode(true));
  $(".titre", panneau).textContent = outil.name;
  $(".description", panneau).textContent = outil.description;
  $(".sortie", panneau).textContent = "Sortie : " + outil.output + " (à côté de l'original, jamais d'écrasement)";
  const raison = raisonIndisponible(outil);
  const statut = $(".statut", panneau);
  statut.textContent = raison ?? "Disponible";
  statut.className = "statut " + (raison ? "indisponible" : "disponible");
  $(".choisir", panneau).addEventListener("click", choisirFichiers);
  $(".lancer", panneau).addEventListener("click", lancer);
  $(".annuler", panneau).addEventListener("click", annuler);
  construireOptions($(".options", panneau), outil.options);
  afficherFichiers();
}

function construireOptions(form, options) {
  form.replaceChildren();
  for (const o of options) {
    const label = el("label", o.label);
    label.append(champOption(o));
    form.append(label);
  }
}

function champOption(o) {
  const k = o.kind;
  let champ;
  if (k.type === "choice") {
    champ = document.createElement("select");
    for (const c of k.choices) {
      const opt = el("option", c.label);
      opt.value = c.value;
      champ.append(opt);
    }
  } else {
    champ = document.createElement("input");
    champ.type = { number: "number", password: "password", bool: "checkbox" }[k.type] ?? "text";
    if (k.type === "number") Object.assign(champ, { min: k.min, max: k.max, step: k.step });
  }
  champ.name = o.key;
  if (k.type === "bool") champ.checked = o.default === "true";
  else champ.value = o.default;
  return champ;
}

function lireOptions() {
  const valeurs = {};
  for (const champ of $("#panneau .options").elements) {
    valeurs[champ.name] = champ.type === "checkbox" ? String(champ.checked) : champ.value;
  }
  return valeurs;
}

function afficherFichiers() {
  const ul = $("#panneau .fichiers");
  if (!ul) return;
  ul.replaceChildren();
  etat.fichiers.forEach((f, i) => {
    const li = el("li", nomFichier(f));
    li.title = f;
    const retirer = el("button", "Retirer", "petit");
    retirer.type = "button";
    retirer.addEventListener("click", () => {
      etat.fichiers.splice(i, 1);
      afficherFichiers();
    });
    li.append(retirer);
    ul.append(li);
  });
  const outil = etat.outil;
  $("#panneau .lancer").disabled = !outil || raisonIndisponible(outil) !== null || etat.fichiers.length < outil.input.minFiles;
}

function ajouterFichiers(chemins) {
  if (!etat.outil) return;
  for (const c of chemins) if (!etat.fichiers.includes(c)) etat.fichiers.push(c);
  if (!etat.outil.input.multiple) etat.fichiers = etat.fichiers.slice(-1);
  afficherFichiers();
}

async function choisirFichiers() {
  const options = { multiple: !!etat.outil?.input.multiple, directory: false };
  const res = tauri.dialog ? await tauri.dialog.open(options) : await invoke("plugin:dialog|open", { options });
  if (res) ajouterFichiers(Array.isArray(res) ? res : [res]);
}

// ---------- Exécution et journal ----------

function journal(message, type = "log") {
  const ol = $("#panneau .journal");
  if (!ol) return;
  ol.append(el("li", message, type));
  ol.scrollTop = ol.scrollHeight;
}

function avertir(message) {
  const bloc = $("#panneau .avertissements");
  if (!bloc) return;
  bloc.hidden = false;
  $("ul", bloc).append(el("li", message));
}

function enCours(actif) {
  const annuler = $("#panneau .annuler");
  if (annuler) {
    annuler.hidden = !actif;
    annuler.disabled = false;
  }
}

async function annuler() {
  if (etat.tache === null) return;
  $("#panneau .annuler").disabled = true;
  journal("Annulation demandée…");
  try {
    await invoke("cancel_job", { jobId: etat.tache });
  } catch (e) {
    journal(String(e), "error");
  }
}

function progression(pourcent) {
  const bloc = $("#panneau .progression");
  if (!bloc) return;
  bloc.hidden = false;
  $("progress", bloc).value = pourcent;
  $(".pourcent", bloc).textContent = Math.round(pourcent) + " %";
}

async function lancer() {
  const outil = etat.outil;
  $("#panneau .lancer").disabled = true;
  progression(0);
  const bloc = $("#panneau .avertissements");
  bloc.hidden = true;
  $("ul", bloc).replaceChildren();
  journal(`Lancement de « ${outil.name} » sur ${etat.fichiers.length} fichier(s)…`);
  try {
    etat.tache = await invoke("run_tool", { toolId: outil.id, inputs: etat.fichiers, options: lireOptions() });
    enCours(true);
  } catch (e) {
    journal(String(e), "error");
    afficherFichiers();
  }
}

function surEvenement({ payload: ev }) {
  if (ev.jobId !== etat.tache) return;
  if (ev.percent !== null) progression(ev.percent);
  if (ev.kind === "progress" || ev.kind === "log") journal(ev.message);
  else if (ev.kind === "warning") {
    journal("⚠ " + ev.message, "warning");
    avertir(ev.message);
  } else if (ev.kind === "cancelled") journal(ev.message, "warning");
  else if (ev.kind === "error") journal("Erreur : " + ev.message, "error");
  else if (ev.kind === "done") {
    journal(ev.message, "done");
    for (const o of ev.outputs) journal("→ " + o, "done");
  }
  if (ev.kind === "done" || ev.kind === "error" || ev.kind === "cancelled") {
    etat.tache = null;
    enCours(false);
    afficherFichiers();
  }
}

// ---------- Diagnostic ----------

async function diagnostic() {
  const tbody = $("#binaires");
  const message = (texte) => {
    const tr = el("tr");
    tr.append(el("td", texte));
    tbody.replaceChildren(tr);
  };
  message("Analyse en cours…");
  try {
    const liste = await invoke("check_tools");
    etat.binaires = new Map(liste.map((b) => [b.id, b]));
    tbody.replaceChildren(...liste.map(ligneBinaire));
  } catch (e) {
    message("Erreur : " + e);
  }
  afficherOutils();
  if (etat.outil) choisirOutil(etat.outil);
}

function ligneBinaire(b) {
  const tr = el("tr", undefined, b.state);
  const cellules = [
    b.label,
    ETATS[b.state] ?? (b.present ? "Présent" : "Absent"),
    b.version ?? "—",
    b.source ? ORIGINES[b.source] : "—",
    b.path ?? "—",
    b.license,
    MODES[b.delivery],
  ];
  for (const c of cellules) tr.append(el("td", c));
  tr.append(celluleAction(b));
  return tr;
}

function messageDiagnostic(texte, classe = "") {
  const p = $("#diag-message");
  p.textContent = texte;
  p.className = "aide " + classe;
}

function telechargementDe(binaire) {
  for (const [id, t] of etat.telechargements) if (t.binaire === binaire) return [id, t];
  return null;
}

function celluleAction(b) {
  const td = el("td");
  const encours = telechargementDe(b.id);
  if (encours) {
    const [jobId, t] = encours;
    const bloc = el("div", undefined, "telechargement");
    const barre = document.createElement("progress");
    barre.max = t.total || 1;
    barre.value = t.termine;
    const annulerDl = el("button", "Annuler", "petit");
    annulerDl.type = "button";
    annulerDl.addEventListener("click", () => invoke("cancel_job", { jobId }).catch((e) => messageDiagnostic(String(e), "error")));
    bloc.append(barre, el("span", t.message || taille(t.termine) + " / " + taille(t.total)), annulerDl);
    td.append(bloc);
  } else if (b.state === "telechargeable") {
    const bouton = el("button", `Télécharger ${b.pinnedVersion ?? ""} (${taille(b.downloadSize)})`, "petit");
    bouton.type = "button";
    bouton.title = `Licence : ${b.license}. Téléchargement depuis une source épinglée, empreinte SHA-256 vérifiée.`;
    bouton.addEventListener("click", () => telecharger(b));
    td.append(bouton);
  } else if (b.state === "absent") {
    td.textContent = b.unavailableReason ?? "Non disponible sur ce système";
  } else {
    td.textContent = "—";
  }
  return td;
}

async function telecharger(b) {
  try {
    const jobId = await invoke("download_binary", { binary: b.id });
    etat.telechargements.set(jobId, { binaire: b.id, termine: 0, total: b.downloadSize ?? 0, message: "" });
    rafraichirDiagnostic();
  } catch (e) {
    messageDiagnostic(`Téléchargement de ${b.label} impossible : ${e}`, "error");
  }
}

function rafraichirDiagnostic() {
  $("#binaires").replaceChildren(...[...etat.binaires.values()].map(ligneBinaire));
}

function surTelechargement({ payload: ev }) {
  const t = etat.telechargements.get(ev.jobId);
  if (!t) return;
  if (ev.kind === "progress") {
    t.termine = ev.done;
    t.total = ev.total;
    t.message = "";
  } else if (ev.kind === "log") {
    t.message = ev.message;
  } else {
    etat.telechargements.delete(ev.jobId);
    const echec = ev.kind === "error";
    messageDiagnostic(echec ? `Échec du téléchargement : ${ev.message}` : ev.message, echec ? "error" : "");
    diagnostic();
    return;
  }
  rafraichirDiagnostic();
}

// ---------- Glisser-déposer ----------

async function activerDepot() {
  const surDepot = (type, chemins) => {
    const zone = $("#panneau .depot");
    zone?.classList.toggle("survol", type === "over" || type === "enter");
    if (type === "drop") ajouterFichiers(chemins ?? []);
  };
  const webview = tauri.webview?.getCurrentWebview?.();
  if (webview?.onDragDropEvent) {
    await webview.onDragDropEvent((e) => surDepot(e.payload.type, e.payload.paths));
  } else {
    await listen("tauri://drag-drop", (e) => surDepot("drop", e.payload.paths));
  }
}

// ---------- Démarrage ----------

function vues() {
  for (const onglet of document.querySelectorAll(".tab")) {
    onglet.addEventListener("click", () => {
      for (const o of document.querySelectorAll(".tab")) o.classList.toggle("active", o === onglet);
      $("#vue-outils").hidden = onglet.dataset.view !== "outils";
      $("#vue-diagnostic").hidden = onglet.dataset.view !== "diagnostic";
    });
  }
  $("#rafraichir").addEventListener("click", diagnostic);
  $("#recherche").addEventListener("input", (e) => {
    etat.recherche = e.target.value;
    afficherOutils();
  });
}

async function demarrer() {
  vues();
  etat.catalogue = await invoke("list_tools");
  etat.categorie = etat.catalogue.categories[0]?.id ?? null;
  afficherCategories();
  afficherOutils();
  await listen("job-event", surEvenement);
  await listen("download-event", surTelechargement);
  await activerDepot();
  diagnostic();
}

demarrer().catch((e) => {
  document.body.append(el("p", "Erreur au démarrage : " + e, "error"));
});
