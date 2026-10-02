# Pairing Atlas with Voicebox

The two directions, both local:

## 1. Voicebox gives Atlas its voice (built in, v0.4.0+)

Start Voicebox, then Atlas → Settings → Voice → **Enabled**. Atlas speaks chat
replies, approval requests and finished sessions through Voicebox's `/speak`
endpoint, using any profile you've cloned (pick it in the same section).
Atlas identifies itself with the client id `atlas`, so Voicebox's per-client
voice bindings can give it a voice distinct from everything else.

Bonus with zero configuration: Voicebox's **global dictation hotkey** types
into whatever field is focused — including Atlas's chat input. And the chat's
own mic button transcribes through Voicebox's Whisper (`/transcribe`).

## 2. Voicebox as an Atlas MCP client (v0.5.0+)

Atlas also *serves* MCP, so Voicebox (or any MCP client) can ask what your
agents are doing and act on their permission requests:

1. Atlas → Settings → MCP → **Enabled** (default port `17510`; the endpoint
   URL appears next to it: `http://127.0.0.1:17510/mcp`).
2. In Voicebox → Settings → MCP, add that endpoint as a server.
3. The tools it gains:

| Tool | What it does |
|---|---|
| `atlas_status` | What Claude Code / ZCode / Codex are doing right now |
| `atlas_pending` | Permission requests waiting in the island |
| `atlas_decide` | Approve or deny one, by request id |
| `atlas_speak` | Make the island say something |
| `atlas_stats` / `atlas_tokens` | Daily activity and token usage per agent |

Now "what's Codex up to?" spoken to Voicebox answers from Atlas's live state,
and a voice "approve it" can clear a pending request.

**Trust note**: `atlas_decide` answers an agent's permission prompt. Point it
only at clients you'd trust with your terminal — the same bar as the agents'
own hooks.


## 3. Any stdio MCP client (Codex, Claude Code, ZCode)

Atlas also speaks MCP over stdio, bridging to its HTTP server — one binary
flag, no extra process to install:

```
atlas --mcp-stdio        # JSON-RPC on stdin/stdout, forwarded to the island
```

Codex, for example:

```
codex mcp add atlas -- /usr/local/bin/atlas --mcp-stdio
codex exec "Use the atlas_status tool and report what the agents are doing."
```

`ATLAS_MCP_PORT` overrides the port the bridge forwards to.


## Running Voicebox in docker (what worked here)

The desktop builds don't cover every distro; `docker compose up` does, with
four local adjustments on a classic-builder daemon:

1. `Dockerfile`: `COPY --chmod=755 …` → plain `COPY` + `RUN chmod 755 …`
   (BuildKit-only flag otherwise).
2. After first start: `docker exec voicebox chown -R voicebox:voicebox /home/voicebox/.cache`
   (the named volume starts root-owned) and `docker exec voicebox chmod 777 /app/data/generations`
   (the bind mount the WAVs land in).
3. A `docker-compose.override.yml` passing the host audio sockets:

   ```yaml
   services:
     voicebox:
       environment:
         - XDG_RUNTIME_DIR=/tmp/xdg
         - PULSE_SERVER=unix:/tmp/xdg/pulse/native
       volumes:
         - /run/user/1000/pulse/native:/tmp/xdg/pulse/native
         - /run/user/1000/pipewire-0:/tmp/xdg/pipewire-0
   ```

4. The compose maps the API to host port **17600**; Atlas probes 17493 (desktop
   install) and 17600 (docker) and uses whichever answers.

Create a profile once (or in the UI): `POST /profiles {"name":"Atlas",
"voice_type":"preset","preset_engine":"kokoro","preset_voice_id":"af_alloy"}`,
pick it in Settings → Voice → Voice, and the island talks.
