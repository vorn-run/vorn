//! `vorn ext new`: a package that links and runs as written, as a program
//! (Node, stdio JSON-RPC) or, with `--wasm`, as a Rust crate built into a
//! Wasm component against `vorn-extension-sdk`.

use std::path::{Path, PathBuf};

/// The Rust package name an extension id makes: `acme.hello` is `acme-hello`.
fn crate_name(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect()
}

/// `acme.hello-world` reads as `Hello world`.
fn title(id: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id).replace(['-', '_'], " ");
    let mut chars = last.chars();
    chars.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Where the SDK comes from: a folder given with `--sdk`, else this release's tag.
fn sdk_dependency(sdk: Option<&Path>, version: &str) -> String {
    match sdk {
        Some(path) => format!(
            "vorn-extension-sdk = {{ path = {:?} }}",
            path.to_string_lossy()
        ),
        None => format!(
            "vorn-extension-sdk = {{ git = \"https://github.com/vorn-run/vorn\", tag = \"v{version}\" }}"
        ),
    }
}

fn manifest(id: &str, wasm: bool) -> String {
    let name = title(id);
    let service = if wasm {
        format!(
            "[build]\ncommand = [\"cargo\", \"build\", \"--release\", \"--target\", \"wasm32-wasip2\"]\n\n\
             [service]\ncomponent = \"target/wasm32-wasip2/release/{}.wasm\"\n",
            crate_name(id).replace('-', "_")
        )
    } else {
        "[service]\ncommand = [\"node\", \"program.mjs\"]\n".to_owned()
    };
    format!(
        "id = \"{id}\"
name = \"{name}\"
version = \"0.1.0\"
api = \"vorn:extension@0.9\"
description = \"Lists the sessions running, and says hello.\"

{service}
[declares]
uses = [\"sessions.read\"]

[[contributes.settings]]
id = \"page\"
title = \"{name}\"
description = \"The sessions running now.\"
icon = \"puzzle\"
tree = true
"
    )
}

const LIB_RS: &str = r#"//! A settings page that lists the sessions running, with a button that says hello.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{export, json, Error, Extension, Host, Init, Op, Value};

struct Hello;

impl Extension for Hello {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Hello)
    }

    fn render(&mut self, host: &mut dyn Host, _: &str, _: Surface) -> Result<tree::Node, Error> {
        let sessions = host.call("sessions.list", Value::Null)?;
        let entries = sessions
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let id = s["id"].as_str().unwrap_or_default();
                entry(id, s["name"].as_str().unwrap_or(id))
                    .detail(s["agent"].as_str().unwrap_or_default())
                    .aside(s["status"].as_str().unwrap_or_default())
            });
        let said = host.rows("hellos")?.len();
        Ok(col([
            toolbar([button("hello", "Say hello").icon("hand")]),
            stat(said.to_string(), "Hellos said"),
            list("sessions", entries).empty("No sessions are running."),
        ])
        .gap(16.0))
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
        if e.id != "hello" {
            return Err(Error::unknown(&e.id));
        }
        let n = host.rows("hellos")?.len() + 1;
        host.publish("hellos", vec![Op::Upsert(json!({ "id": n.to_string() }))]);
        Ok(json!({ "toast": "Hello from Wasm" }))
    }
}

export!(Hello);

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::testing::FakeHost;
    use vorn_extension_sdk::{dispatch, Ctx};

    #[test]
    fn says_hello_and_counts_it() {
        let mut host = FakeHost::default().answer("sessions.list", |_| Ok(json!([])));
        let mut hello = Hello::init(&Init::default(), &mut host).unwrap();
        let click = json!({ "point": "page", "event": { "id": "hello", "kind": "click" } });
        let done = dispatch(&mut hello, &mut host, tree::EVENT, click, &Ctx::default()).unwrap();
        assert_eq!(done["toast"], "Hello from Wasm");
        let page = dispatch(&mut hello, &mut host, tree::RENDER, json!({ "point": "page" }), &Ctx::default()).unwrap();
        assert!(page.to_string().contains("Hellos said"));
        assert_eq!(host.rows("hellos").unwrap().len(), 1);
    }
}
"#;

const PROGRAM_MJS: &str = r#"// A settings page that lists the sessions running, with a button that says hello.
// Vorn speaks to it over stdio, one JSON-RPC message per line.
import { createInterface } from 'node:readline'

let next = 100
const waiting = new Map()
const send = (message) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n')
const ask = (method, params) =>
  new Promise((resolve, reject) => {
    const id = next++
    waiting.set(id, { resolve, reject })
    send({ id, method, params })
  })
let hellos = 0

async function handle(method, args) {
  if (method === 'ui.render') {
    const sessions = await ask('host.call', { method: 'sessions.list', args: null })
    const entries = sessions.map((s) => ({ kind: 'entry', id: s.id, title: s.name, detail: s.agent }))
    return {
      tokens: 1,
      tree: {
        kind: 'box',
        props: { gap: 16 },
        children: [
          { kind: 'toolbar', children: [{ kind: 'button', id: 'hello', label: 'Say hello' }] },
          { kind: 'stat', value: String(hellos), label: 'Hellos said' },
          { kind: 'list', id: 'sessions', empty: 'No sessions are running.', children: entries }
        ]
      }
    }
  }
  if (method === 'ui.event' && args.event?.id === 'hello') {
    hellos += 1
    return { toast: 'Hello from a program' }
  }
  throw { code: 'unknown-action', message: `no action ${method}` }
}

