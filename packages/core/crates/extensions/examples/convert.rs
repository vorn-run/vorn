//! Prints a 0.8 pack's `manifest.json` as `vorn-extension.toml`, the way each
//! connector moves to the one manifest: `cargo run -p vorn-extensions
//! --example convert -- manifest.json`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: convert <manifest.json>");
        return ExitCode::from(2);
    };
    let read = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())
        });
    let manifest = match read {
        Ok(serde_json::Value::Object(payload)) => {
            vorn_extensions::manifest::file::from_pack_json(&payload).map_err(|e| e.to_string())
        }
        Ok(_) => Err(format!("{path} is not a JSON object")),
        Err(e) => Err(e),
    };
    match manifest {
        Ok(m) => {
            print!("{}", vorn_extensions::manifest::file::to_toml(&m));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            ExitCode::FAILURE
        }
    }
}
