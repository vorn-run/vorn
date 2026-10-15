use super::*;
use serde_json::json;

const CHECKS: &str = r#"// The current branch's pull request checks, failing first.
schema "vorn:view@1"

// Every row also has an id.
rows "checks" {
    name string
    workflow string
    state enum pass fail pending skipping cancel
    link url
}

column {
    text "{checks.state.fail} failing · {checks.state.pending} pending" role=secondary
    list "checks" empty="No pull request for this branch" {
        row {
            status "{row.state}" pass=good fail=bad pending=busy skipping=muted cancel=warn
            text "{row.name}" grow=#true
            button Open open="{row.link}"
        }
    }
}
"#;

const FILE: &str = "views/checks.kdl";

fn errors(text: &str) -> Vec<String> {
    file::parse(FILE, text, Options::default())
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn rows(list: Value) -> BTreeMap<String, Vec<Row>> {
    let rows = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| Row::from_value(r).unwrap())
        .collect();
    BTreeMap::from([("checks".to_owned(), rows)])
}

#[test]
fn reads_a_template_file_and_fills_its_bindings() {
    let t = file::parse(FILE, CHECKS, Options::default()).unwrap();
    assert_eq!(t.first(), Some("checks"));
    let Kind::Column(kids) = &t.view.kind else {
        panic!("a column")
    };
    let Kind::Text { value, .. } = &kids[0].kind else {
        panic!("a text")
    };
    let src = rows(json!([
        { "id": "a", "name": "build", "state": "fail" },
        { "id": "b", "name": "lint", "state": "pending" },
        { "id": "c", "name": "test", "state": "fail" }
    ]));
    assert_eq!(value.fill(None, &src), "2 failing · 1 pending");
    let json = file::to_json(FILE, CHECKS, Options::default()).unwrap();
    assert_eq!(
        json["view"]["column"]["children"][1]["list"]["item"]["row"]["children"][2]["button"]
            ["action"]["open"],
        "{row.link}"
    );
    assert_eq!(parse_value(&json, Options::default()).unwrap(), t);
}

#[test]
fn names_the_file_line_and_column_of_a_misspelt_node() {
    assert_eq!(
        errors(&CHECKS.replace("button Open", "buton Open")),
        ["views/checks.kdl:18:13: unknown node 'buton'; did you mean 'button'?"]
    );
}

#[test]
fn names_an_unknown_attribute_and_lists_what_a_node_takes() {
    let broken = CHECKS
        .replace("role=secondary", "rol=secondary")
        .replace("grow=#true", "size=12");
    assert_eq!(
        errors(&broken),
        [
            "views/checks.kdl:13:73: unknown attribute 'rol' on text; did you mean 'role'?",
            "views/checks.kdl:17:31: unknown attribute 'size' on text; text takes value, role, grow, when",
        ]
    );
}

#[test]
fn points_at_a_binding_to_a_missing_field_and_a_tone_for_a_missing_value() {
    let broken = CHECKS
        .replace("{row.name}", "{row.nmae}")
        .replace("pass=good", "passed=good");
    assert_eq!(
        errors(&broken),
        [
            "views/checks.kdl:16:34: 'passed' is not a value of the bound field; did you mean 'pass'?",
            "views/checks.kdl:17:18: '{row.nmae}': checks has no field 'nmae'; did you mean 'name'?",
        ]
    );
}

#[test]
fn a_row_binding_needs_an_item_or_a_session() {
    let outside = CHECKS.replace("{checks.state.fail} failing", "{row.name}");
    assert!(errors(&outside)[0]
        .ends_with("'{row.name}' means something only in a list's item or a session's template"));
    let footer = "schema \"vorn:view@1\"\nrows { branch string\n ok bool }\nrow { text \"{row.branch}\"; badge \"{row.ok}\" }\n";
    let t = file::parse("views/footer.kdl", footer, Options { session_row: true }).unwrap();
    assert_eq!(t.first(), Some("rows"));
}

#[test]
fn opens_only_a_url_field_or_an_https_link() {
    let broken = CHECKS.replace("open=\"{row.link}\"", "open=\"{row.name}\"");
    assert_eq!(
        errors(&broken),
        ["views/checks.kdl:18:25: open takes one field declared as url, or an https link"]
    );
    let literal = CHECKS.replace("open=\"{row.link}\"", "open=\"https://vorn.run\"");
    assert!(file::parse(FILE, &literal, Options::default()).is_ok());
}

#[test]
fn a_list_names_a_declared_collection() {
    let broken = CHECKS.replace("list \"checks\"", "list \"chekcs\"");
    assert_eq!(
        errors(&broken),
        ["views/checks.kdl:14:10: no collection 'chekcs'; did you mean 'checks'?"]
    );
}

