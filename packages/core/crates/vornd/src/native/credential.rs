//! SSH keys and remote hosts' passwords, kept in the vault
//! ([`super::secrets`]): `credential:*`, the passwords a configuration save
//! carries, and the ones the desktop sealed before vornd kept them
//! (`credentials:import`). A row holds [`IN_VAULT`] where the secret was;
//! the secret itself is never in a row, an answer or a log line.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use tracing::info;
use vorn_connectors::connections::IN_VAULT;
use vorn_store::Store;
use vorn_vault::{Kind, Secret};

use super::config::{blocking, with_store};
use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &[
    "credential:storeKey",
    "credential:listKeys",
    "credential:deleteKey",
    "credential:getEncryptedKey",
];

/// The kind of key a private key's text names, as the desktop read it.
fn key_type(private_key: &str) -> Option<&'static str> {
    [
        ("ED25519", "ed25519"),
        ("RSA", "rsa"),
        ("ECDSA", "ecdsa"),
        ("DSA", "dsa"),
    ]
    .iter()
    .find(|(marker, _)| private_key.contains(marker))
    .map(|(_, kind)| *kind)
}

fn text(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    if native.database().is_none() {
        return Answer::Forward;
    }
    let n = Arc::clone(native);
    let m = method.to_owned();
    match blocking(method, move || call(&n, &m, &params)).await {
        Ok(Value::Null) => Answer::Void,
        Ok(value) => Answer::Result(value),
        Err(message) => Answer::Error(message),
    }
}

fn call(native: &Native, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "credential:storeKey" => {
            let private = text(params, "privateKey").ok_or("an SSH key needs its private key")?;
            let id = uuid::Uuid::new_v4().to_string();
            native
                .secrets
                .put_item(Kind::SshKey, &id, &Secret::new(private.as_str()))?;
            let key = vorn_protocol::SshKey {
                id: id.clone(),
                label: text(params, "label").unwrap_or_default(),
                encrypted_private_key: IN_VAULT.to_owned(),
                public_key: text(params, "publicKey"),
                certificate: text(params, "certificate"),
                key_type: text(params, "keyType").or_else(|| key_type(&private).map(str::to_owned)),
                created_at: vorn_work::js::iso_now(),
            };
            let saved = with_store(native, |s| s.db_save_ssh_key(&key));
            if saved.is_err() {
                native.secrets.remove_item(Kind::SshKey, &id);
            }
            saved?;
            Ok(json!({ "id": id }))
        }
        "credential:listKeys" => with_store(native, |s| {
            Ok(serde_json::to_value(s.db_list_ssh_keys()?).unwrap_or(Value::Null))
        }),
        "credential:deleteKey" => {
            let id = params.as_str().unwrap_or_default().to_owned();
            with_store(native, |s| s.db_delete_ssh_key(&id))?;
            native.secrets.remove_item(Kind::SshKey, &id);
            Ok(Value::Null)
        }
        // What the desktop reads to hand over a key it sealed: the marker once vornd keeps it.
        "credential:getEncryptedKey" => {
            let id = params.as_str().unwrap_or_default().to_owned();
            with_store(native, |s| {
                Ok(serde_json::to_value(s.db_get_ssh_key(&id)?).unwrap_or(Value::Null))
            })
        }
        _ => Err(format!("vornd does not answer {method}")),
    }
}

/// Takes the passwords a configuration carries out of it (`password` on a
/// remote host), leaving [`IN_VAULT`] where each was, to be filed by host id.
pub fn take_host_passwords(config: &mut Value) -> Vec<(String, Secret)> {
    let mut taken = Vec::new();
    let hosts = config
        .get_mut("remoteHosts")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten();
    for host in hosts.filter_map(Value::as_object_mut) {
        let Some(password) = host.remove("password") else {
            continue;
        };
        let (Some(id), Some(password)) = (
            host.get("id").and_then(Value::as_str).map(str::to_owned),
            password.as_str().filter(|p| !p.is_empty()),
        ) else {
            continue;
        };
        taken.push((id, Secret::new(password)));
        host.insert("encryptedPassword".into(), json!(IN_VAULT));
    }
    taken
}

