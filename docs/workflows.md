# Workflow files and step types

A workflow can live in a file in your project, next to the code it works on. Vorn reads every
`.kdl` file in a project's `.vorn/workflows/` folder, watches the folder, and runs what it finds
the same way it runs a workflow made in the editor. Edits made in the editor go back into the
file.

## The file

A workflow file holds one `workflow` node named after the workflow. Inside it are the
workflow's fields, its trigger, and its steps in order.

```kdl
// Reviews each pull request it is given.
workflow "Review a pull request" {
    id "review"
    icon "GitPullRequest"
    iconColor "#6366f1"
    enabled #true
    trigger "manual" id="start" {
        inputs {
            - {
                key "pullRequest"
                label "Pull request"
                type "text"
            }
        }
    }
    step "script" id="fetch" {
        scriptType "bash"
        scriptContent "gh pr diff {{inputs.pullRequest}}"
    }
    step "condition" id="big" needs="fetch" {
        variable "{{steps.fetch.output}}"
        operator "isNotEmpty"
    }
    step "launchAgent" id="review" needs="big:true" {
        agentType "claude"
        projectName (project)"."
        prompt """
            Review this diff.
            {{steps.fetch.output}}
            """
    }
}
```

- A step runs after the step before it unless it says what it `needs=`. Give several ids
  separated by spaces to wait for all of them. `needs="big:true"` follows the true branch of a
  condition; `needs=#null` starts a step with nothing before it.
- A step's config is its children: a key and its value, an object as a block of keys, a list as
  a block of `-` items.
- A connection is written by name, `connectionId (connection)"workspace-eng"`. A file shared with
  someone else runs against their connection of that name.
- A path inside the file's project is written relative to it, `(project)"src/app"`, and the
  project itself is `(project)"."`. `(project-name)"web"` names the project the file is in.
- `x=` and `y=` on a step keep its place on the canvas.
- Comments stay. When the editor writes a change back, it rewrites only the steps and fields
  that changed.

If the file and the editor both change the same workflow before either is saved, Vorn keeps both
changes. When they change the same field, the file wins.

Removing a file removes its workflow, and deleting a file-backed workflow in the app removes its
file.

### From the app

**Save to project** in a workflow's menu writes it to its project's `.vorn/workflows/` folder.
From then on the file is where the workflow lives. The project is the one its steps name, or the
active project.

### Checking a file

```bash
vorn workflow check .vorn/workflows/*.kdl
```

prints each mistake as `file:line:column: message`, for example
`review.kdl:14:26: unknown property lable=; did you mean label=?`. It also names step types and
connections this machine does not have. It exits 1 when any file has a mistake.

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
`file = "workflows/try.kdl"`. Each one is added once, when the extension is installed.

When an extension is off, its step types stay known. A step of that type fails with a message
saying which extension to turn on.
