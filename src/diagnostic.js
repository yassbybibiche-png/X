// Libellés de l'onglet Diagnostic (sans dépendance à Tauri, testables sous node).

const MODES_SOURCE = {
  bundled: "inclus dans l'installeur",
  "app-data": "téléchargé au premier usage",
  "system-path": "installé sur le système (PATH)",
};

// Mode fidèle à la source réelle : v0.1.0 affichait « téléchargé au premier
// usage » (mode prévu) même pour un binaire trouvé dans le PATH.
export function modeBinaire(b) {
  if (b.source) return MODES_SOURCE[b.source] ?? "—";
  if (b.delivery === "bundled") return MODES_SOURCE.bundled;
  if (b.state === "telechargeable") return MODES_SOURCE["app-data"];
  return "à installer sur le système";
}
