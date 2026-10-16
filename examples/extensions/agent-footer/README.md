# Agent footer

An example extension that adds a footer under each agent's card: the model the session was started with, how many tools its hooks reported, and the last one.

```sh
vorn ext link examples/extensions/agent-footer
```

Its program (`program.mjs`, which needs Node) listens to three events:

- `session.created` names the agent and, when one was chosen, the model.
- `agent.hook` carries each hook the agent posts, with the tool it is about.
- `session.exited` takes the row away.

It publishes one row per session to its `sessions` collection, which the footer template (`views/footer.kdl`) draws on that session's card. The footer shows once the first tool call is reported.

To draw it on one agent's cards only, give the footer a rule in the manifest:

```toml
[[contributes.card_footers]]
id = "tools"
view = "views/footer.kdl"
when = { agent = ["acme"] }
```

An agent adapter (see `docs/adding-an-agent.md`) can carry the same footer and program beside its `[agent]` section.
