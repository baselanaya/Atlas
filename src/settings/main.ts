// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Claude Code hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, onEvent, type HookStatus } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { STR } from "../core/strings";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Coding agents ─────────────────────────────────────────────────────────────

/** How each agent's config file is labelled in the rows below. */
const CONFIG_LABEL: Record<string, string> = {
  claude: "settings.json",
  zcode: "config.json",
  codex: "hooks.json",
};

function agentSection(status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, statusDot(status.installed), h("span", { text: status.name })),
    body,
  );

  const rebuild = async () => {
    const fresh = await Bridge.hooksStatus(status.agent);
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const head = section.querySelector("h2")!;
    clear(head);
    head.append(statusDot(status.installed), h("span", { text: status.name }));
  };

  function draw() {
    body.append(
      h("div", {
        class: "hint",
        text: status.installed
          ? `Atlas is hooked into your ${status.name} sessions. Tool calls, questions and permission requests show up in the island, and you can answer them there.`
          : `Install the hooks to see your ${status.name} sessions in the island and approve permissions without leaving what you are doing.`,
      }),
      h("div", { class: "hint", text: status.blurb }),
      h("div", { class: "row" },
        h("label", { text: CONFIG_LABEL[status.agent] ?? "config" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: "Relay" }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );

    if (!status.hookReady) {
      const relay = status.hookPath.split(/[\\/]/).pop() ?? "atlas-hook";
      body.append(h("div", {
        class: "notice warn",
        text: `${relay} is not in place yet. Restart Atlas; if it still fails, build it with \`cargo build -p atlas-hook\`.`,
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: status.installed ? "Reinstall hooks…" : "Install hooks…",
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every agent session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = "The relay isn't installed yet.";
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", {
        class: "danger",
        text: "Uninstall hooks…",
        onclick: () => showPreview(false),
      }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await Bridge.hooksPreview(status.agent, install);
    } catch (err) {
      // An unreadable or invalid config stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", {
          text: "Back",
          onclick: () => { clear(body); draw(); },
        })),
      );
      return;
    }
    if (!preview) return;
    clear(body);
    body.append(
      h("div", {
        class: "hint",
        text: install
          ? `This is exactly what will change in your ${CONFIG_LABEL[status.agent] ?? "config"}. Your own hooks are left untouched.`
          : "This removes Atlas's entries only. Your own hooks are left untouched.",
      }),
      renderDiff(preview.diff),
      h("div", { class: "row" },
        h("span", { class: "path", text: `Backup → ${preview.backup}` }),
      ),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? "Back up and write" : "Back up and remove",
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(status.agent, install, preview.fingerprint);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: `Done. Previous settings saved as ${backup}.${
            status.agent === "codex"
              ? " Now run /hooks inside Codex once to trust Atlas."
              : ` Open a new ${status.name} session to pick the hooks up.`
          }`,
        }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: `Could not write: ${String(err)}` }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: "Cancel",
      onclick: () => { clear(body); draw(); },
    })));
  }

  draw();
  return section;
}

// ── Chat API section ──────────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
  ["glm-5.3", "GLM 5.3 (Z.AI)"],
  ["glm-5.3-air", "GLM 5.3 Air (Z.AI)"],
];

/** Bases the chat knows how to talk to. The select stays editable — any
 * Anthropic-compatible base works — these are just the two it ships with. */