#[test]
fn a_syntax_error_has_its_place() {
    let errs = file::parse(FILE, "column {\n  text \"x\n}\n", Options::default()).unwrap_err();
    assert_eq!(errs[0].at.map(|(l, _)| l), Some(2), "{errs:?}");
    assert_eq!(errs[0].file.as_deref(), Some(FILE));
}

#[test]
fn a_leaf_holds_no_children_and_a_list_one_item() {
    let leaf = CHECKS.replace(
        "text \"{row.name}\" grow=#true",
        "text \"{row.name}\" grow=#true { badge x }",
    );
    assert_eq!(
        errors(&leaf),
        ["views/checks.kdl:17:44: 'text' holds no children; only column, row, section, more and list do"]
    );
    let two = CHECKS.replace("        row {", "        divider\n        row {");
    assert_eq!(
        errors(&two),
        ["views/checks.kdl:16:9: a list draws one item per row; wrap these nodes in a row or a column"]
    );
}

#[test]
fn a_list_in_an_item_draws_the_rows_of_its_row() {
    let t = r#"schema "vorn:view@1"
rows "asks" { tool string }
rows "picks" { ask string; label string }
list "asks" {
    column {
        text "{row.tool}"
        list "picks" where="ask" is="{row.id}" {
            button "{row.label}" call="pick" window="small"
        }
    }
}
"#;
    let parsed = file::parse(FILE, t, Options::default()).unwrap();
    let Kind::List { item, .. } = &parsed.view.kind else {
        panic!("{:?}", parsed.view.kind);
    };
    let Kind::Column(c) = &item.kind else {
        panic!()
    };
    let Kind::List {
        only: Some((field, is)),
        item,
        ..
    } = &c[1].kind
    else {
        panic!("{:?}", c[1].kind);
    };
    assert_eq!(field, "ask");
    let outer = Row::from_value(&json!({ "id": "a1", "tool": "Bash" })).unwrap();
    assert_eq!(is.fill(Some(&outer), &BTreeMap::new()), "a1");
    let Kind::Button { action, .. } = &item.kind else {
        panic!()
    };
    assert_eq!(action.window.as_deref(), Some("small"));
    let unfiltered = t.replace(" where=\"ask\" is=\"{row.id}\"", "");
    assert_eq!(
        errors(&unfiltered),
        ["views/checks.kdl:7:9: a list inside a list's item draws the rows of its row, as where=\"parent\" is=\"{row.id}\""]
    );
    assert_eq!(
        errors(&t.replace("where=\"ask\"", "where=\"asc\"")),
        ["views/checks.kdl:7:22: 'picks' has no field 'asc'"]
    );
    assert_eq!(
        errors(&t.replace(" is=\"{row.id}\"", "")),
        ["views/checks.kdl:7:9: where and is go together: where=\"parent\" is=\"{row.id}\""]
    );
}

#[test]
fn a_template_has_one_view() {
    let two = format!("{CHECKS}divider\n");
    assert_eq!(
        errors(&two),
        ["views/checks.kdl:22:1: a template has one view; 'divider' is a second"]
    );
}

#[test]
fn checks_buttons_toggles_fields_and_conditions() {
    let t = r#"schema "vorn:view@1"
rows "items" { name string; on bool; size number }
rows "summary" { total string }
section "Items {summary.total}" {
    list "items" {
        row when="!{row.on}" {
            toggle "{row.on}" label="{row.name}" call="select"
            button Remove call="remove" confirm="Remove {row.name}?" style=danger icon="trash-2"
        }
    }
    field placeholder="Filter" call="filter"
    button Close close=#true
}
"#;
    let parsed = file::parse("views/t.kdl", t, Options::default()).unwrap();
    assert_eq!(parsed.collections.len(), 2);
    let bad = t
        .replace("toggle \"{row.on}\"", "toggle \"{row.size}\"")
        .replace("call=\"remove\"", "call=\"remove\" open=\"https://x\"")
        .replace("style=danger", "style=dangr")
        .replace("button Close close=#true", "button Close");
    let errs = errors(&bad);
    assert_eq!(
        errs,
        [
            "views/checks.kdl:7:20: a toggle shows one bool field, as \"{row.on}\"",
            "views/checks.kdl:8:13: a button does one of call, open, focus and signin",
            "views/checks.kdl:8:87: unknown style 'dangr'; did you mean 'danger'?",
            "views/checks.kdl:12:5: a button needs an action: call, open, focus, show, close, window or signin",
        ]
    );
}

