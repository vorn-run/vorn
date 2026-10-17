# Workflow files and step types

A workflow can live in a file in your project, next to the code it works on. Vorn reads every
`.toml` file in a project's `.vorn/workflows/` folder, watches the folder, and runs what it finds
the same way it runs a workflow made in the editor. Edits made in the editor go back into the
file.

Workflow files are TOML, like extension manifests and layouts.

## The file

A workflow file starts with the workflow's fields. Then come its trigger, `[trigger]`, and its
steps in order, one `[[step]]` table each.

```toml
# Triages open issues: an agent reads each one, and a person approves the plans.
name = "Triage issues"
id = "triage-issues"
icon = "ListChecks"
iconColor = "#6366f1"
enabled = true

[trigger]
id = "start"
type = "manual"

[[trigger.inputs]]
key = "repo"
label = "Repository"
type = "text"
required = true

# Prints {"items": [...]}, so later steps can read its fields.
[[step]]
id = "fetch"
type = "script"
label = "Fetch open issues"
slug = "fetch"
scriptType = "bash"
scriptContent = """
gh issue list --repo {{inputs.repo}} --state open --limit 20 --json number,title,body --jq '{items: .}'
"""

[[step]]
id = "any"
type = "condition"
label = "Any open?"
variable = "{{steps.fetch.json.items.length}}"
operator = "notEquals"
value = "0"

[[step]]
id = "each"
type = "loop"
label = "Each issue"
slug = "each"
needs = ["any:true"]
mode = "forEach"
items = "{{steps.fetch.json.items}}"
bodyNodeIds = ["triage"]
parallel = true
concurrency = 3

[[step]]
id = "triage"
type = "launchAgent"
label = "Triage one issue"
slug = "triage"
agentType = "claude"
headless = true
projectName = { "@project-name" = "app" }
projectPath = { "@project" = "." }
prompt = """
Read issue #{{loop.item.number}}: {{loop.item.title}}

{{loop.item.body}}

Say in two lines how you would fix it.
"""

[[step]]
id = "approve"
type = "approval"
label = "Approve the plans"
slug = "approve"
message = """
The plans, one per issue:
{{steps.each.outputs}}
"""
```

- A step's table holds its own fields, `id`, `type`, `label`, `slug`, `onError`, `x`, `y` and
  `needs`, and beside them its settings: what the editor shows in the step's panel.
- A step runs after the step before it unless it says what it `needs`. List several ids to wait
  for all of them. `"any:true"` follows the true branch of a condition, and `needs = []` starts
  a step with nothing before it. An edge with an id of its own is written `"fetch#e1"`.
- A list of objects is a list of tables, as `[[trigger.inputs]]` above; an object is a table,
  `[step.headers]`, or written inline, `headers = { Accept = "text/plain" }`.
- Text across several lines goes between `"""`. Use `'''` instead when the text has
  backslashes, so they stay as written.
- A connection is written by name, `connectionId = { "@connection" = "workspace-eng" }`. A file
  shared with someone else runs against their connection of that name.
- A path inside the file's project is written relative to it, `{ "@project" = "src/app" }`, and
  the project itself is `{ "@project" = "." }`. `{ "@project-name" = "web" }` names the project
  the file is in.
- `x` and `y` keep a step's place on the canvas.
- Keys starting with `@` belong to the file format. TOML has no null, so a null value is
  `{ "@json" = "null" }`. You will rarely write these yourself; Vorn writes them so that every
  workflow reads back exactly as it was stored.
- Comments stay. When the editor writes a change back, it rewrites only the steps and fields
  that changed.

If the file and the editor both change the same workflow before either is saved, Vorn keeps both
changes. When they change the same field, the file wins.

Removing a file removes its workflow, and deleting a file-backed workflow in the app removes its
file.

### Reading a script's JSON

When everything a script step prints is one JSON value, later steps can read its fields with
`{{steps.<slug>.json.<path>}}`. A path goes into objects by key and into lists by index, and
`.length` counts a list:

```text
{{steps.fetch.json.items}}            the whole list
{{steps.fetch.json.items.0.title}}    the first item's title
{{steps.fetch.json.items.length}}     how many items there are
```

`{{steps.<slug>.output}}` is still the printed text. Output that is not JSON, or that has other
text around the JSON, has no `json` field. Terminal colours are removed before the output is
read, so `jq` works as it does in a terminal.

### Loops in parallel

A loop that goes through a list (`mode = "forEach"`) can run its items side by side. Set
`parallel = true`, and `concurrency` to the number of items that may run at once: 4 when it is
not set, at most 16. In the editor, turn on **Run in parallel** in the loop's panel.

- Each item runs the loop's steps as steps of its own. The run shows them as separate rows,
  for example "Triage one issue · item 3".
- `{{steps.<loop>.results}}` and `{{steps.<loop>.outputs}}` keep the list's order, whatever
  order the items finish in.
- When an item fails, items that have not started do not start, and the loop fails. Items
  already running finish.
- After the loop, each of its steps holds what the last item left, as after a loop that runs one
  item at a time.
- A loop in parallel cannot stop early with **Stop early when**, because every item starts.

### From the app

**Save to project** in a workflow's menu writes it to its project's `.vorn/workflows/` folder.
From then on the file is where the workflow lives. The project is the one its steps name, or the
active project.

### Checking a file

```bash
vorn workflow check .vorn/workflows/*.toml
```

prints each mistake as `file:line:column: message`, for example
``triage.toml:14:1: needs `fetch`, which no step here is``. It also names step types and
connections this machine does not have, and hints at a setting whose name is a misspelt step
field, such as `lable`. It exits 1 when any file has a mistake.

## Step types and triggers from extensions

Every step type and trigger is declared by an extension. Vorn's own come from the built-in
**Workflow steps and triggers**, and Tasks and Connectors add theirs. Another extension adds a
step type with `[[contributes.nodes]]`:

```toml
[[contributes.nodes]]
id = "shout"
title = "Shout"
description = "Repeats its text, louder."
icon = "megaphone"
category = "Utilities"
config = "nodes/shout.json"   # JSON Schema of the step's config; the editor draws its form from it

[[contributes.nodes.outputs]]
name = "output"
description = "The shouted text"
```

When a run reaches the step, Vorn calls the extension's `workflow.node.<id>` with the step's
config, every template already filled in, and the run's context. The answer is the step's state:
`status` (`success`, `error`, `skipped`, or `waiting`), and any of `output`, `logs`, `error`
and `structuredOutput`. [examples/extensions/shout](../examples/extensions/shout) is a complete
one.

A trigger is declared with `[[contributes.triggers]]`, with `timing = "cron"`, `"once"` or
`"webhook"` when Vorn should fire it on a schedule or from a webhook. Otherwise the extension
fires it itself with `workflow.fire`.

An extension can also ship workflows with `[[contributes.workflows]]` and
`file = "workflows/try.toml"`. Each one is added once, when the extension is installed.

When an extension is off, its step types stay known. A step of that type fails with a message
saying which extension to turn on.
