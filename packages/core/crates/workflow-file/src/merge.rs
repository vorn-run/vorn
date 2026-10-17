//! A workflow's changes written into its file: keys and `[[step]]`s (by `id`) that say the same keep their text and comments.

use serde_json::{Map, Value};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::read::read;
use crate::write::write;
use crate::Bindings;

/// `old`, a workflow file, changed to say `definition`. A file that does not
/// read is written fresh.
pub fn update(old: &str, definition: &Value, bindings: &Bindings<'_>) -> String {
    let fresh = write(definition, bindings);
    let Ok(was) = read(old, bindings) else {
        return fresh;
    };
    let canon = write(&was.workflow, bindings);
    if canon == fresh {
        return old.to_owned();
    }
    let (Ok(o), Ok(c), Ok(n)) = (
        old.parse::<DocumentMut>(),
        canon.parse::<DocumentMut>(),
        fresh.parse::<DocumentMut>(),
    ) else {
        return fresh;
    };
    let mut root = merge(o.as_table(), c.as_table(), n.as_table());
    let mut next = 0;
    renumber(&mut root, &mut next);
    let mut doc = DocumentMut::from(root);
    doc.set_trailing(o.trailing().clone());
    doc.to_string()
}

/// New table `n`, kept as old table `o` where it says what `c` (`o` written fresh) says.
fn merge(o: &Table, c: &Table, n: &Table) -> Table {
    let mut out = Table::new();
    out.set_implicit(n.is_implicit());
    out.set_dotted(n.is_dotted());
    *out.decor_mut() = o.decor().clone();
    for (key, new) in n.iter() {
        let kept = match (o.get_key_value(key), c.get(key)) {
            (Some((old_key, old)), Some(canon)) => {
                let item = if same(canon, new) {
                    old.clone()
                } else {
                    match (old, canon, new) {
                        (Item::Table(ot), Item::Table(ct), Item::Table(nt)) => {
                            Item::Table(merge(ot, ct, nt))
                        }
                        (
                            Item::ArrayOfTables(oa),
                            Item::ArrayOfTables(ca),
                            Item::ArrayOfTables(na),
                        ) => Item::ArrayOfTables(merge_list(oa, ca, na)),
                        _ => new.clone(),
                    }
                };
                Some((old_key.clone(), item))
            }
            _ => None,
        };
        match kept {
            Some((k, item)) => {
                out.insert_formatted(&k, item);
            }
            None => {
                if let Some(k) = n.key(key) {
                    out.insert_formatted(k, new.clone());
                }
            }
        }
    }
    out
}

/// A list of tables, each new one merged with the old one of its id.
fn merge_list(o: &ArrayOfTables, c: &ArrayOfTables, n: &ArrayOfTables) -> ArrayOfTables {
    let mut out = ArrayOfTables::new();
    for (i, nt) in n.iter().enumerate() {
        let id = id_of(nt);
        match (find(o, i, id), find(c, i, id)) {
            (Some(ot), Some(ct)) => out.push(merge(ot, ct, nt)),
            _ => out.push(nt.clone()),
        }
    }
    out
}

fn id_of(t: &Table) -> Option<&str> {
    t.get("id").and_then(Item::as_str)
}

/// The one table of `list` with `id`; without an id, the id-less table at the same place.
fn find<'a>(list: &'a ArrayOfTables, at: usize, id: Option<&str>) -> Option<&'a Table> {
    match id {
        Some(id) => {
            let mut matching = list.iter().filter(|t| id_of(t) == Some(id));
            match (matching.next(), matching.next()) {
                (Some(t), None) => Some(t),
                _ => None,
            }
        }
        None => list.get(at).filter(|t| id_of(t).is_none()),
    }
}

/// Whether two items say the same, however each is laid out.
fn same(a: &Item, b: &Item) -> bool {
    plain(a) == plain(b)
}

/// An item's meaning without its layout: what [`same`] compares.
fn plain(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(v) => plain_value(v),
        Item::Table(t) => plain_table(t),
        Item::ArrayOfTables(a) => Value::Array(a.iter().map(plain_table).collect()),
    }
}

fn plain_table(t: &Table) -> Value {
    Value::Object(
        t.iter()
            .map(|(k, v)| (k.to_owned(), plain(v)))
            .collect::<Map<_, _>>(),
    )
}

fn plain_value(v: &toml_edit::Value) -> Value {
    match v {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::from(*i.value()),
        // As text, so a float of any value compares.
        toml_edit::Value::Float(f) => Value::String(format!("{:?}", f.value())),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(plain_value).collect()),
        toml_edit::Value::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), plain_value(v)))
                .collect::<Map<_, _>>(),
        ),
    }
}

/// Every table placed in the order it is reached, so old and fresh tables print in the new order.
fn renumber(table: &mut Table, next: &mut isize) {
    table.set_position(Some(*next));
    *next += 1;
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(t) => renumber(t, next),
            Item::ArrayOfTables(a) => {
                for t in a.iter_mut() {
                    renumber(t, next);
                }
            }
            _ => {}
        }
    }
}