const API_BASES: [string, string][] = [
  ["https://api.anthropic.com", "Anthropic"],
  ["https://api.z.ai/api/anthropic", "Z.AI (GLM)"],
];

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: hasKey ? "Key saved in the system keyring." : "No key yet — the chat needs one." });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? "••••••••••••  (stored)" : "sk-…",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: "Save key" });
  const clearBtn = h("button", { class: "danger", text: "Remove" });
  const feedback = h("div", {});

  // Who answers: the API key above, or a logged-in CLI on the user's own plan.
  const route = h("select", {}) as HTMLSelectElement;
  const ROUTES: [string, string][] = [
    ["api", "Direct API (key above)"],
    ["codex", "Codex (your ChatGPT login)"],
    ["claude", "Claude Code (your Anthropic login)"],
    ["ollama", "Ollama (local, model above)"],
  ];
  for (const [id, label] of ROUTES) route.append(h("option", { value: id, text: label }));
  if (!ROUTES.some(([id]) => id === settings.chatRoute)) {
    route.append(h("option", { value: settings.chatRoute, text: settings.chatRoute }));
  }
  route.value = settings.chatRoute;
  route.addEventListener("change", () => {
    settings.chatRoute = route.value as typeof settings.chatRoute;
    void save();
  });

  async function refresh() {
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present
      ? "Key saved in the system keyring."
      : "No key yet — the chat needs one.";
    field.placeholder = present ? "••••••••••••  (stored)" : "sk-…";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("anthropic-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Saved. It never touches disk." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not save: ${String(err)}` }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("anthropic-api-key");
      feedback.append(h("div", { class: "notice ok", text: "Key removed." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not remove: ${String(err)}` }));
    }
  });

  // A combo, not a list: API models are known, Ollama's are whatever the
  // user has pulled — typing must work.
  const model = h("input", {
    type: "text",
    style: "flex:1 1 auto;min-width:0",
    list: "model-names",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;
  const modelList = h("datalist", { id: "model-names" });
  for (const [id] of MODELS) modelList.append(h("option", { value: id }));
  for (const id of ["llama3.1", "qwen3", "gemma3", "mistral"]) {
    modelList.append(h("option", { value: id }));
  }
  model.value = settings.model;
  model.addEventListener("change", () => {
    settings.model = model.value.trim();
    model.value = settings.model;
    void save();
  });

  // The chat speaks the Anthropic Messages API wherever it is served. Picking
  // Z.AI points it at GLM; the GLM models above are the ones worth listing.
  const base = h("input", {
    type: "text",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
    list: "api-bases",
  }) as HTMLInputElement;
  base.value = settings.apiBase;
  const datalist = h("datalist", { id: "api-bases" });
  for (const [url] of API_BASES) datalist.append(h("option", { value: url }));
  base.addEventListener("change", () => {
    const value = base.value.trim().replace(/\/+$/, "");
    if (!value) return;
    settings.apiBase = value;
    base.value = value;
    void save();
  });

  clearBtn.style.display = hasKey ? "" : "none";

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: STR.chatSection })),
    state,
    h("div", { class: "row" }, h("label", { text: STR.apiKey }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: STR.apiBase }), base, datalist),
    h("div", { class: "row" }, h("label", { text: STR.model }), model, modelList),
    h("div", { class: "row" }, h("label", { text: STR.answers }), route),
    h("div", { class: "hint", text: STR.cliRouteHint }),
    feedback,
  );
}

// ── Voice section (Voicebox) ──────────────────────────────────────────────────

function voiceSection(): HTMLElement {
  const dot = statusDot(false);
  const state = h("span", { class: "hint", text: STR.voiceOff });
  const profile = h("select", {}) as HTMLSelectElement;
  profile.style.flex = "1";

  async function refresh() {
    const status = await Bridge.voiceStatus();
    dot.style.background = status?.available ? "#22c55e" : "#f4505e";
    state.textContent = status?.available ? STR.voiceHint : STR.voiceOff;
    clear(profile);
    profile.append(h("option", { value: "", text: "Default voice" }));
    for (const name of status?.profiles ?? []) {
      profile.append(h("option", { value: name, text: name }));
    }
    profile.value = settings.voiceProfile || "";
    if (profile.value !== settings.voiceProfile) profile.value = "";
  }

  const enabled = toggle(settings.voiceEnabled, (on) => {
    settings.voiceEnabled = on;
    void save();
  });
  const speakChat = toggle(settings.voiceSpeakChat, (on) => {
    settings.voiceSpeakChat = on;
    void save();
  });
  const speakEvents = toggle(settings.voiceSpeakEvents, (on) => {
    settings.voiceSpeakEvents = on;
    void save();
  });
  profile.addEventListener("change", () => {
    settings.voiceProfile = profile.value;
    void save();
  });

  void refresh();

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: STR.voice })),
    state,
    h("div", { class: "row" }, h("label", { text: "Enabled" }), enabled),
    h("div", { class: "row" }, h("label", { text: "Voice" }), profile),
    h("div", { class: "row" }, h("label", { text: STR.speakChat }), speakChat),
    h("div", { class: "row" }, h("label", { text: STR.speakEvents }), speakEvents),
    h("div", { class: "hint", text: "github.com/jamiepine/voicebox — local, open source, MIT." }),
  );
}