createInterface({ input: process.stdin }).on('line', async (line) => {
  const m = JSON.parse(line)
  if (m.method === undefined) {
    const w = waiting.get(m.id)
    waiting.delete(m.id)
    if (m.error) w?.reject(m.error)
    else w?.resolve(m.result)
    return
  }
  if (m.method === 'guest.init') return send({ id: m.id, result: null })
  if (m.method !== 'guest.handle') return
  try {
    send({ id: m.id, result: await handle(m.params.method, m.params.args) })
  } catch (e) {
    send({ id: m.id, error: { code: -32000, message: e.message ?? String(e), data: { code: e.code ?? 'failed' } } })
  }
})
"#;

fn readme(id: &str, wasm: bool) -> String {
    let name = title(id);
    if wasm {
        format!(
            "# {name}\n\n\
             A Vorn extension in Rust, built as a Wasm component. It runs sandboxed in\n\
             Vorn: it reaches nothing but what it asks Vorn for, held to what\n\
             `vorn-extension.toml` declares.\n\n\
             ```sh\n\
             rustup target add wasm32-wasip2   # once\n\
             cargo test                        # its logic, against a fake Vorn\n\
             vorn ext link --watch             # build, link, and rebuild on every save\n\
             ```\n\n\
             Its page is under Settings, as {name}.\n"
        )
    } else {
        format!(
            "# {name}\n\n\
             A Vorn extension as a program: Vorn starts `node program.mjs` and speaks\n\
             to it over stdio.\n\n\
             ```sh\n\
             vorn ext link\n\
             ```\n\n\
             Its page is under Settings, as {name}.\n"
        )
    }
}

/// Writes a new package for extension `id` into `dir`, which must not exist
/// or be empty; answers the files written.
pub fn write(
    dir: &Path,
    id: &str,
    wasm: bool,
    sdk: Option<&Path>,
    version: &str,
) -> Result<Vec<PathBuf>, String> {
    if !vorn_extensions::pack::is_safe_id(id) {
        return Err(format!(
            "\"{id}\" is not an extension id: letters, digits, '.', '-' and '_', as acme.hello"
        ));
    }
    let occupied = std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some());
    if occupied {
        return Err(format!("{} is not empty", dir.display()));
    }
    let mut files: Vec<(&str, String)> = vec![
        ("vorn-extension.toml", manifest(id, wasm)),
        ("README.md", readme(id, wasm)),
    ];
    if wasm {
        let cargo = format!(
            "[package]
name = \"{}\"
version = \"0.1.0\"
edition = \"2021\"
publish = false

[lib]
crate-type = [\"cdylib\", \"rlib\"]

[dependencies]
{}

[profile.release]
opt-level = \"s\"
lto = true
strip = true
panic = \"abort\"

# A package of its own, even inside another workspace.
[workspace]
",
            crate_name(id),
            sdk_dependency(sdk, version)
        );
        files.push(("Cargo.toml", cargo));
        files.push(("src/lib.rs", LIB_RS.to_owned()));
        files.push((".gitignore", "/target\n".to_owned()));
    } else {
        files.push(("program.mjs", PROGRAM_MJS.to_owned()));
    }
    let mut written = Vec::with_capacity(files.len());
    for (name, text) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_rust_component_package_whose_manifest_reads() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("hello");
        let sdk = Path::new("/src/vorn/packages/core/crates/extension-sdk");
        let files = write(&at, "acme.hello-world", true, Some(sdk), "0.8.0").unwrap();
        assert_eq!(files.len(), 5);
        let text = std::fs::read_to_string(at.join("vorn-extension.toml")).unwrap();
        let m = vorn_extensions::manifest::file::parse(&text).expect("the manifest reads");
        let service = m.service.expect("a service");
        assert_eq!(
            service.component.as_deref(),
            Some("target/wasm32-wasip2/release/acme_hello_world.wasm")
        );
        assert_eq!(m.name, "Hello world");
        let cargo = std::fs::read_to_string(at.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"acme-hello-world\""), "{cargo}");
        assert!(
            cargo.contains(&format!("path = {:?}", sdk.to_string_lossy())),
            "{cargo}"
        );
        assert!(write(&at, "acme.hello", true, None, "0.8.0")
            .unwrap_err()
            .contains("is not empty"));
    }

    #[test]
    fn writes_a_program_package_and_names_the_release_sdk_by_tag() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hello", false, None, "0.8.0").unwrap();
        let text = std::fs::read_to_string(dir.path().join("vorn-extension.toml")).unwrap();
        let m = vorn_extensions::manifest::file::parse(&text).expect("the manifest reads");
        assert_eq!(m.service.unwrap().command, ["node", "program.mjs"]);
        assert!(sdk_dependency(None, "0.8.0").contains("tag = \"v0.8.0\""));
        assert!(write(dir.path(), "../x", false, None, "0.8.0").is_err());
    }
}
