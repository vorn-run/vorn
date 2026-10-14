//! A test vornd reached as any client reaches it: the port and token its data directory publishes.

use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

pub struct Vornd {
    port: u64,
    token: String,
}

impl Vornd {
    pub fn open(data_dir: &str) -> Result<Vornd, String> {
        let port: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{data_dir}/ws-port")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let token = std::fs::read_to_string(format!("{data_dir}/local-token"))
            .map_err(|e| e.to_string())?;
        Ok(Vornd {
            port: port["port"].as_u64().ok_or("no port")?,
            token: token.trim().to_owned(),
        })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut req = format!("ws://127.0.0.1:{}/ws", self.port)
            .into_client_request()
            .map_err(|e| e.to_string())?;
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.token)
                .parse()
                .map_err(|_| "token")?,
        );
        let (mut ws, _) = tungstenite::connect(req).map_err(|e| e.to_string())?;
        let frame = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        ws.send(Message::text(frame.to_string()))
            .map_err(|e| e.to_string())?;
        loop {
            let msg = ws.read().map_err(|e| e.to_string())?;
            let Message::Text(t) = msg else { continue };
            let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
            if v.get("id") == Some(&json!(1)) {
                if let Some(err) = v.get("error") {
                    return Err(err.to_string());
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}
