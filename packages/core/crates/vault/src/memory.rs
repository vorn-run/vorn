//! A keychain in memory: where nothing may outlive the process, and the fake
//! the tests of everything that keeps secrets use.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::{Error, Keychain, Kind, Op, Result, Secret};

/// Items by kind and id, and every write made, in order.
#[derive(Debug, Default)]
pub struct Memory {
    items: Mutex<BTreeMap<(Kind, String), Secret>>,
    writes: Mutex<Vec<String>>,
    failing: AtomicBool,
}

impl Memory {
    /// How many items it holds.
    pub fn len(&self) -> usize {
        self.items().len()
    }

    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }

    /// The writes made so far, as `set <id>` and `delete <id>`.
    pub fn writes(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Makes every write and delete fail, as a locked keychain does.
    pub fn fail_writes(&self, failing: bool) {
        self.failing.store(failing, Ordering::Release);
    }

    fn items(&self) -> MutexGuard<'_, BTreeMap<(Kind, String), Secret>> {
        self.items.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, op: Op, kind: Kind, id: &str) -> Result<()> {
        if self.failing.load(Ordering::Acquire) {
            return Err(Error::Failed {
                op,
                kind,
                reason: "the keychain is locked".into(),
            });
        }
        let verb = if op == Op::Delete { "delete" } else { "set" };
        self.writes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!("{verb} {id}"));
        Ok(())
    }
}

impl Keychain for Memory {
    fn get(&self, kind: Kind, id: &str) -> Result<Option<Secret>> {
        Ok(self.items().get(&(kind, id.to_owned())).cloned())
    }

    fn set(&self, kind: Kind, id: &str, secret: &Secret) -> Result<()> {
        self.write(Op::Write, kind, id)?;
        self.items().insert((kind, id.to_owned()), secret.clone());
        Ok(())
    }

    fn delete(&self, kind: Kind, id: &str) -> Result<()> {
        self.write(Op::Delete, kind, id)?;
        self.items().remove(&(kind, id.to_owned()));
        Ok(())
    }
}
