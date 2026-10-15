//! A hooks file of Vorn's own, for an adapter whose `[agent.hooks.file]`
//! names it: shared by every terminal of that agent, each event runs a
//! script that reads the endpoint's port and token beside the owner record
//! and posts the event as HTTP hooks do, under a conversation id Vorn derives
//! from the terminal ([`Spec::session_id`]). Outside Vorn the script does
//! nothing. A file Vorn did not write (no `_vorn` mark) is never replaced or
//! removed.

use std::io;
use std::path::Path;

use serde_json::{json, Map, Value};

/// What the adapter says of its hooks file.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// The conversation id the scripts post; `{terminal}` is the terminal's id.
    pub session: &'a str,
    /// The agent's event names, each with Vorn's name for it.
    pub events: &'a [[String; 2]],
}

impl Spec<'_> {
    /// The conversation id a terminal's hooks post.
    pub fn session_id(&self, terminal: &str) -> String {
        self.session.replace("{terminal}", terminal)
    }

    /// The script's expression for that id, `t` being the terminal's.
    fn session_expr(&self) -> String {
        let (before, after) = self
            .session
            .split_once("{terminal}")
            .unwrap_or((self.session, ""));
        let quote = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
        if after.is_empty() {
            format!("'{}'+t", quote(before))
        } else {
            format!("'{}'+t+'{}'", quote(before), quote(after))
        }
    }
}

fn node_script(event: &str, vorn_dir: &Path, session: &str) -> String {
    let file = |name: &str| vorn_dir.join(name).to_string_lossy().replace('\\', "/");
    let (port, token) = (file("port"), file("token"));
    [
        "const t=process.env.VORN_SESSION_ID||'';".to_owned(),
        "if(!t){process.stdout.write('{}');process.exit(0)}".to_owned(),
        "const d=JSON.parse(require('fs').readFileSync(0,'utf8'));".to_owned(),
        "let port,token;".to_owned(),
        format!("try{{port=require('fs').readFileSync('{port}','utf8').trim();token=require('fs').readFileSync('{token}','utf8').trim()}}catch(e){{process.stdout.write('{{}}');process.exit(0)}}"),
        format!("const body=JSON.stringify({{session_id:{session},hook_event_name:'{event}',cwd:d.cwd||'',tool_name:d.toolName||'',vorn_terminal_id:t}});"),
        "const r=require('http').request({hostname:'127.0.0.1',port:+port,path:'/hooks',method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+token}});".to_owned(),
        "r.on('error',()=>{});r.end(body);".to_owned(),
        "process.stdout.write('{}')".to_owned(),
    ]
    .concat()
}

/// The hooks file's text, for the endpoint files in `vorn_dir` (`~/.vorn`).
pub fn hooks_json(spec: Spec<'_>, vorn_dir: &Path) -> String {
    let mut hooks = Map::new();
    let session = spec.session_expr();
    for [own, vorn] in spec.events {
        let script = node_script(vorn, vorn_dir, &session);
        // Outside Vorn the event is still read, so the agent's write never meets a closed pipe.
        let bash = format!(
            "if [ -z \"$VORN_SESSION_ID\" ]; then cat >/dev/null; else node -e \"{}\"; fi",
            script.replace('"', "\\\"")
        );
        let powershell = format!(
            "if ($env:VORN_SESSION_ID) {{ node -e '{}' }}",
            script.replace('\'', "''")
        );
        hooks.insert(
            own.clone(),
            json!([{ "type": "command", "bash": bash, "powershell": powershell }]),
        );
    }
    let file = json!({ "version": 1, "_vorn": true, "hooks": hooks });
    serde_json::to_string_pretty(&file).expect("a JSON value always serializes")
}

/// The file's text and whether Vorn wrote it; `None` when there is none.
fn read(file: &Path) -> Option<(String, bool)> {
    let text = std::fs::read_to_string(file).ok()?;
    let ours = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("_vorn").and_then(Value::as_bool))
        == Some(true);
    Some((text, ours))
}

/// What [`install`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Written,
    Current,
    /// The person's own file is there.
    NotOurs,
}

/// Puts the hooks file in place when it is missing or stale.
pub fn install(spec: Spec<'_>, file: &Path, vorn_dir: &Path) -> io::Result<Installed> {
    let text = hooks_json(spec, vorn_dir);
    match read(file) {
        Some((_, false)) => return Ok(Installed::NotOurs),
        Some((existing, true)) if existing == text => return Ok(Installed::Current),
        _ => {}
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)?;
    Ok(Installed::Written)
}

/// Removes the hooks file when it is still Vorn's.
pub fn uninstall(file: &Path) -> io::Result<bool> {
    match read(file) {
        Some((_, true)) => std::fs::remove_file(file).map(|()| true),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> Vec<[String; 2]> {
        [("start", "SessionStart"), ("preTool", "PreToolUse")]
            .map(|(a, b)| [a.to_owned(), b.to_owned()])
            .to_vec()
    }

    #[test]
    fn writes_vorn_s_file_and_leaves_the_person_s() {
        let dir = tempfile::tempdir().unwrap();
        let vorn = dir.path().join(".vorn");
        let file = dir.path().join(".agent/hooks/vorn.json");
        let events = events();
        let spec = Spec {
            session: "agent-{terminal}",
            events: &events,
        };
        assert_eq!(install(spec, &file, &vorn).unwrap(), Installed::Written);
        assert_eq!(install(spec, &file, &vorn).unwrap(), Installed::Current);
        let text = std::fs::read_to_string(&file).unwrap();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let bash = parsed["hooks"]["preTool"][0]["bash"].as_str().unwrap();
        assert!(bash.contains("hook_event_name:'PreToolUse'"), "{bash}");
        assert!(bash.contains("session_id:'agent-'+t,"), "{bash}");
        assert!(bash.contains(&format!(
            "{}/port",
            vorn.to_string_lossy().replace('\\', "/")
        )));
        assert!(uninstall(&file).unwrap());
        assert!(!file.exists());

        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"hooks":{}}"#).unwrap();
        assert_eq!(install(spec, &file, &vorn).unwrap(), Installed::NotOurs);
        assert!(!uninstall(&file).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"hooks":{}}"#);
        assert_eq!(spec.session_id("t1"), "agent-t1");
        let wrapped = Spec {
            session: "a-{terminal}-b",
            events: &events,
        };
        assert_eq!(wrapped.session_expr(), "'a-'+t+'-b'");
    }
}
