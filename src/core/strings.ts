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
  voice: fr ? "Voix" : "Voice",
  voiceOff: fr
    ? "Voicebox n'est pas détecté — lancez-le, puis revenez ici."
    : "Voicebox isn't detected — start it and come back here.",
  voiceHint: fr
    ? "Voicebox (open source, local) prête la voix ; Atlas décide quand parler."
    : "Voicebox (open source, local) provides the voice; Atlas decides when to speak.",
  speakChat: fr ? "Lire les réponses" : "Speak chat replies",
  speakEvents: fr ? "Annoncer approbations et fins" : "Announce approvals and finishes",
  micTitle: fr ? "Dicter (Voicebox)" : "Dictate (Voicebox)",
  micRecording: fr ? "Clic pour arrêter" : "Click to stop",
  mcp: "MCP",
  mcpHint: fr
    ? "Expose Atlas comme serveur MCP local — d'autres outils peuvent suivre vos agents et répondre à leurs demandes."
    : "Expose Atlas as a local MCP server — other tools can watch your agents and answer their requests.",
  mcpEndpoint: (port: number) => `http://127.0.0.1:${port}/mcp`,
  notifications: fr ? "Notifications" : "Notifications",
  notifyHint: fr
    ? "Notifications système pour les demandes d'approbation."
    : "System notifications for approval requests.",
  stats: fr ? "Statistiques" : "Stats",
  sessions: fr ? "sessions" : "sessions",
  tools: fr ? "outils" : "tools",
  approvals: fr ? "approbations" : "approvals",
  rateWindow: fr ? "Fenêtre 5h" : "5h window",
  rateWarn: fr ? "approche de la limite" : "approaching limit",
} as const;