fn vaulted_hosts(config: &Value) -> Vec<&str> {
    config
        .get("remoteHosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|h| h.get("encryptedPassword").and_then(Value::as_str) == Some(IN_VAULT))
        .filter_map(|h| h.get("id").and_then(Value::as_str))
        .collect()
}

/// The hosts whose password the vault held before a save and no longer
/// should: gone, or no longer logging in by password.
pub fn dropped_host_passwords(before: &Value, after: &Value) -> Vec<String> {
    let kept = vaulted_hosts(after);
    vaulted_hosts(before)
        .into_iter()
        .filter(|id| !kept.contains(id))
        .map(str::to_owned)
        .collect()
}

/// `credentials:import`'s SSH keys and host passwords: what the desktop
/// decrypted of the ones it sealed, filed once, each row then holding the
/// marker. How many of each were filed.
pub fn import(native: &Native, store: &mut Store, params: &Value) -> Result<(u64, u64), String> {
    let entries = |key: &str| -> Vec<(String, String)> {
        params
            .get(key)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(id, v)| {
                Some((id.clone(), v.as_str().filter(|v| !v.is_empty())?.to_owned()))
            })
            .collect()
    };
    let mut keys = 0;
    for (id, private) in entries("sshKeys") {
        let Some(mut row) = store.db_get_ssh_key(&id).map_err(|e| e.to_string())? else {
            continue;
        };
        native
            .secrets
            .put_item(Kind::SshKey, &id, &Secret::new(private))?;
        row.encrypted_private_key = IN_VAULT.to_owned();
        store.db_delete_ssh_key(&id).map_err(|e| e.to_string())?;
        store.db_save_ssh_key(&row).map_err(|e| e.to_string())?;
        keys += 1;
    }
    let passwords = entries("hostPasswords");
    let mut filed = 0;
    if !passwords.is_empty() {
        let mut config = store.load_config().map_err(|e| e.to_string())?;
        let hosts = config
            .get_mut("remoteHosts")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object_mut);
        let mut marked: Vec<&mut Map<String, Value>> = Vec::new();
        for host in hosts {
            let id = host.get("id").and_then(Value::as_str).unwrap_or_default();
            if let Some((_, password)) = passwords.iter().find(|(h, _)| h == id) {
                native
                    .secrets
                    .put_item(Kind::HostPassword, id, &Secret::new(password.as_str()))?;
                marked.push(host);
            }
        }
        for host in &mut marked {
            host.insert("encryptedPassword".into(), json!(IN_VAULT));
        }
        filed = marked.len() as u64;
        if filed > 0 {
            store.save_config(&config, &[]).map_err(|e| e.to_string())?;
        }
    }
    if keys + filed > 0 {
        info!(
            keys,
            passwords = filed,
            "SSH secrets the desktop sealed are in the vault"
        );
    }
    Ok((keys, filed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_host_s_password_out_of_the_configuration() {
        let mut config = json!({ "remoteHosts": [
            { "id": "a", "password": "pw-a", "encryptedPassword": "old" },
            { "id": "b", "encryptedPassword": IN_VAULT },
            { "id": "c", "password": "" },
        ]});
        let taken = take_host_passwords(&mut config);
        assert_eq!(taken.len(), 1);
        assert_eq!((taken[0].0.as_str(), taken[0].1.expose()), ("a", "pw-a"));
        assert_eq!(
            config["remoteHosts"],
            json!([
                { "id": "a", "encryptedPassword": IN_VAULT },
                { "id": "b", "encryptedPassword": IN_VAULT },
                { "id": "c" },
            ])
        );
        assert!(!config.to_string().contains("pw-a"));
    }

    #[test]
    fn drops_the_password_of_a_host_gone_or_no_longer_using_one() {
        let before = json!({ "remoteHosts": [
            { "id": "kept", "encryptedPassword": IN_VAULT },
            { "id": "gone", "encryptedPassword": IN_VAULT },
            { "id": "switched", "encryptedPassword": IN_VAULT },
            { "id": "sealed", "encryptedPassword": "old" },
        ]});
        let after = json!({ "remoteHosts": [
            { "id": "kept", "encryptedPassword": IN_VAULT },
            { "id": "switched", "authMethod": "agent" },
        ]});
        assert_eq!(
            dropped_host_passwords(&before, &after),
            ["gone", "switched"]
        );
    }

    #[test]
    fn names_a_key_by_its_text() {
        assert_eq!(
            key_type("-----BEGIN OPENSSH PRIVATE KEY----- ED25519"),
            Some("ed25519")
        );
        assert_eq!(key_type("-----BEGIN RSA PRIVATE KEY-----"), Some("rsa"));
        assert_eq!(key_type("opaque"), None);
    }
}
