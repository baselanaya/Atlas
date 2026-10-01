// The strings Atlas authors itself, in one place, so a translation is a file
// edit instead of a hunt. The island's older inherited strings are still
// inline; pull them in here as they get touched.

const fr = navigator.language.startsWith("fr");

export const STR = {
  chatSection: fr ? "Discussion" : "Chat",
  apiKey: fr ? "Clé API" : "API key",
  apiBase: fr ? "Point d'accès" : "API base",
  model: fr ? "Modèle" : "Model",
  answers: fr ? "Répond par" : "Answers",
  cliRouteHint: fr
    ? "Les agents CLI répondent via leur propre session — plus lent, sans clé ni facturation."
    : "CLI routes answer through the agent's own login — slower, but no API key and no extra billing.",
  access: fr ? "Accès" : "Access",
  ask: fr ? "Demander" : "Ask",
  auto: "Auto",
  root: fr ? "Total" : "Root",
  newSessionsOnly: fr ? "Nouvelles sessions seulement" : "New sessions only",
  hookedIn: fr ? "Branché · en attente de sessions" : "Hooked in · waiting for sessions",
  notHooked: fr ? "Non branché" : "Not hooked",
  keyMissing: fr ? "Clé non configurée" : "Key not configured",
  connected: fr ? "Connecté · chargement…" : "Connected · loading…",
} as const;
