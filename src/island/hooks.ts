// Claude Code hook events → island state.
// Port of HookServer.processEvent / processPermissionRequest from the macOS app.
// Difference from macOS: no terminal filter. On Windows the hook fires from any
// terminal (Windows Terminal, VS Code, PowerShell…) and all of them are handled.

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { Island } from "./island";

const CLAUDE_ID = "integration_claude";

/** Which pill each harness reports to. */
const TASK_FOR_AGENT: Record<string, string> = {
  claude: "integration_claude",
  zcode: "integration_zcode",
  codex: "integration_codex",
};

/** Base pill names, restored when a session ends. */
const BASE_NAMES: Record<string, string> = {
  integration_claude: "Claude",
  integration_zcode: "ZCode",
  integration_codex: "Codex",
};

/** Clears the approval card if no decision was made before the hook gave up. */
let pendingTimeout: number | null = null;

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  /** Which harness fired: "claude" (default), "zcode" or "codex". */
  agent?: string;
  cwd?: string;
  message?: string;
  /** UserPromptSubmit carries `prompt`; `message` belongs to Notification/Stop. */
  prompt?: string;
  /** Codex's Stop carries the final answer here instead of `message`. */
  last_assistant_message?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
}

/** Short tag for steps and the approval card when the event isn't Claude's. */
function agentTag(agent: string | undefined): string {
  return agent && agent !== "claude" ? agent : "";
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

const TOOL_LABELS: Record<string, string> = {
  Bash: "Runs",
  Read: "Reads",
  Write: "Writes",
  Edit: "Edits",
  Glob: "Finds",
  Grep: "Greps",
  WebSearch: "Searches web",
  WebFetch: "Fetches",
  TodoWrite: "Todos",
  Task: "Agent",
  Agent: "Agent",
  LS: "Lists",
  MultiEdit: "Edits",
  NotebookEdit: "Notebook",
  PowerShell: "Runs",
  Shell: "Runs",
  apply_patch: "Patches",
  Exit: "Runs",
};

function stepLabel(tool: string, input: Record<string, unknown>): string {
  const label = TOOL_LABELS[tool] ?? tool;
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  const cmd = str("command");
  if (cmd) return `${label} · ${cmd.slice(0, 40)}`;
  const path = str("path");
  if (path) return `${label} · ${lastPathComponent(path)}`;
  const file = str("file_path");
  if (file) return `${label} · ${lastPathComponent(file)}`;
  const query = str("query");
  if (query) return `${label} · ${query.slice(0, 40)}`;
  return label;
}

/**
 * What the Allow button actually authorises. Approving "Write" tells you nothing
 * — approving `Write · C:\…\.env` tells you everything, and the difference is
 * the whole point of approving from the island rather than blind.
 *
 * Ordered by how specific the field is, so an unfamiliar tool still shows
 * whatever identifying string it carries instead of falling back to its name.
 */
const APPROVAL_FIELDS = [
  "command", // Bash, PowerShell
  "file_path", // Write, Edit, MultiEdit, NotebookEdit
  "path", // Read, LS
  "url", // WebFetch
  "query", // WebSearch
  "pattern", // Glob, Grep
  "prompt", // Task
] as const;

function approvalTarget(tool: string, input: Record<string, unknown>): string {
  for (const field of APPROVAL_FIELDS) {
    const value = input[field];
    if (typeof value === "string" && value.trim()) {
      return `${tool} · ${value.trim()}`;
    }
  }
  return tool;
}

function upsert(taskId: string, projectName: string, cwd: string, agent: string) {
  const t = State.tasks.find((x) => x.id === taskId);
  if (!t) return;
  t.name = projectName;
  if (cwd) t.sessionCwd = cwd;
  t.agent = agent;
}

function clearSession() {
  for (const id of Object.keys(BASE_NAMES)) {
    const t = State.tasks.find((x) => x.id === id);
    if (!t) continue;
    t.steps = [];
    t.stepIndex = 0;
    t.name = BASE_NAMES[id];
    t.agent = null;
    t.pillBadge = null;
  }
}

export function registerHookHandlers(island: Island) {
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  if (State.paused) {
    // Silence here used to cost Claude Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately.
    if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
    return;
  }

  const name = payload.hook_event_name ?? "";
  const cwd = payload.cwd ?? "";
  const raw = lastPathComponent(cwd);
  const projectName = raw || "Session";
  // When two harnesses run at once their pills keep their own timelines; the
  // tag marks the shared surfaces (the approval card).
  const tag = agentTag(payload.agent);
  const tagged = (step: string) => (tag ? `${tag} · ${step}` : step);
  const agentName = tag || "Claude Code";
  // Every state change below lands on the pill of the harness that fired it,
  // so three agents can work at once without stepping on each other.
  const taskId = TASK_FOR_AGENT[payload.agent ?? "claude"] ?? CLAUDE_ID;
  const focused = State.focusId === taskId;

  /** Alerts force the island open; work events only reveal the compact island. */
  const surface = (view: Parameters<Island["alert"]>[0], isAlert: boolean) => {
    if (State.mode === "expanded") {
      if (isAlert) island.setView(view);
    } else if (isAlert) {
      island.alert(view);
    } else if (State.mode === "hidden") {
      island.reveal();
    }
  };

  switch (name) {
    case "SessionStart":
      upsert(taskId, projectName, cwd, agentName);
      surface("overview", false);
      Sound.play("work");
      break;

    case "UserPromptSubmit": {
      upsert(taskId, projectName, cwd, agentName);
      State.updateTask(taskId, "thinking");
      // The field is `prompt`; reading `message` meant this step was always blank.
      const asked = payload.prompt ?? payload.message;
      if (asked) State.appendStep(taskId, tagged(asked.slice(0, 60)));
      surface("overview", false);
      break;
    }

    case "PreToolUse": {
      upsert(taskId, projectName, cwd, agentName);
      State.updateTask(taskId, "working");
      const tool = payload.tool_name ?? "Tool";
      State.appendStep(taskId, tagged(stepLabel(tool, payload.tool_input ?? {})));
      surface("overview", false);
      break;
    }

    case "PostToolUse":
      State.updateTask(taskId, "working");
      break;

    case "PostToolUseFailure":
      State.updateTask(taskId, "working");
      State.appendStep(taskId, tagged("⚠ failed"));
      break;

    case "Notification": {
      const message = payload.message ?? "";
      const lower = message.toLowerCase();
      if (lower.includes("rate limit")) {
        State.updateTask(taskId, "ratelimit");
        Sound.play("rate");
      } else if (message.endsWith("?")) {
        State.updateTask(taskId, "question");
        State.appendStep(taskId, tagged(message));
      }
      break;
    }

    case "Stop": {
      // zcode and Codex have no Notification event, so the faces Claude gets
      // for free are inferred here from what the turn actually said.
      const said = (payload.message ?? payload.last_assistant_message ?? "").trim();
      const lower = said.toLowerCase();
      if (taskId !== CLAUDE_ID && lower) {
        if (lower.includes("rate limit") || lower.includes("usage limit") || lower.includes("quota")) {
          State.updateTask(taskId, "ratelimit");
          Sound.play("rate");
        } else if (said.endsWith("?")) {
          State.updateTask(taskId, "question");
          State.appendStep(taskId, tagged(said.slice(0, 60)));
        }
      }
      State.updateTask(taskId, "finished");
      if (said) State.appendStep(taskId, tagged(said.slice(0, 60)));
      Sound.play("finish");
      if (focused) surface("finished", true);
      else State.setPillBadge(taskId, "finished");
      window.setTimeout(() => {
        State.updateTask(taskId, "idle");
        State.setPillBadge(taskId, null);
      }, 5200);
      break;
    }

    case "StopFailure":
      State.updateTask(taskId, "error");
      Sound.play("error");
      if (focused) surface("error", true);
      else State.setPillBadge(taskId, "error");
      break;

    case "SessionEnd":
      State.updateTask(taskId, "idle");
      clearSession();
      break;

    case "SubagentStart":
      State.appendStep(taskId, tagged("+ subagent"));
      break;

    case "SubagentStop":
      State.appendStep(taskId, tagged("• subagent done"));
      break;

    case "PermissionRequest": {
      const requestId = payload.request_id ?? "";
      // One card, one request. A second one must never quietly replace the first
      // — that would leave a human staring at request B while request A waits for
      // a decision nobody can give. Hand it straight back to the terminal.
      if (State.pendingApproval && State.pendingApproval.requestId !== requestId) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      upsert(taskId, projectName, cwd, agentName);
      if (pendingTimeout != null) window.clearTimeout(pendingTimeout);
      const tool = payload.tool_name ?? "Tool";
      const input = payload.tool_input ?? {};
      State.pendingApproval = {
        requestId,
        sessionId: payload.session_id ?? "",
        tool,
        command: tag ? `${tag} · ${approvalTarget(tool, input)}` : approvalTarget(tool, input),
      };
      // The relay's short ack window closes in 800 ms; everything below this
      // line is synchronous, so the card really is up by the time it lands.
      if (requestId) void Bridge.approvalAck(requestId);
      State.updateTask(taskId, "approval");
      State.isPinned = true;
      Sound.play("approval");
      if (focused) {
        island.alert("approval");
      } else {
        // Another agent holds the view, so the card would yank it away. The badge
        // is the signal instead — but it has to be on screen for that to mean
        // anything, hence the reveal. We just told the relay a human can act.
        State.setPillBadge(taskId, "approval");
        island.reveal();
      }
      // Atlas answers within 108 s or not at all; after that the terminal has
      // taken over and the card would be lying.
      pendingTimeout = window.setTimeout(() => {
        pendingTimeout = null;
        if (!State.pendingApproval) return;
        State.pendingApproval = null;
        State.isPinned = false;
        island.dropPin();
        State.updateTask(taskId, "working");
        State.setPillBadge(taskId, null);
        if (State.view === "approval") island.setView(State.defaultView());
        State.notify();
      }, 110_000);
      break;
    }

    default:
      break;
  }
  State.notify();
}
