# Shout

A workflow step from outside Vorn. Its manifest declares a node type,
`shout`, under `[[contributes.nodes]]`, with the JSON Schema of its config
in `nodes/shout.json`; the editor's step library lists it beside Vorn's own
steps and draws its form from that schema. When a run reaches a `shout`
step, Vorn calls the extension's `workflow.node.shout` with:

- `node`: the step's id, type, label and slug
- `config`: its config, every template already expanded against the run
- `raw`: its config as written
- `context`: what the run was started with (task, trigger, inputs, item)
- `runId`, `workflowId`, `workflowName`

and the answer is the step's state: `status` (`success`, `error`,
`skipped`, or `waiting` to park it), and any of `output`, `logs`, `error`,
`structuredOutput`, `taskId` and `sessionId`.

`workflows/try-shout.toml` is a workflow the extension ships
(`[[contributes.workflows]]`): it is added once when the extension is
installed. Install it from a git repository with `vorn ext install <url>`.
