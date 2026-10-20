use std::cell::RefCell;
use std::rc::Rc;

use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::tree::{self, Event, EventKind, Kind, Node};
use vorn_extension_sdk::{dispatch, json, Ctx, Value};

use super::*;

/// Vorn's hosts and keys, as the core keeps them, shared with its answers.
#[derive(Clone, Default)]
struct Kept {
    hosts: Rc<RefCell<Vec<Value>>>,
    keys: Rc<RefCell<Vec<Value>>>,
}

fn dev() -> Value {
    json!({ "id": "h1", "label": "Dev", "hostname": "dev.example.com", "user": "ubuntu",
            "port": 22, "authMethod": "agent" })
}

/// Vorn with host `h1` and key `k1`; a test fails only on `nowhere.invalid`.
fn host(kept: &Kept) -> FakeHost {
    kept.hosts.borrow_mut().push(dev());
    kept.keys.borrow_mut().push(json!({
        "id": "k1", "label": "Work", "keyType": "ed25519",
        "publicKey": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample work@laptop",
    }));
    let (hosts, saved, keys) = (kept.hosts.clone(), kept.hosts.clone(), kept.keys.clone());
    let (stored, deleted) = (kept.keys.clone(), kept.keys.clone());
    FakeHost::default()
        .answer("remote.hosts", move |_| {
            Ok(Value::Array(hosts.borrow().clone()))
        })
        .answer("remote.saveHosts", move |a| {
            *saved.borrow_mut() = a["hosts"].as_array().cloned().unwrap_or_default();
            Ok(Value::Null)
        })
        .answer("remote.keys", move |_| {
            Ok(Value::Array(keys.borrow().clone()))
        })
        .answer("remote.storeKey", move |a| {
            let mut k = a.clone();
            k["id"] = json!("k2");
            stored.borrow_mut().push(k);
            Ok(json!({ "id": "k2" }))
        })
        .answer("remote.importKey", |a| match a["path"].as_str() {
            Some("~/.ssh/id_ed25519") => Ok(json!({ "id": "k3" })),
            _ => Err(Error::new("failed", "Could not read that key file")),
        })
        .answer("remote.deleteKey", move |a| {
            deleted.borrow_mut().retain(|k| k["id"] != a["id"]);
            Ok(Value::Null)
        })
        .answer("remote.test", |a| {
            if a.get("then").is_some() {
                return Ok(json!({ "testing": true }));
            }
            Ok(match a["host"]["hostname"].as_str() {
                Some("nowhere.invalid") => {
                    json!({ "success": false, "message": "could not resolve", "durationMs": 3 })
                }
                _ => json!({ "success": true, "message": "Connected", "durationMs": 40 }),
            })
        })
}

fn call(s: &mut Ssh, h: &mut FakeHost, method: &str, args: Value) -> Result<Value, Error> {
    dispatch(s, h, method, args, &Ctx::default())
}

fn render(s: &mut Ssh, h: &mut FakeHost) -> Node {
    let args = json!({ "point": PAGE, "surface": { "width": 600 } });
    let v = call(s, h, tree::RENDER, args).expect("renders");
    let node = tree::read(&v).expect("a tree").tree;
    tree::check(&node).expect("a valid tree");
    node
}

fn tell(s: &mut Ssh, h: &mut FakeHost, id: &str, kind: EventKind, value: &str) -> Value {
    let e = Event::new(id, kind, value);
    call(s, h, tree::EVENT, json!({ "point": PAGE, "event": e })).expect("told")
}

fn click(s: &mut Ssh, h: &mut FakeHost, id: &str) -> Value {
    tell(s, h, id, EventKind::Click, "")
}

/// Opens (`true`) or closes a row as a click on its disclosure does.
fn disclose(s: &mut Ssh, h: &mut FakeHost, id: &str, open: bool) -> Value {
    tell(s, h, id, EventKind::Toggle, &open.to_string())
}

/// Every node in `n`, depth first.
fn walk<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    out.push(n);
    for c in &n.children {
        walk(c, out);
    }
    if let Kind::Entry(e) = &n.kind {
        for b in e.actions.iter().chain(&e.more) {
            walk(b, out);
        }
    }
}

fn find<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    let mut all = Vec::new();
    walk(n, &mut all);
    all.into_iter().find(|n| n.id.as_deref() == Some(id))
}