// ── MCP-out section ────────────────────────────────────────────────────────────

function mcpSection(): HTMLElement {
  const dot = statusDot(false);
  const state = h("span", { class: "hint" });
  const port = h("input", {
    type: "number",
    min: "1024",
    max: "65535",
    style: "width:90px",
    value: String(settings.mcpPort),
  }) as HTMLInputElement;
  const endpoint = h("span", { class: "path" });

  async function refresh() {
    const status = await Bridge.mcpStatus();
    dot.style.background = status?.running ? "#22c55e" : "#8e939c";
    state.textContent = status?.running
      ? STR.mcpEndpoint(status.port)
      : settings.mcpEnabled ? "Restart Atlas to serve." : STR.mcpHint;
    endpoint.textContent = status?.running ? STR.mcpEndpoint(status.port) : "";
  }

  const enabled = toggle(settings.mcpEnabled, (on) => {
    settings.mcpEnabled = on;
    void save();
    void refresh();
  });
  port.addEventListener("change", () => {
    settings.mcpPort = Number(port.value) || 17510;
    void save();
    void refresh();
  });

  void refresh();
  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: STR.mcp })),
    state,
    h("div", { class: "row" }, h("label", { text: "Enabled" }), enabled,
      h("label", { text: "Port" }), port),
    h("div", { class: "row" }, h("label", { text: "Endpoint" }), endpoint),
    h("div", { class: "hint", text: STR.mcpHint }),
  );
}

// ── Notifications section ──────────────────────────────────────────────────────

function notificationsSection(): HTMLElement {
  const enabled = toggle(settings.notifyEnabled, (on) => {
    settings.notifyEnabled = on;
    void save();
  });
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: STR.notifications })),
    h("div", { class: "row" }, h("label", { text: "Enabled" }), enabled),
    h("div", { class: "hint", text: STR.notifyHint }),
  );
}

// ── Stats section ──────────────────────────────────────────────────────────────

function statsSection(): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
  void (async () => {
    const snap = await Bridge.statsSnapshot(7);
    if (!snap) return;
    const days = Object.entries(snap as Record<string, unknown>)
      .sort((a, b) => b[0].localeCompare(a[0]));
    for (const [day, entry] of days.slice(0, 7)) {
      const agents = ((entry as { agents?: Record<string, Record<string, number>> }).agents) ?? {};
      const cells = Object.entries(agents).map(([agent, s]) =>
        `${agent}: ${s.sessions ?? 0} ${STR.sessions}, ${s.tool_calls ?? 0} ${STR.tools}, ${s.approvals ?? 0} ${STR.approvals}`);
      body.append(h("div", { class: "row" },
        h("label", { text: day }),
        h("span", { class: "hint", text: cells.join(" · ") || "—" }),
      ));
    }
    if (days.length === 0) body.append(h("div", { class: "hint", text: "—" }));
  })();
  return h("section", {}, h("h2", {}, h("span", { text: STR.stats })), body);
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Pick up to ${MAX_ACTIVE} pills to show next to Atlas — ${used}/${MAX_ACTIVE} in use. Keys are stored in the Windows Credential Manager, never on disk.`;
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: "Save" });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: field.label }),
          input, saveBtn, dotEl,
        ),
      );
    }

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: "Integrations" })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Main display" }),
    h("option", { value: "cursor", text: "Display under the cursor" }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" },
      h("label", { text: "Sound" }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: "Auto-close" }),
      autoClose,
      h("span", { class: "hint", text: "seconds after you leave the island" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Island lives on" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  const statuses = (await Bridge.hooksStatuses()) ?? [];

  const hasKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Atlas" }), h("span", { class: "version", text: version })),
    ...statuses.map(agentSection),
    apiSection(hasKey),
    voiceSection(),
    mcpSection(),
    notificationsSection(),
    statsSection(),
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: "No telemetry. Network requests only go to the services you configure yourself.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();
