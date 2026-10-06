//! The owner and device tokens: the rows a device's credential is checked
//! against.
//!
//! The SQL is here rather than on [`Store`](crate::Store) alone because a
//! process beside the server (vornd) reads and writes the same rows from the
//! same file: [`DeviceTokens`] opens an existing database for that, and
//! creates no schema, runs no migration and seeds nothing, all of which stay
//! the server's.

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row};
use vorn_protocol::{DeviceToken, DeviceTokenSecret, NewDeviceToken, User, UserRole};

use crate::sql::{get_opt_text, get_text};
use crate::Result;

pub(crate) fn owner_user(conn: &Connection) -> Result<Option<User>> {
    conn.query_row(
        "SELECT * FROM users WHERE role = 'owner' ORDER BY created_at LIMIT 1",
        [],
        |row| Ok(to_user(row)),
    )
    .optional()?
    .transpose()
}

fn to_user(row: &Row<'_>) -> Result<User> {
    Ok(User {
        id: get_text(row, "id")?,
        name: get_text(row, "name")?,
        role: UserRole(get_text(row, "role")?),
        created_at: get_text(row, "created_at")?,
    })
}

pub(crate) fn insert(conn: &Connection, token: &NewDeviceToken) -> Result<()> {
    conn.execute(
        "INSERT INTO device_tokens (id, user_id, name, token_hash, created_at)
       VALUES (?, ?, ?, ?, ?)",
        params![
            token.id,
            token.user_id,
            token.name,
            token.token_hash,
            token.created_at
        ],
    )?;
    Ok(())
}

pub(crate) fn secret(conn: &Connection, id: &str) -> Result<Option<DeviceTokenSecret>> {
    conn.query_row(
        "SELECT id, user_id, token_hash, revoked_at FROM device_tokens WHERE id = ?",
        [id],
        |row| Ok(to_secret(row)),
    )
    .optional()?
    .transpose()
}

fn to_secret(row: &Row<'_>) -> Result<DeviceTokenSecret> {
    Ok(DeviceTokenSecret {
        id: get_text(row, "id")?,
        user_id: get_text(row, "user_id")?,
        token_hash: get_text(row, "token_hash")?,
        revoked_at: get_opt_text(row, "revoked_at")?,
    })
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<DeviceToken>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_id, name, created_at, last_seen_at, revoked_at
       FROM device_tokens ORDER BY created_at",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(DeviceToken {
            id: get_text(row, "id")?,
            user_id: get_text(row, "user_id")?,
            name: get_text(row, "name")?,
            created_at: get_text(row, "created_at")?,
            last_seen_at: get_opt_text(row, "last_seen_at")?,
            revoked_at: get_opt_text(row, "revoked_at")?,
        });
    }
    Ok(out)
}

pub(crate) fn any(conn: &Connection) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM device_tokens LIMIT 1", [], |row| row.get(0))
        .optional()?;
    Ok(found.is_some())
}

pub(crate) fn revoke(conn: &Connection, id: &str, revoked_at: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE device_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        params![revoked_at, id],
    )?;
    Ok(changed > 0)
}

pub(crate) fn touch(conn: &Connection, id: &str, seen_at: &str) -> Result<()> {
    conn.execute(
        "UPDATE device_tokens SET last_seen_at = ? WHERE id = ?",
        params![seen_at, id],
    )?;
    Ok(())
}

/// The owner and device tokens of a database the server keeps, opened by a
/// second process. Each call reads the file as it is now.
pub struct DeviceTokens {
    conn: Connection,
}

impl DeviceTokens {
    /// Opens the database at `path` for reading and writing these rows.
    /// `None` when there is no such file, or it has no `device_tokens`
    /// table yet (the server has not migrated it): the caller cannot tell,
    /// and leaves the call to the server.
    pub fn open(path: &Path) -> Result<Option<DeviceTokens>> {
        if !path.exists() {
            return Ok(None);
        }
        // No CREATE: a file that went away since is not one to make.
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('users', 'device_tokens')",
            [],
            |row| row.get(0),
        )?;
        if tables < 2 {
            return Ok(None);
        }
        Ok(Some(DeviceTokens { conn }))
    }

    /// The seeded owner.
    pub fn owner(&self) -> Result<Option<User>> {
        owner_user(&self.conn)
    }

    pub fn insert(&self, token: &NewDeviceToken) -> Result<()> {
        insert(&self.conn, token)
    }

    /// The hash and state of token `id`, for verification only.
    pub fn secret(&self, id: &str) -> Result<Option<DeviceTokenSecret>> {
        secret(&self.conn, id)
    }

    /// Every token, oldest first, without its hash.
    pub fn list(&self) -> Result<Vec<DeviceToken>> {
        list(&self.conn)
    }

    /// False when `id` is unknown or was already revoked.
    pub fn revoke(&self, id: &str, revoked_at: &str) -> Result<bool> {
        revoke(&self.conn, id, revoked_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_support, Store};

    #[test]
    fn opens_only_a_database_the_server_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        assert!(DeviceTokens::open(&path).unwrap().is_none());
        assert!(!path.exists(), "opening created the file");

        let (store, _) = Store::open(&path, test_support::options()).unwrap();
        let owner = store.db_get_owner_user().unwrap().unwrap();
        let beside = DeviceTokens::open(&path).unwrap().unwrap();
        assert_eq!(beside.owner().unwrap().unwrap().id, owner.id);

        beside
            .insert(&NewDeviceToken {
                id: "t1".into(),
                user_id: owner.id.clone(),
                name: "Phone".into(),
                token_hash: "ab".into(),
                created_at: "2026-10-05T00:00:00.000Z".into(),
            })
            .unwrap();
        // The server's connection sees what the second one wrote.
        let json = |t: Vec<DeviceToken>| serde_json::to_value(t).unwrap();
        assert_eq!(
            json(store.db_list_device_tokens().unwrap()),
            json(beside.list().unwrap())
        );
        assert_eq!(
            json(beside.list().unwrap())[0]["lastSeenAt"],
            serde_json::Value::Null
        );
        assert_eq!(beside.secret("t1").unwrap().unwrap().token_hash, "ab");
        assert!(beside.revoke("t1", "2026-10-05T00:00:01.000Z").unwrap());
        assert!(!beside.revoke("t1", "2026-10-05T00:00:02.000Z").unwrap());
        assert_eq!(
            store
                .db_get_device_token_secret("t1")
                .unwrap()
                .unwrap()
                .revoked_at
                .as_deref(),
            Some("2026-10-05T00:00:01.000Z")
        );
    }
}