fn texts(n: &Node) -> Vec<String> {
    let mut all = Vec::new();
    walk(n, &mut all);
    all.into_iter()
        .filter_map(|n| match &n.kind {
            Kind::Text { text, .. } => Some(text.clone()),
            Kind::Button { label, .. } => Some(label.clone()),
            Kind::Notice { text, .. } => Some(text.clone()),
            Kind::Badge(b) => Some(b.text.clone()),
            _ => None,
        })
        .collect()
}

fn icon_of<'a>(n: &'a Node, id: &str) -> Option<&'a str> {
    match &find(n, id)?.kind {
        Kind::Button { icon, .. } => icon.as_deref(),
        _ => None,
    }
}

fn shows(n: &Node, line: &str) -> bool {
    texts(n).iter().any(|t| t == line)
}

#[test]
fn the_page_lists_hosts_and_keys_closed() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    let page = render(&mut s, &mut h);
    assert!(find(&page, "host:h1").is_some());
    assert!(find(&page, "keyrow:k1").is_some());
    assert!(
        find(&page, "label:h1").is_none(),
        "a closed host shows no form"
    );
    assert!(shows(&page, "Add Host"));
    assert!(shows(&page, "Import Key") && shows(&page, "Paste Key"));
}

#[test]
fn nothing_kept_says_so() {
    let mut h = FakeHost::default()
        .answer("remote.hosts", |_| Ok(json!([])))
        .answer("remote.keys", |_| Ok(json!([])));
    let page = render(&mut Ssh::default(), &mut h);
    let mut all = Vec::new();
    walk(&page, &mut all);
    let empties: Vec<_> = all
        .iter()
        .filter_map(|n| match &n.kind {
            Kind::List { empty, .. } => empty.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(empties, ["No remote hosts yet.", "No stored keys yet."]);
}

#[test]
fn opening_a_host_shows_its_form_as_kept() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    let page = render(&mut s, &mut h);
    let Some(Node {
        kind: Kind::Field { value, .. },
        ..
    }) = find(&page, "hostname:h1")
    else {
        panic!("no hostname field");
    };
    assert_eq!(value, "dev.example.com");
    assert!(shows(&page, "Uses your running SSH agent (ssh-agent)"));
    assert!(shows(&page, "Test Connection"));
    assert!(shows(&page, "Delete Host"), "in the form, as 0.8 had it");
    let Some(Node {
        kind: Kind::Entry(e),
        ..
    }) = find(&page, "host:h1")
    else {
        panic!("no host row");
    };
    assert_eq!(e.expanded, Some(true));
    assert!(
        e.actions.is_empty() && e.more.is_empty(),
        "no Edit or Close"
    );
    disclose(&mut s, &mut h, "host:h1", false);
    assert!(find(&render(&mut s, &mut h), "hostname:h1").is_none());
}

#[test]
fn typing_in_a_field_saves_the_host() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    tell(
        &mut s,
        &mut h,
        "hostname:h1",
        EventKind::Change,
        "prod.example.com",
    );
    tell(&mut s, &mut h, "port:h1", EventKind::Change, "2222");
    let saved = &kept.hosts.borrow()[0];
    assert_eq!(saved["hostname"], "prod.example.com");
    assert_eq!(saved["port"], 2222);
    assert_eq!(saved["label"], "Dev", "the rest is kept");
}

#[test]
fn switching_auth_shows_its_credential_and_saves() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "key-stored");
    assert_eq!(kept.hosts.borrow()[0]["authMethod"], "key-stored");
    let page = render(&mut s, &mut h);
    assert!(
        find(&page, "key:h1").is_some(),
        "the stored keys to pick from"
    );
    tell(&mut s, &mut h, "key:h1", EventKind::Select, "k1");
    assert_eq!(kept.hosts.borrow()[0]["credentialId"], "k1");
    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "password");
    let page = render(&mut s, &mut h);
    assert!(find(&page, "password:h1").is_some());
    tell(&mut s, &mut h, "password:h1", EventKind::Submit, "hunter2");
    assert_eq!(kept.hosts.borrow()[0]["password"], "hunter2");
}

#[test]
fn an_unknown_auth_method_is_refused() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    let e = Event::new("auth:h1", EventKind::Select, "telnet");
    let said = call(
        &mut s,
        &mut h,
        tree::EVENT,
        json!({ "point": PAGE, "event": e }),
    );
    assert!(said.is_err());
    assert_eq!(kept.hosts.borrow()[0]["authMethod"], "agent");
}

