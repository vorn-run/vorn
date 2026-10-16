# Adding an agent

Vorn runs each coding agent CLI through an adapter. An adapter is an extension package whose manifest has an `[agent]` section. The section says, as data, how Vorn finds the CLI, launches it, resumes a conversation, lists its models, reads its past sessions and its status, and installs its hooks. Most adapters need no program at all.

The adapters Vorn ships are in `packages/core/crates/adapters/agents/`, one folder each, in the same format. Adding one there is adding its folder and its name to `agents/shipped.toml`; no code changes. Any other adapter installs like any extension.

This guide builds an adapter for a made-up CLI called `acme`.

## 1. Make the package

Make a folder with a `vorn-extension.toml` and the agent's mark as an SVG:

```
acme-adapter/
  vorn-extension.toml
  icon.svg
```

Start the manifest with the package's own fields, then the agent's:

```toml
id = "example.acme"
name = "Acme"
version = "1.0.0"
api = "vorn:extension@0.9"
description = "Runs the Acme coding agent."

[agent]
id = "acme"                 # the agent type sessions carry
name = "Acme Code"          # Settings and the launcher
label = "Acme"              # menus, the palette, the task board
short_name = "Acme"         # workflow steps
description = "Acme CLI"
url = "https://acme.example/docs"
install = "npm install -g acme-cli"
icon = "icon.svg"
mark = "▲"                  # one character where only text is drawn
color = "#ff7a45"           # the card's accent
tint = "#ff7a4520"          # the tile behind the mark
```

The id is lowercase letters, digits and dashes. `shell` is taken.

## 2. Launch it

```toml
[agent.launch]
command = "acme"                   # looked for on PATH
args = []
headless_args = ["--yes"]          # used instead of args for headless runs
prompt = ["--prompt", "{prompt}"]  # how a first prompt rides the line
```

Leave `prompt` out when the CLI takes the prompt as a plain argument. Every word is quoted for the person's shell; `{prompt}` is replaced whole.

## 3. Resume and pin conversations

```toml
[agent.resume]
args = ["--resume", "{id}"]        # resume conversation {id}
pin = ["--session-id", "{id}"]     # start a fresh one under an id Vorn chooses
selectors = { with_value = ["--resume", "--session-id"], bare = ["--continue"] }
```

- `at = "start"` puts `args` first, for a CLI whose resume is a subcommand (`acme resume <id>`). Set `selectors = { subcommand = "resume" }` with it.
- `selectors` are the words a person's configured command may already carry. Vorn removes them where it can prove them before adding its own.
- Leave `pin` out when the CLI cannot be told an id. Vorn then reads the conversation the session took from the agent's own database, when you say how:

```toml
[agent.resume.capture]
db = "~/.acme/state.sqlite"
query = "SELECT id FROM threads WHERE rtrim(lower(replace(cwd, '\\', '/')), '/') = ?1 ORDER BY updated_at DESC LIMIT 1"
```

`?1` is the session's directory: lowercase, forward slashes, no trailing slash.

Leave `[agent.resume]` out when the CLI cannot be sent back to a conversation by id. Recent sessions still lists the agent's conversations, and resuming lets the agent choose.

## 4. Run it headless

Workflows and the CLI start agents without a terminal. The prompt goes to stdin.

```toml
[agent.headless]
prompt = ["--print"]               # ends the arguments when there is a prompt
no_prompt = ["--print", ""]        # and when there is none
resume = ["--resume", "{id}"]      # added for a resumed run
```

`resume_tail` replaces the ending for a resumed run instead, as `["exec", "resume", "{id}", "-"]`.

## 5. Models

```toml
[agent.models]
flag = "--model"                   # the default
selectors = ["-m"]                 # other flags taking a model, replaced
group = true                       # ids are provider/model
```

An adapter with `[agent.models]` lets a person type a model id. To fill the model picker, say how to ask the CLI for its list without starting a conversation. Either one run prints the models:

```toml
[agent.models.list]
args = ["models"]
lines = '^[^\s/]+/\S+$'            # each matching line is a model id
```

Or the CLI speaks JSON lines on stdin and stdout:

```toml
[agent.models.list]
args = ["--rpc"]

[[agent.models.list.steps]]
send = [{ id = 1, method = "initialize" }]
reply = { id = "{id}" }            # the line answering the last message sent
fail = "error"                     # a field that fails the listing when set

[[agent.models.list.steps]]
send = [{ id = 2, method = "models/list", params = { cwd = "{cwd}" } }]
reply = { id = "{id}" }
models = "result.models"           # the list, read through `choice`
next = "result.nextCursor"         # a page's cursor
cursor = "params.cursor"           # where the next request carries it

[agent.models.list.choice]
id = ["modelId", "id"]             # the first that is set
label = ["displayName", "name"]
description = ["{usage} usage", "description"]
skip = [{ path = "hidden", is = true }, { path = "access", is_not = "enabled" }]
```

