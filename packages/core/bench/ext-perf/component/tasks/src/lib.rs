//! Tasks as a Safe-tier component: vorn-work's Tasks code over vornd's storage bridge.

wit_bindgen::generate!({ world: "tasks", path: "../../wit" });

use vorn::ext_perf::bridge;

struct Tasks;

impl Guest for Tasks {
    fn call(
        method: String,
        params: String,
        now: String,
        id: String,
    ) -> Result<(Option<String>, bool), String> {
        let params: serde_json::Value =
            serde_json::from_str(&params).map_err(|e| format!("{method}: {e}"))?;
        let mut store = vorn_store::Store::new(|op, args| bridge::db(op, args));
        let (answer, changed) =
            vorn_ext_perf_guest::methods::call(&mut store, &method, &params, &now, id)
                .map_err(|e| e.to_string())?;
        Ok((answer.map(|v| v.to_string()), changed))
    }
}

export!(Tasks);
