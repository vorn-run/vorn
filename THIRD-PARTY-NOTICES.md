# Third-party notices

Vorn bundles data derived from the projects below. Their licence terms apply
to that data and travel with it.

## Completion-spec corpus

`src/renderer/lib/completion-index/` is generated from the completion specs
published as `@withfig/autocomplete`, used to give the terminal's input bar
subcommand and flag suggestions.

- Upstream: https://github.com/withfig/autocomplete
- Licence: MIT — full text in `src/renderer/lib/completion-index/LICENSE`
- Version indexed: see `src/renderer/lib/completion-index/meta.json`

Only the static parts of each spec are extracted — names, descriptions,
subcommands, flags, and whether an argument is a file or a directory. The
dynamic generators are dropped; Vorn resolves branches, paths, and package
scripts from the live session instead.

Regenerate with `yarn gen:completions` after editing
`scripts/completion-allowlist.txt`.
