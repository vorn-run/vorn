# vorn-extension-sdk

Write a Vorn extension in Rust, built as a sandboxed Wasm component for
`wasm32-wasip2`: the extension API's calls, events, storage, files and
sessions, and the element tree its UI is drawn from. `vorn ext new --wasm`
writes a package that uses it, and
[docs/wasm-extensions.md](../../../../docs/wasm-extensions.md) walks through it.