#[test]
fn a_host_without_an_address_is_not_tested() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    tell(&mut s, &mut h, "hostname:h1", EventKind::Change, "");
    click(&mut s, &mut h, "test:h1");
    assert!(h.asked("remote.test").is_empty());
    disclose(&mut s, &mut h, "host:h1", true);
    assert!(shows(&render(&mut s, &mut h), hosts::NEEDS_ADDRESS));
}

#[test]
fn a_test_runs_in_vornd_and_its_result_shows() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    click(&mut s, &mut h, "test:h1");
    let asked = h.asked("remote.test");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["then"], TESTED);
    assert_eq!(asked[0]["key"], "h1");
    assert_eq!(asked[0]["host"]["hostname"], "dev.example.com");
    let page = render(&mut s, &mut h);
    assert_eq!(icon_of(&page, "test:h1"), Some("loader-circle"), "testing");

    click(&mut s, &mut h, "test:h1");
    assert_eq!(h.asked("remote.test").len(), 1, "one test at a time");

    let result = json!({ "success": false, "message": "Permission denied", "durationMs": 9 });
    call(
        &mut s,
        &mut h,
        TESTED,
        json!({ "key": "h1", "result": result }),
    )
    .expect("told");
    assert_eq!(
        h.rows(TESTS).expect("rows").len(),
        1,
        "the page is drawn again"
    );
    let page = render(&mut s, &mut h);
    assert!(shows(&page, "Permission denied"));
    assert_eq!(icon_of(&page, "test:h1"), Some("wifi"));
}

#[test]
fn test_connection_answers_as_before() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    let asked =
        json!({ "id": "x", "label": "x", "hostname": "nowhere.invalid", "user": "u", "port": 22 });
    let said = call(&mut s, &mut h, TEST_CONNECTION, asked.clone()).expect("answers");
    assert_eq!(said["success"], false);
    assert_eq!(said["message"], "could not resolve");
    assert_eq!(h.asked("remote.test"), [json!({ "host": asked })]);
}

#[test]
fn add_and_delete_a_host() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    click(&mut s, &mut h, "add-host");
    let added = hosts::id(&kept.hosts.borrow()[1]).to_owned();
    assert!(!added.is_empty());
    assert_eq!(kept.hosts.borrow()[1]["port"], 22);
    let page = render(&mut s, &mut h);
    assert!(
        find(&page, &format!("label:{added}")).is_some(),
        "a new host opens"
    );
    click(&mut s, &mut h, "add-host");
    assert_ne!(hosts::id(&kept.hosts.borrow()[2]), added, "ids differ");
    click(&mut s, &mut h, "delete-host:h1");
    assert!(kept.hosts.borrow().iter().all(|h| hosts::id(h) != "h1"));
}

#[test]
fn paste_a_key() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    click(&mut s, &mut h, "paste");
    assert!(find(&render(&mut s, &mut h), "new-key:private").is_some());
    let said = click(&mut s, &mut h, "save-key");
    assert!(said["error"].is_string(), "no private key");
    assert!(h.asked("remote.storeKey").is_empty());
    let typed = tell(
        &mut s,
        &mut h,
        "new-key:private",
        EventKind::Change,
        " PRIV \n",
    );
    assert_eq!(typed, json!({ "same": true }));
    click(&mut s, &mut h, "save-key");
    assert_eq!(
        h.asked("remote.storeKey"),
        [json!({ "label": "Untitled Key", "privateKey": "PRIV" })]
    );
    assert!(s.adding.is_none());
    assert!(find(&render(&mut s, &mut h), "keyrow:k2").is_some());
}

#[test]
fn import_and_delete_a_key() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    click(&mut s, &mut h, "import");
    assert!(find(&render(&mut s, &mut h), "import-path").is_some());
    let e = Event::new("import-path", EventKind::Submit, "/nope");
    let said = call(
        &mut s,
        &mut h,
        tree::EVENT,
        json!({ "point": PAGE, "event": e }),
    );
    assert!(said.is_err(), "a key file that cannot be read says so");
    assert!(s.importing, "and the field stays");
    tell(
        &mut s,
        &mut h,
        "import-path",
        EventKind::Submit,
        "~/.ssh/id_ed25519",
    );
    assert!(!s.importing);

    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "key-stored");
    tell(&mut s, &mut h, "key:h1", EventKind::Select, "k1");
    disclose(&mut s, &mut h, "keyrow:k1", true);
    let page = render(&mut s, &mut h);
    assert!(shows(&page, "Public Key"));
    assert!(
        texts(&page).iter().any(|t| t == "Dev"),
        "the hosts that use it"
    );
    click(&mut s, &mut h, "delete-key:k1");
    assert!(kept.keys.borrow().is_empty());
}

