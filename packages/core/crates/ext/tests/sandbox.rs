//! The safe tier survives what a component can do wrong: a call that never
//! ends, a trap, and a request to a host its manifest does not name. Runs
//! against the example's packed component; skips when it is not built
//! (`vorn ext pack ext/examples/pr-checks-rs`).
#![cfg(feature = "wasm")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use vorn_ext::bridge::World;
use vorn_ext::checks::{Recording, Replay};
use vorn_ext::manifest::Manifest;
use vorn_ext::quick::CallCtx;
use vorn_ext::wasm::{Instance, Runtime};

#[test]
fn a_component_survives_a_timeout_a_trap_and_a_denied_host() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ext/examples/pr-checks-rs");
    let cwasm = dir.join("pr-checks.cwasm");
    if !cwasm.exists() {
        eprintln!(
            "skipped: {} is not built; run vorn ext pack first",
            cwasm.display()
        );
        return;
    }
    let m = Arc::new(Manifest::load(&dir).unwrap());
    let rt = Runtime::new().unwrap();
    let c = match rt.load(&cwasm) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipped: {e}, built by another Wasmtime or for another CPU");
            return;
        }
    };
    let world: Arc<dyn World> = Arc::new(Replay(Recording::default()));
    let mut i = Instance::new(rt, c, m, world).unwrap();
    let ctx = CallCtx {
        project: String::new(),
        caller: "test".into(),
    };

    let t = Instant::now();
    let e = i
        .call(
            "selftest.spin",
            &Value::Null,
            &ctx,
            Duration::from_millis(100),
        )
        .unwrap_err();
    assert_eq!(e.code, "timeout", "{e}");
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "the deadline held: {:?}",
        t.elapsed()
    );

    let e = i
        .call("selftest.trap", &Value::Null, &ctx, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(e.code, "trap", "{e}");

    let e = i
        .call("selftest.fetch", &Value::Null, &ctx, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(e.code, "denied", "{e}");
    assert!(
        e.message
            .contains("example.com is not in the manifest's network list"),
        "{e}"
    );

    // A fresh instance after the trap answers as before.
    assert_eq!(
        i.call("ping", &Value::Null, &ctx, Duration::from_secs(5))
            .unwrap(),
        Value::Null
    );
}
