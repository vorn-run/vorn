# Agent footer

An example extension, written in Rust and run as a Wasm component, that adds a footer under each agent's card: the model the session was started with, how many tools its hooks reported, and the last one.

```sh
rustup target add wasm32-wasip2
vorn ext link examples/extensions/agent-footer
```

It listens to three events:

- `session.created` names the agent and, when one was chosen, the model.
- `agent.hook` carries each hook the agent posts, with the tool it is about.
- `session.exited` takes the row away.

It keeps one row per session in its `sessions` collection. The footer is an element tree (`tree = true`) drawn once per session: the root holds one child per session, its id the session's id, built with `per_session`. It shows once the first tool call is reported.

To draw it on one agent's cards only, give the footer a rule in the manifest:

```toml
[[contributes.card_footers]]
id = "tools"
tree = true
when = { agent = ["acme"] }
```

An agent adapter (see `docs/adding-an-agent.md`) can carry the same footer and component beside its `[agent]` section.