#[test]
fn the_password_saves_when_left() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "password");
    let page = render(&mut s, &mut h);
    let Some(Node {
        kind: Kind::Field { secret, blur, .. },
        ..
    }) = find(&page, "password:h1")
    else {
        panic!("no password field");
    };
    assert!(*secret && *blur, "a hidden field that submits on blur");
    assert!(shows(&page, "Encrypted with your OS keychain"));
}

#[test]
fn the_stored_key_is_picked_from_a_dropdown_that_starts_with_none() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "key-stored");
    let page = render(&mut s, &mut h);
    let Some(Node {
        kind: Kind::Choice { options, dropdown },
        ..
    }) = find(&page, "key:h1")
    else {
        panic!("no stored key choice");
    };
    assert!(*dropdown);
    let chosen: Vec<&str> = options
        .iter()
        .filter(|o| o.selected)
        .map(|o| o.id.as_str())
        .collect();
    assert_eq!(chosen, [NO_KEY]);
    let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Select a key...", "Work (ed25519)"]);
    tell(&mut s, &mut h, "key:h1", EventKind::Select, "k1");
    assert_eq!(kept.hosts.borrow()[0]["credentialId"], "k1");
    tell(&mut s, &mut h, "key:h1", EventKind::Select, NO_KEY);
    assert_eq!(kept.hosts.borrow()[0]["credentialId"], "");
}

#[test]
fn browse_picks_a_key_file_for_a_host_and_to_import() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    disclose(&mut s, &mut h, "host:h1", true);
    tell(&mut s, &mut h, "auth:h1", EventKind::Select, "key-file");
    let page = render(&mut s, &mut h);
    let Some(Node {
        kind: Kind::Button { pick, .. },
        ..
    }) = find(&page, "browse-key:h1")
    else {
        panic!("no browse button");
    };
    assert!(pick.is_some(), "it asks Vorn for a file");
    tell(
        &mut s,
        &mut h,
        "browse-key:h1",
        EventKind::Submit,
        "/k/id_rsa",
    );
    assert_eq!(kept.hosts.borrow()[0]["sshKeyPath"], "/k/id_rsa");

    click(&mut s, &mut h, "import");
    let page = render(&mut s, &mut h);
    assert!(find(&page, "import-path").is_some(), "the path field stays");
    assert!(matches!(
        &find(&page, "import-browse").expect("browse").kind,
        Kind::Button { pick: Some(_), .. }
    ));
    tell(
        &mut s,
        &mut h,
        "import-browse",
        EventKind::Submit,
        "~/.ssh/id_ed25519",
    );
    assert_eq!(
        h.asked("remote.importKey"),
        [json!({ "path": "~/.ssh/id_ed25519" })]
    );
    assert!(!s.importing);
}

#[test]
fn without_the_keychain_the_page_warns_and_offers_no_key_to_add() {
    let kept = Kept::default();
    let mut s = Ssh::default();
    let mut h = host(&kept).answer("remote.vault", |_| Ok(json!({ "keychain": false })));
    let page = render(&mut s, &mut h);
    assert!(shows(&page, NO_KEYCHAIN));
    assert!(!shows(&page, "Import Key") && !shows(&page, "Paste Key"));

    let mut h = host(&Kept::default()).answer("remote.vault", |_| Ok(json!({ "keychain": true })));
    let page = render(&mut s, &mut h);
    assert!(!shows(&page, NO_KEYCHAIN));
}

#[test]
fn paste_key_hides_while_its_form_shows() {
    let kept = Kept::default();
    let (mut s, mut h) = (Ssh::default(), host(&kept));
    click(&mut s, &mut h, "paste");
    let page = render(&mut s, &mut h);
    assert!(!shows(&page, "Paste Key") && shows(&page, "Import Key"));
    let Some(Node {
        kind: Kind::Field { lines, .. },
        ..
    }) = find(&page, "new-key:private")
    else {
        panic!("no private key field");
    };
    assert_eq!(*lines, 3);
}
