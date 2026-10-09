//! `vorn_agents::launch` on seeded random lines against the recorded answers (`tests/fixtures/js-reference/launch-random.json`).

use std::path::PathBuf;

use serde_json::{json, Value};
use vorn_agents::launch::{self, LaunchRequest, Machine, Platform, Quoting};
use vorn_agents::{Agent, AgentCommand};

const CASES: usize = 4000;

/// mulberry32 in wrapping 32-bit arithmetic, as `Math.imul` and `>>>` compute it.
struct Seeded(u32);

impl Seeded {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
        list[(self.next() * list.len() as f64) as usize]
    }

    fn maybe<'a>(&mut self, list: &[&'a str]) -> Option<&'a str> {
        if self.next() < 0.5 {
            None
        } else {
            Some(self.pick(list))
        }
    }
}

const FRAGMENTS: &[&str] = &[
    "claude",
    "codex",
    "npx",
    "--resume",
    "--resume=",
    "-r",
    "-rold",
    "-ir",
    "--continue",
    "-c",
    "--session-id",
    "--session",
    "-s",
    "resume",
    "--last",
    "--",
    "--model",
    "-m",
    "-mx",
    "model=x",
    "x",
    "old",
    "'a b'",
    "'",
    "\"",
    "\"q $x\"",
    "\"a\\\"b\"",
    "\\",
    "\\ ",
    "$(",
    "$",
    ")",
    "(",
    "`",
    "|",
    "&&",
    ";",
    ">",
    "<",
    "\n",
    "\r",
    "\t",
    "é",
    "😀",
    "\u{a0}",
    "$HOME",
    "~/bin",
];
const SEPARATORS: &[&str] = &[" ", " ", " ", "", "\t", "  "];
const AGENTS: &[&str] = &["claude", "copilot", "codex", "opencode", "gemini"];

/// One generated case and the command line it carries.
struct Case {
    req: LaunchRequest,
    config: AgentCommand,
    machine: Machine,
    line: String,
}

fn cases(count: usize) -> Vec<Case> {
    let mut rand = Seeded(0x5eed1);
    let mut cases = Vec::with_capacity(count);
    for _ in 0..count {
        let mut line = String::new();
        let words = 1 + (rand.next() * 7.0) as usize;
        for w in 0..words {
            if w > 0 {
                line.push_str(rand.pick(SEPARATORS));
            }
            line.push_str(rand.pick(FRAGMENTS));
        }
        if rand.next() < 0.1 {
            line.push_str(rand.pick(SEPARATORS));
        }
        let agent = Agent::from_id(rand.pick(AGENTS)).expect("every generated agent is known");
        let platform = if rand.next() < 0.7 { "linux" } else { "win32" };
        let shell = rand.pick(&["cmd.exe", "C:\\pwsh.exe"]);
        let resume_session_id = rand.maybe(&["new", "", "a b", "it's"]).map(str::to_owned);
        let session_id = rand.maybe(&["pin", "", "50%"]).map(str::to_owned);
        let model = (rand.next() < 0.15).then(|| rand.pick(&["m", "-bad", "a b"]).to_owned());
        let initial_prompt = rand
            .maybe(&["p", "", "it's \"x\"", "a\nb"])
            .map(str::to_owned);
        let remote_host_id = (rand.next() < 0.1).then(|| "host".to_owned());
        let args = (rand.next() < 0.3).then(|| {
            vec![
                rand.pick(FRAGMENTS).to_owned(),
                rand.pick(FRAGMENTS).to_owned(),
            ]
        });
        let command_args = if rand.next() < 0.5 {
            Vec::new()
        } else {
            vec![rand.pick(FRAGMENTS).to_owned()]
        };
        let platform = Platform::from_node(platform);
        cases.push(Case {
            req: LaunchRequest {
                agent,
                args,
                model,
                remote_host_id,
                resume_session_id,
                session_id,
                initial_prompt,
            },
            config: AgentCommand {
                command: line.clone(),
                args: command_args,
                headless_args: None,
                fallback_command: None,
                fallback_args: None,
            },
            machine: Machine {
                platform,
                quoting: Quoting::local(platform, shell),
            },
            line,
        });
    }
    cases
}

fn reference() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/js-reference/launch-random.json");
    let text = std::fs::read_to_string(&path).expect("the recording is checked in");
    serde_json::from_str(&text).expect("the recording is JSON")
}

fn outcome<T>(result: Result<T, launch::LaunchError>, ok: impl Fn(T) -> Value) -> Value {
    match result {
        Ok(v) => ok(v),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

#[test]
fn agrees_with_the_recording_on_seeded_random_lines_refusals_included() {
    let reference = reference();
    let env = vec![("PATH".to_owned(), "/nonexistent-vorn-bin".to_owned())];
    for (i, case) in cases(CASES).iter().enumerate() {
        let name = format!("random {i}");
        let want = &reference[&name];
        assert!(want.is_object(), "{name} is not in the recording");

        let line = outcome(
            launch::launch_line(&case.req, Some(&case.config), &env, &case.machine),
            Value::String,
        );
        assert_eq!(line, want["line"], "line: {name}, {:?}", case.line);
        let headless = outcome(
            launch::headless_spawn(&case.req, Some(&case.config), &env, &case.machine),
            |h| match h.stdin {
                Some(stdin) => json!({ "command": h.command, "args": h.args, "stdin": stdin }),
                None => json!({ "command": h.command, "args": h.args }),
            },
        );
        assert_eq!(
            headless, want["headless"],
            "headless: {name}, {:?}",
            case.line
        );
        let tokens = launch::tokenize(&case.line).map(|tokens| {
            tokens
                .into_iter()
                .map(|t| json!({ "raw": t.raw, "value": t.value }))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            json!(tokens),
            want["tokens"],
            "tokens: {name}, {:?}",
            case.line
        );
    }
}
