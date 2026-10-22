//! The Rust extension of the extension load test, doing the Node and Python
//! services' work: each `work` call asks Vorn for its sessions, as a sidebar
//! would, and it counts the session events it is woken for.
//!
//! The load test runs it both ways: as a Wasm component inside vornd, and as
//! a program on stdio (`vorn-bench-ext-tiers`' `tier-load`).

use vorn_extension_sdk::{export, json, Ctx, Error, Event, Extension, Host, Init, Value};

/// The extension; `heard` is how many events it has been woken for.
pub struct Load {
    heard: u64,
}

impl Extension for Load {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Load, Error> {
        Ok(Load { heard: 0 })
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        _: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        match method {
            "work" => {
                let sessions = host.call("sessions.list", json!({}))?;
                Ok(json!({ "bytes": sessions.to_string().len(), "heard": self.heard }))
            }
            other => Err(Error::unknown(other)),
        }
    }

    fn on_events(&mut self, _: &mut dyn Host, batch: Vec<Event>) {
        self.heard += batch.len() as u64;
    }
}

export!(Load);

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::testing::FakeHost;

    fn ctx() -> Ctx {
        Ctx {
            caller: "bench".into(),
            project: None,
        }
    }

    #[test]
    fn work_asks_for_the_sessions_and_reports_the_events_heard() {
        let mut host = FakeHost::default().answer("sessions.list", |_| Ok(json!([{ "id": "s1" }])));
        let mut ext = Load { heard: 0 };
        let event = || Event {
            name: "session.status".into(),
            data: Value::Null,
        };
        ext.on_events(&mut host, vec![event(), event()]);
        let answer = ext.handle(&mut host, "work", Value::Null, &ctx());
        assert_eq!(answer, Ok(json!({ "bytes": 13, "heard": 2 })));
        assert_eq!(host.asked("sessions.list").len(), 1);
    }

    #[test]
    fn any_other_action_is_unknown() {
        let mut host = FakeHost::default();
        let mut ext = Load { heard: 0 };
        let answer = ext.handle(&mut host, "nope", Value::Null, &ctx());
        assert_eq!(answer, Err(Error::unknown("nope")));
    }
}