`keep = { paired = ["--profile"], flags = ["--local"] }` keeps the configured arguments that change which models the CLI sees.

## 6. Past sessions

Recent sessions reads the agent's own files. Pick the shape that fits:

```toml
[agent.history]
activity = "turn"                  # what one unit of a conversation is called
title_limit = 80
rank = 5                           # orders it among agents at the same moment, lower first

# One JSON object per line, one line per prompt.
[agent.history.jsonl]
file = "~/.acme/history.jsonl"
session = "sessionId"
project = "cwd"
time = "timestamp"                 # milliseconds
title = "prompt"
```

`[agent.history.sqlite]` runs a query over the agent's database, with `{scope}` where the project's directories are matched. `[agent.history.json_files]` reads one JSON file per conversation in a folder per project.

Paths start with `~`, `{data}` (`$XDG_DATA_HOME`, `~/.local/share`, or `%LOCALAPPDATA%` on Windows) or `{env:NAME|fallback}`.

## 7. Status

What a session is doing comes, in this order, from:

1. what the agent reports itself (OSC 7501);
2. its hooks;
3. its screen rules.

Screen rules read the last screen. The first that matches wins:

```toml
[[agent.status]]
status = "waiting"
rows = [-20, -1]
any = ['Allow this command\?']

[[agent.status]]
status = "working"
rows = [-10, -1]
any = ['(?i)esc to cancel']

[[agent.status]]
status = "idle"
rows = [-6, -1]
any = ['^\s*> ']
```

Keep a recorded screen for each state beside the rules (`fixtures/status/<state>-<name>.toml`); the shipped adapters' tests read them.

## 8. Hooks

An agent with hooks tells Vorn what it does as it does it. Vorn installs them in one of two ways.

**Settings hooks.** HTTP hooks are merged into the agent's JSON settings while Vorn runs, beside the person's own:

```toml
[agent.hooks.settings]
file = "~/.acme/settings.json"
events = ["PreToolUse", "PostToolUse", "Stop", "Notification", "PermissionRequest", "SessionStart", "SessionEnd"]
own_status = { since = "3.0.0", events = ["SessionStart", "PermissionRequest"] }
```

**A hooks file.** Vorn writes a whole file of its own, whose scripts post each event:

```toml
[agent.hooks.file]
path = "{env:ACME_HOME|~/.acme}/hooks/vorn.json"
session = "acme-{terminal}"
events = [["sessionStart", "SessionStart"], ["preToolUse", "PreToolUse"]]
```

Variables the agent sets in its own sessions are never passed on:

```toml
[agent.env]
strip = ["ACME_SESSION"]
strip_prefixes = ["ACME_INTERNAL_"]
```

## 9. Context use and Vorn's tools

```toml
# Where the context a conversation used is recorded, for extensions with agent.usage.
[agent.usage]
transcript = "~/.acme/projects/{cwd}/{id}.jsonl"
at = "message.usage"
context = ["input_tokens", "cached_tokens"]
cached = "cached_tokens"

# Shown in Settings › Skills & MCP and the welcome guide.
[agent.mcp]
commands = ["acme mcp add vorn -- npx -y @vornrun/mcp@latest"]
note = "Restart Acme to load the tools."
```

## 10. Optional: read a headless stream

A CLI whose headless output is a stream of JSON events can tell Vorn the conversation it took and what it does. Give the package a `[service]`, and set `stream = true` under `[agent.headless]`. Vorn calls the service's `guest.handle` with method `agent.stream` and `{session, lines}`. It answers `{conversation?, events?: [{event, tool?}]}`, with events named as hooks name them.

## 11. Draw on its cards

An adapter may add templates like any extension (`[[contributes.card_footers]]` with `when = { agent = ["acme"] }`). Its templates read one row per session of the agent from the `sessions` collection: `{id, model, tool, tools}`.

## 12. Try it

```sh
vorn ext link ./acme-adapter
```

The agent appears in the launcher, the agent and model pickers, Settings › Coding Agents and Skills & MCP. Removing the package takes it out again. A shipped adapter is switched on and off in Settings › Extensions; with every adapter off, the launcher opens a plain shell.