#[test]
fn conditions_read_filled_bindings() {
    let t = parse_value(
        &json!({
            "schema": SCHEMA,
            "rows": { "items": { "on": "bool" } },
            "view": { "list": { "item": { "text": { "value": "{row.id}", "when": "!{row.on}" } } } }
        }),
        Options::default(),
    )
    .unwrap();
    let Kind::List { item, .. } = &t.view.kind else {
        panic!("a list")
    };
    let src: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    let on = Row::from_value(&json!({ "id": "a", "on": true })).unwrap();
    let off = Row::from_value(&json!({ "id": "b", "on": false })).unwrap();
    let when = item.when.as_ref().unwrap();
    assert!(!when.holds(Some(&on), &src));
    assert!(when.holds(Some(&off), &src));
}

#[test]
fn checks_rows_against_the_declared_fields() {
    let t = file::parse(FILE, CHECKS, Options::default()).unwrap();
    let schema = t.schema("checks").unwrap();
    assert!(schema
        .check_row(&json!({ "id": "a", "name": "build", "state": "pass" }))
        .is_ok());
    let errs: Vec<String> = [
        json!({ "id": "a", "state": "succes" }),
        json!({ "id": "b", "colour": "red" }),
        json!({ "id": "c", "link": "http://insecure" }),
        json!({ "name": "no id" }),
    ]
    .iter()
    .map(|r| schema.check_row(r).unwrap_err())
    .collect();
    assert_eq!(
        errs,
        [
            "row a: state is \"succes\", not one of pass, fail, pending, skipping, cancel",
            "row b: the template declares no field 'colour'",
            "row c: link is \"http://insecure\", not an https link",
            "every row needs a string id",
        ]
    );
}

#[test]
fn suggests_a_near_name() {
    assert_eq!(suggest("buton", NODES), Some("button"));
    assert_eq!(suggest("colum", NODES), Some("column"));
    assert_eq!(suggest("colour", &["value", "role"]), None);
}

#[test]
fn reports_braces_that_do_not_pair() {
    let broken = CHECKS.replace("text \"{row.name}\"", "text \"{row.name\"");
    assert!(errors(&broken)[0].ends_with("a '{' with no '}'"));
}

#[test]
fn checks_choices_secret_fields_and_signin() {
    let t = r#"schema "vorn:view@1"
rows "plans" { name string; selected bool }
rows "accounts" { name string }
column {
    choice "plans" label="{row.name}" selected="{row.selected}" call="plan"
    field placeholder="Key" call="key" secret=#true
    list "accounts" {
        button "Sign in" signin="{row.id}"
    }
}
"#;
    let parsed = file::parse("views/t.kdl", t, Options::default()).unwrap();
    let Kind::Column(children) = &parsed.view.kind else {
        panic!("a column");
    };
    assert!(
        matches!(&children[0].kind, Kind::Choice { rows, call, .. } if rows == "plans" && call == "plan")
    );
    assert!(matches!(
        &children[1].kind,
        Kind::Field { secret: true, .. }
    ));
    let bad = t
        .replace("choice \"plans\"", "choice \"plan\"")
        .replace(
            "label=\"{row.name}\" selected",
            "label=\"{row.nme}\" selected",
        )
        .replace("signin=\"{row.id}\"", "signin=\"{row.id}\" call=\"x\"");
    let errs = errors(&bad);
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert!(errs[0].contains("'plan'"), "{errs:?}");
    assert!(errs[1].contains("a button does one of"), "{errs:?}");
}

#[test]
fn a_more_menu_holds_buttons_only() {
    let t = r#"schema "vorn:view@1"
rows "items" { name string }
list "items" {
    row {
        text "{row.name}"
        more {
            button Rename call="rename"
            button Remove call="remove" style=danger confirm="Remove {row.name}?"
        }
    }
}
"#;
    let parsed = file::parse("views/t.kdl", t, Options::default()).unwrap();
    let Kind::List { item, .. } = &parsed.view.kind else {
        panic!("a list");
    };
    let Kind::Row(kids) = &item.kind else {
        panic!("a row");
    };
    assert!(matches!(&kids[1].kind, Kind::More(b) if b.len() == 2));
    let errs = errors(&t.replace("button Rename call=\"rename\"", "text \"Rename\""));
    assert!(
        errs[0].contains("a more menu holds one button or more"),
        "{errs:?}"
    );
    let errs = errors(&t.replace(
        "            button Rename call=\"rename\"\n            button Remove call=\"remove\" style=danger confirm=\"Remove {row.name}?\"\n",
        "",
    ));
    assert!(
        errs[0].contains("a more menu holds one button or more"),
        "{errs:?}"
    );
}
