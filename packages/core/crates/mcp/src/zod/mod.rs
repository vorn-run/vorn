//! The tools' argument schemas, checked as zod checks them.
//!
//! The SDK validates a call's arguments with zod before a tool runs, and a
//! refusal reaches the agent as the zod issues themselves, as JSON. So the
//! schemas here are zod's, walked out of the TypeScript into
//! `generated/schemas.json` by `scripts/gen-mcp-tools.mjs`, and [`parse`]
//! follows zod 4's rules: which issue stops which check, the shape and key
//! order of each issue, and what the parsed arguments look like (unknown keys
//! dropped, known ones in the schema's order), since the tools work on the
//! parsed arguments and some store them as they are.
//!
//! The rules zod cannot write down, its refinements, are named in the
//! generated file and implemented in [`refine`].

mod refine;

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::json;

pub use refine::{node_config_issues, Refinement};

/// A position inside the value an issue is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

impl Seg {
    fn to_json(&self) -> Value {
        match self {
            Seg::Key(k) => Value::String(k.clone()),
            Seg::Index(i) => json!(i),
        }
    }

    /// As `String(segment)` prints it.
    pub fn display(&self) -> String {
        match self {
            Seg::Key(k) => k.clone(),
            Seg::Index(i) => i.to_string(),
        }
    }
}

/// One schema node, as zod builds it.
#[derive(Debug)]
pub enum Schema {
    String(Vec<Check>),
    Number(Vec<Check>),
    Boolean,
    /// `z.unknown()` and `z.any()`.
    Unknown,
    Literal(Vec<Value>),
    Enum(Vec<Value>),
    Optional(Box<Schema>),
    Array {
        item: Box<Schema>,
        checks: Vec<Check>,
    },
    /// String keys, values of one schema.
    Record {
        value: Box<Schema>,
        checks: Vec<Check>,
    },
    Object {
        shape: Vec<(String, Schema)>,
        loose: bool,
        checks: Vec<Check>,
    },
    Union {
        options: Vec<Schema>,
        checks: Vec<Check>,
    },
    /// `z.discriminatedUnion`: the option is picked by one key's literal.
    Discriminated {
        key: String,
        options: Vec<(Vec<Value>, Schema)>,
        checks: Vec<Check>,
    },
    /// A schema written once under `defs`.
    Ref(String),
}

/// A check on a value that already has the right type.
#[derive(Debug)]
pub enum Check {
    /// `.min(n)` on a string or an array, in UTF-16 units or items.
    Min(f64, Option<String>),
    Max(f64, Option<String>),
    /// Number bounds; `Gte(0)` is `.nonnegative()`, `Gt(0)` `.positive()`.
    Gte(f64, Option<String>),
    Gt(f64, Option<String>),
    Lte(f64, Option<String>),
    Lt(f64, Option<String>),
    /// `.int()`: a whole number in the safe integer range.
    Int(Option<String>),
    Regex {
        regex: Regex,
        /// The pattern as `String(regexp)` prints it, for the issue.
        shown: String,
        message: Option<String>,
    },
    /// A refinement, which reports its own issues.
    Custom {
        refinement: Refinement,
        message: Option<String>,
        path: Vec<Seg>,
    },
}

/// What zod found wrong, with zod's fields.
#[derive(Clone, Debug)]
pub enum IssueKind {
    InvalidType {
        expected: String,
        received: String,
        /// `.int()` reports its format.
        format: Option<&'static str>,
        /// A few of zod's issues put `code` before `expected`.
        code_first: bool,
    },
    TooSmall {
        origin: &'static str,
        minimum: f64,
        inclusive: bool,
        /// The safe integer range, which zod words with a note.
        safe_int: bool,
    },
    TooBig {
        origin: &'static str,
        maximum: f64,
        inclusive: bool,
        safe_int: bool,
    },
    InvalidValue {
        values: Vec<Value>,
    },
    InvalidFormat {
        pattern: String,
    },
    Custom,
    InvalidUnion {
        errors: Vec<Vec<Issue>>,
    },
    NoDiscriminator {
        discriminator: String,
        options: Vec<Value>,
    },
}

/// One zod issue.
#[derive(Clone, Debug)]
pub struct Issue {
    pub kind: IssueKind,
    pub path: Vec<Seg>,
    pub message: String,
    /// Whether checks after it still run: zod's `continue`.
    flow: Flow,
}

/// zod's `continue` on an issue, which decides the checks that still run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// `continue: true`: later checks run.
    Continue,
    /// `continue` unset: only the length checks still run. A type mismatch
    /// is of this kind.
    Stop,
    /// `continue: false`: nothing more runs, length checks included. Only
    /// `.int()` sets it.
    Halt,
}

const SAFE_INTEGER_NOTE: &str = "Integers must be within the safe integer range.";
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

impl Issue {
    fn new(kind: IssueKind, message: String, flow: Flow) -> Issue {
        Issue {
            kind,
            path: Vec::new(),
            message,
            flow,
        }
    }

    /// A refinement's issue at `path`. zod gives `.refine` and `superRefine`
    /// issues alike `continue: true`, so neither stops the checks after it.
    pub fn custom(path: Vec<Seg>, message: String) -> Issue {
        Issue {
            kind: IssueKind::Custom,
            path,
            message,
            flow: Flow::Continue,
        }
    }

    fn invalid_type(expected: &str, input: Option<&Value>) -> Issue {
        let received = received(input);
        Issue::new(
            IssueKind::InvalidType {
                expected: expected.to_owned(),
                received: received.to_owned(),
                format: None,
                code_first: false,
            },
            format!("Invalid input: expected {expected}, received {received}"),
            Flow::Stop,
        )
    }

    fn prefixed(mut self, seg: Seg) -> Issue {
        self.path.insert(0, seg);
        self
    }

    /// The issue as zod serializes it, its keys in zod's order.
    pub fn to_json(&self) -> Value {
        let path = Value::Array(self.path.iter().map(Seg::to_json).collect());
        let mut out = Map::new();
        match &self.kind {
            IssueKind::InvalidType {
                expected,
                format,
                code_first,
                ..
            } => {
                if *code_first {
                    out.insert("code".into(), "invalid_type".into());
                    out.insert("expected".into(), expected.as_str().into());
                } else {
                    out.insert("expected".into(), expected.as_str().into());
                    if let Some(format) = format {
                        out.insert("format".into(), (*format).into());
                    }
                    out.insert("code".into(), "invalid_type".into());
                }
            }
            IssueKind::TooSmall {
                origin,
                minimum,
                inclusive,
                safe_int,
            } => bound(
                &mut out,
                "too_small",
                "minimum",
                origin,
                *minimum,
                *inclusive,
                *safe_int,
            ),
            IssueKind::TooBig {
                origin,
                maximum,
                inclusive,
                safe_int,
            } => bound(
                &mut out, "too_big", "maximum", origin, *maximum, *inclusive, *safe_int,
            ),
            IssueKind::InvalidValue { values } => {
                out.insert("code".into(), "invalid_value".into());
                out.insert("values".into(), Value::Array(values.clone()));
            }
            IssueKind::InvalidFormat { pattern } => {
                out.insert("origin".into(), "string".into());
                out.insert("code".into(), "invalid_format".into());
                out.insert("format".into(), "regex".into());
                out.insert("pattern".into(), pattern.as_str().into());
            }
            IssueKind::Custom => {
                out.insert("code".into(), "custom".into());
            }
            IssueKind::InvalidUnion { errors } => {
                out.insert("code".into(), "invalid_union".into());
                out.insert(
                    "errors".into(),
                    Value::Array(
                        errors
                            .iter()
                            .map(|e| Value::Array(e.iter().map(Issue::to_json).collect()))
                            .collect(),
                    ),
                );
            }
            IssueKind::NoDiscriminator {
                discriminator,
                options,
            } => {
                out.insert("code".into(), "invalid_union".into());
                out.insert("errors".into(), Value::Array(Vec::new()));
                out.insert("note".into(), "No matching discriminator".into());
                out.insert("discriminator".into(), discriminator.as_str().into());
                out.insert("options".into(), Value::Array(options.clone()));
            }
        }
        out.insert("path".into(), path);
        out.insert("message".into(), self.message.as_str().into());
        Value::Object(out)
    }
}

fn bound(
    out: &mut Map<String, Value>,
    code: &str,
    key: &str,
    origin: &str,
    limit: f64,
    inclusive: bool,
    safe_int: bool,
) {
    if safe_int {
        out.insert("code".into(), code.into());
        out.insert(key.into(), json::num(limit));
        out.insert("note".into(), SAFE_INTEGER_NOTE.into());
        out.insert("origin".into(), origin.into());
    } else {
        out.insert("origin".into(), origin.into());
        out.insert("code".into(), code.into());
        out.insert(key.into(), json::num(limit));
    }
    out.insert("inclusive".into(), inclusive.into());
}

/// `ZodError.message`: the issues as indented JSON.
pub fn error_message(issues: &[Issue]) -> String {
    json::pretty(&Value::Array(issues.iter().map(Issue::to_json).collect()))
}

/// How zod names the type of what it was given.
fn received(input: Option<&Value>) -> &'static str {
    match input {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// A primitive as zod quotes it in a message.
fn primitive(value: &Value) -> String {
    match value {
        Value::String(_) => json::stringify(value),
        other => json::display(Some(other)),
    }
}

fn invalid_value(values: &[Value]) -> Issue {
    let message = match values {
        [one] => format!("Invalid input: expected {}", primitive(one)),
        many => format!(
            "Invalid option: expected one of {}",
            many.iter().map(primitive).collect::<Vec<_>>().join("|")
        ),
    };
    Issue::new(
        IssueKind::InvalidValue {
            values: values.to_vec(),
        },
        message,
        Flow::Stop,
    )
}

/// What a schema made of a value.
struct Parsed {
    /// The parsed value; `None` is `undefined`.
    value: Option<Value>,
    issues: Vec<Issue>,
}

impl Parsed {
    fn ok(value: Option<Value>) -> Parsed {
        Parsed {
            value,
            issues: Vec::new(),
        }
    }

    fn failed(issue: Issue) -> Parsed {
        Parsed {
            value: None,
            issues: vec![issue],
        }
    }

    fn aborted(&self) -> bool {
        self.issues.iter().any(|i| i.flow != Flow::Continue)
    }
}

/// Every schema the tools are checked with.
#[derive(Debug)]
pub struct Registry {
    defs: HashMap<String, Schema>,
    tools: BTreeMap<String, Option<Schema>>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    let text = include_str!("../../generated/schemas.json");
    let value: Value = serde_json::from_str(text).expect("generated/schemas.json is JSON");
    Registry::from_json(&value).expect("generated/schemas.json is a schema file this build reads")
});

/// The schemas generated from the TypeScript.
pub fn registry() -> &'static Registry {
    &REGISTRY
}

impl Registry {
    /// Reads the generator's output, refusing anything it does not know.
    pub fn from_json(value: &Value) -> Result<Registry, String> {
        let mut defs = HashMap::new();
        for (name, def) in value
            .get("defs")
            .and_then(Value::as_object)
            .ok_or("no defs")?
        {
            defs.insert(name.clone(), read(def)?);
        }
        let mut tools = BTreeMap::new();
        for (name, schema) in value
            .get("tools")
            .and_then(Value::as_object)
            .ok_or("no tools")?
        {
            let schema = match schema {
                Value::Null => None,
                other => Some(read(other)?),
            };
            tools.insert(name.clone(), schema);
        }
        let registry = Registry { defs, tools };
        registry.check_refs()?;
        Ok(registry)
    }

    fn check_refs(&self) -> Result<(), String> {
        fn walk(r: &Registry, s: &Schema) -> Result<(), String> {
            match s {
                Schema::Ref(name) if !r.defs.contains_key(name) => {
                    Err(format!("no definition for {name}"))
                }
                Schema::Optional(inner) => walk(r, inner),
                Schema::Array { item, .. } => walk(r, item),
                Schema::Record { value, .. } => walk(r, value),
                Schema::Object { shape, .. } => shape.iter().try_for_each(|(_, s)| walk(r, s)),
                Schema::Union { options, .. } => options.iter().try_for_each(|s| walk(r, s)),
                Schema::Discriminated { options, .. } => {
                    options.iter().try_for_each(|(_, s)| walk(r, s))
                }
                _ => Ok(()),
            }
        }
        for s in self.defs.values().chain(self.tools.values().flatten()) {
            walk(self, s)?;
        }
        Ok(())
    }

    /// A tool's argument schema: `None` for a tool nothing knows,
    /// `Some(None)` for one the SDK calls without looking at its arguments.
    pub fn tool(&self, name: &str) -> Option<Option<&Schema>> {
        self.tools.get(name).map(Option::as_ref)
    }

    /// A schema written once under `defs`, such as `nodeConfig.loop`.
    pub fn def(&self, name: &str) -> Option<&Schema> {
        self.defs.get(name)
    }

    /// `schema.safeParse(value)`: the parsed value, or every issue.
    pub fn parse(
        &self,
        schema: &Schema,
        value: Option<&Value>,
    ) -> Result<Option<Value>, Vec<Issue>> {
        let parsed = self.run(schema, value);
        if parsed.issues.is_empty() {
            Ok(parsed.value)
        } else {
            Err(parsed.issues)
        }
    }

    fn resolve<'a>(&'a self, mut schema: &'a Schema) -> &'a Schema {
        while let Schema::Ref(name) = schema {
            match self.defs.get(name) {
                Some(def) => schema = def,
                // `from_json` refuses a dangling ref.
                None => break,
            }
        }
        schema
    }

    fn run(&self, schema: &Schema, input: Option<&Value>) -> Parsed {
        match schema {
            Schema::Ref(_) => match self.resolve(schema) {
                Schema::Ref(_) => Parsed::ok(input.cloned()),
                def => self.run(def, input),
            },
            Schema::Optional(inner) => match input {
                None => Parsed::ok(None),
                Some(_) => self.run(inner, input),
            },
            Schema::Unknown => Parsed::ok(input.cloned()),
            Schema::Boolean => match input {
                Some(Value::Bool(_)) => Parsed::ok(input.cloned()),
                other => Parsed::failed(Issue::invalid_type("boolean", other)),
            },
            Schema::String(checks) => match input {
                Some(Value::String(_)) => self.checked(Parsed::ok(input.cloned()), checks),
                other => self.refused(Issue::invalid_type("string", other), other, checks),
            },
            Schema::Number(checks) => match input {
                Some(Value::Number(_)) => self.checked(Parsed::ok(input.cloned()), checks),
                other => Parsed::failed(Issue::invalid_type("number", other)),
            },
            Schema::Literal(values) | Schema::Enum(values) => {
                if values.iter().any(|v| json::strict_equals(Some(v), input)) {
                    Parsed::ok(input.cloned())
                } else {
                    Parsed::failed(invalid_value(values))
                }
            }
            Schema::Array { item, checks } => {
                let Some(Value::Array(items)) = input else {
                    return self.refused(Issue::invalid_type("array", input), input, checks);
                };
                let mut out = Vec::with_capacity(items.len());
                let mut issues = Vec::new();
                for (i, element) in items.iter().enumerate() {
                    let r = self.run(item, Some(element));
                    issues.extend(r.issues.into_iter().map(|x| x.prefixed(Seg::Index(i))));
                    out.push(r.value.unwrap_or(Value::Null));
                }
                self.checked(
                    Parsed {
                        value: Some(Value::Array(out)),
                        issues,
                    },
                    checks,
                )
            }
            Schema::Record { value, checks } => {
                let Some(Value::Object(map)) = input else {
                    return self.refused(Issue::invalid_type("record", input), input, checks);
                };
                let mut out = Map::new();
                let mut issues = Vec::new();
                for (key, element) in map {
                    let r = self.run(value, Some(element));
                    issues.extend(
                        r.issues
                            .into_iter()
                            .map(|x| x.prefixed(Seg::Key(key.clone()))),
                    );
                    if let Some(v) = r.value {
                        out.insert(key.clone(), v);
                    }
                }
                self.checked(
                    Parsed {
                        value: Some(Value::Object(out)),
                        issues,
                    },
                    checks,
                )
            }
            Schema::Object {
                shape,
                loose,
                checks,
            } => {
                let Some(Value::Object(map)) = input else {
                    return self.refused(Issue::invalid_type("object", input), input, checks);
                };
                let mut out = Map::new();
                let mut issues = Vec::new();
                for (key, field) in shape {
                    let given = map.get(key);
                    let optional = matches!(self.resolve(field), Schema::Optional(_));
                    let r = self.run(field, given);
                    if given.is_none() && optional {
                        continue;
                    }
                    if given.is_none() && r.issues.is_empty() && r.value.is_none() {
                        // Nothing complained, but a required key is missing.
                        issues.push(
                            Issue::new(
                                IssueKind::InvalidType {
                                    expected: "nonoptional".into(),
                                    received: "undefined".into(),
                                    format: None,
                                    code_first: true,
                                },
                                "Invalid input: expected nonoptional, received undefined".into(),
                                Flow::Stop,
                            )
                            .prefixed(Seg::Key(key.clone())),
                        );
                        continue;
                    }
                    issues.extend(
                        r.issues
                            .into_iter()
                            .map(|x| x.prefixed(Seg::Key(key.clone()))),
                    );
                    if let Some(v) = r.value {
                        out.insert(key.clone(), v);
                    }
                }
                if *loose {
                    for (key, value) in map {
                        if !shape.iter().any(|(k, _)| k == key) {
                            out.insert(key.clone(), value.clone());
                        }
                    }
                }
                self.checked(
                    Parsed {
                        value: Some(Value::Object(out)),
                        issues,
                    },
                    checks,
                )
            }
            Schema::Union { options, checks } => {
                let mut results = Vec::with_capacity(options.len());
                for option in options {
                    let r = self.run(option, input);
                    if r.issues.is_empty() {
                        return self.checked(r, checks);
                    }
                    results.push(r);
                }
                let mut standing = results.iter().filter(|r| !r.aborted());
                if let (Some(_), None) = (standing.next(), standing.next()) {
                    let only = results
                        .into_iter()
                        .find(|r| !r.aborted())
                        .expect("one result did not abort");
                    return only;
                }
                Parsed::failed(Issue::new(
                    IssueKind::InvalidUnion {
                        errors: results.into_iter().map(|r| r.issues).collect(),
                    },
                    "Invalid input".into(),
                    Flow::Stop,
                ))
            }
            Schema::Discriminated {
                key,
                options,
                checks,
            } => {
                let Some(Value::Object(map)) = input else {
                    return Parsed::failed(Issue::new(
                        IssueKind::InvalidType {
                            expected: "object".into(),
                            received: received(input).into(),
                            format: None,
                            code_first: true,
                        },
                        format!(
                            "Invalid input: expected object, received {}",
                            received(input)
                        ),
                        Flow::Stop,
                    ));
                };
                let tag = map.get(key);
                match options
                    .iter()
                    .find(|(values, _)| values.iter().any(|v| json::strict_equals(Some(v), tag)))
                {
                    Some((_, option)) => {
                        let r = self.run(option, input);
                        self.checked(r, checks)
                    }
                    None => {
                        let all: Vec<Value> = options
                            .iter()
                            .flat_map(|(v, _)| v.iter().cloned())
                            .collect();
                        let expected = all
                            .iter()
                            .map(|v| format!("'{}'", json::display(Some(v))))
                            .collect::<Vec<_>>()
                            .join(" | ");
                        let mut issue = Issue::new(
                            IssueKind::NoDiscriminator {
                                discriminator: key.clone(),
                                options: all,
                            },
                            format!("Invalid discriminator value. Expected {expected}"),
                            Flow::Stop,
                        );
                        issue.path = vec![Seg::Key(key.clone())];
                        Parsed::failed(issue)
                    }
                }
            }
        }
    }

    /// Runs `checks` on a value of the right type. A check that only makes
    /// sense on a sound value is skipped once an issue has stopped it; the
    /// length checks run unless an issue halted everything, as zod's do.
    fn checked(&self, mut parsed: Parsed, checks: &[Check]) -> Parsed {
        let mut aborted = parsed.aborted();
        for check in checks {
            if matches!(check, Check::Min(..) | Check::Max(..)) {
                if parsed.issues.iter().any(|i| i.flow == Flow::Halt) {
                    continue;
                }
            } else if aborted {
                continue;
            }
            let before = parsed.issues.len();
            self.check(check, &mut parsed);
            if !aborted {
                aborted = parsed.issues[before..]
                    .iter()
                    .any(|i| i.flow != Flow::Continue);
            }
        }
        parsed
    }

    /// A value of the wrong type. zod still runs the checks on it that guard
    /// themselves (the length checks, on anything with a `length`), so an
    /// empty array where a non-empty string belongs is reported twice.
    fn refused(&self, issue: Issue, input: Option<&Value>, checks: &[Check]) -> Parsed {
        self.checked(
            Parsed {
                value: input.cloned(),
                issues: vec![issue],
            },
            checks,
        )
    }

    fn check(&self, check: &Check, parsed: &mut Parsed) {
        let Some(value) = parsed.value.as_ref() else {
            return;
        };
        let issue = match check {
            Check::Min(min, message) => sized(value).and_then(|(origin, len)| {
                (len < *min).then(|| {
                    let text = message.clone().unwrap_or_else(|| {
                        format!(
                            "Too small: expected {origin} {}>={}{}",
                            verb(origin),
                            json::number_to_string(*min),
                            unit(origin)
                        )
                    });
                    Issue::new(
                        IssueKind::TooSmall {
                            origin,
                            minimum: *min,
                            inclusive: true,
                            safe_int: false,
                        },
                        text,
                        Flow::Continue,
                    )
                })
            }),
            Check::Max(max, message) => sized(value).and_then(|(origin, len)| {
                (len > *max).then(|| {
                    let text = message.clone().unwrap_or_else(|| {
                        format!(
                            "Too big: expected {origin} {}<={}{}",
                            verb(origin),
                            json::number_to_string(*max),
                            unit(origin)
                        )
                    });
                    Issue::new(
                        IssueKind::TooBig {
                            origin,
                            maximum: *max,
                            inclusive: true,
                            safe_int: false,
                        },
                        text,
                        Flow::Continue,
                    )
                })
            }),
            Check::Gte(limit, message) => small(value, *limit, true, message),
            Check::Gt(limit, message) => small(value, *limit, false, message),
            Check::Lte(limit, message) => big(value, *limit, true, message),
            Check::Lt(limit, message) => big(value, *limit, false, message),
            Check::Int(message) => {
                let n = value.as_f64().unwrap_or(f64::NAN);
                if n.fract() != 0.0 || !n.is_finite() {
                    Some(Issue::new(
                        IssueKind::InvalidType {
                            expected: "int".into(),
                            received: "number".into(),
                            format: Some("safeint"),
                            code_first: false,
                        },
                        message.clone().unwrap_or_else(|| {
                            "Invalid input: expected int, received number".into()
                        }),
                        Flow::Halt,
                    ))
                } else if n > MAX_SAFE_INTEGER {
                    Some(Issue::new(
                        IssueKind::TooBig {
                            origin: "int",
                            maximum: MAX_SAFE_INTEGER,
                            inclusive: true,
                            safe_int: true,
                        },
                        message.clone().unwrap_or_else(|| {
                            "Too big: expected int to be <=9007199254740991".into()
                        }),
                        Flow::Continue,
                    ))
                } else if n < -MAX_SAFE_INTEGER {
                    Some(Issue::new(
                        IssueKind::TooSmall {
                            origin: "int",
                            minimum: -MAX_SAFE_INTEGER,
                            inclusive: true,
                            safe_int: true,
                        },
                        message.clone().unwrap_or_else(|| {
                            "Too small: expected int to be >=-9007199254740991".into()
                        }),
                        Flow::Continue,
                    ))
                } else {
                    None
                }
            }
            Check::Regex {
                regex,
                shown,
                message,
            } => {
                let text = value.as_str().unwrap_or_default();
                (!regex.is_match(text)).then(|| {
                    Issue::new(
                        IssueKind::InvalidFormat {
                            pattern: shown.clone(),
                        },
                        message.clone().unwrap_or_else(|| {
                            format!("Invalid string: must match pattern {shown}")
                        }),
                        Flow::Continue,
                    )
                })
            }
            Check::Custom {
                refinement,
                message,
                path,
            } => {
                let found = refine::apply(self, *refinement, message.as_deref(), path, value);
                parsed.issues.extend(found);
                None
            }
        };
        parsed.issues.extend(issue);
    }
}

/// What a length check measures, and how much of it there is, or `None`
/// for a value without a `length`, which zod's length checks pass over. zod
/// counts a string in code points, so an emoji is one character, not two;
/// an object's own `length` key is a length too.
fn sized(value: &Value) -> Option<(&'static str, f64)> {
    match value {
        Value::String(s) => Some(("string", s.chars().count() as f64)),
        Value::Array(items) => Some(("array", items.len() as f64)),
        Value::Object(map) => map
            .get("length")
            .and_then(Value::as_f64)
            .map(|n| ("unknown", n)),
        _ => None,
    }
}

/// How zod's English messages size an origin: "to have" a count of units
/// for a string or an array, "to be" a number for anything else.
fn verb(origin: &str) -> &'static str {
    match origin {
        "string" | "array" => "to have ",
        _ => "to be ",
    }
}

fn unit(origin: &str) -> &'static str {
    match origin {
        "array" => " items",
        "string" => " characters",
        _ => "",
    }
}

fn small(value: &Value, limit: f64, inclusive: bool, message: &Option<String>) -> Option<Issue> {
    let n = value.as_f64()?;
    let fails = if inclusive { n < limit } else { n <= limit };
    fails.then(|| {
        let adj = if inclusive { ">=" } else { ">" };
        Issue::new(
            IssueKind::TooSmall {
                origin: "number",
                minimum: limit,
                inclusive,
                safe_int: false,
            },
            message.clone().unwrap_or_else(|| {
                format!(
                    "Too small: expected number to be {adj}{}",
                    json::number_to_string(limit)
                )
            }),
            Flow::Continue,
        )
    })
}

fn big(value: &Value, limit: f64, inclusive: bool, message: &Option<String>) -> Option<Issue> {
    let n = value.as_f64()?;
    let fails = if inclusive { n > limit } else { n >= limit };
    fails.then(|| {
        let adj = if inclusive { "<=" } else { "<" };
        Issue::new(
            IssueKind::TooBig {
                origin: "number",
                maximum: limit,
                inclusive,
                safe_int: false,
            },
            message.clone().unwrap_or_else(|| {
                format!(
                    "Too big: expected number to be {adj}{}",
                    json::number_to_string(limit)
                )
            }),
            Flow::Continue,
        )
    })
}

// ── Reading the generated file ──────────────────────────────────

fn read(node: &Value) -> Result<Schema, String> {
    let kind = node
        .get("t")
        .and_then(Value::as_str)
        .ok_or("schema without t")?;
    let checks = || -> Result<Vec<Check>, String> {
        node.get("checks")
            .and_then(Value::as_array)
            .map_or(Ok(Vec::new()), |c| c.iter().map(read_check).collect())
    };
    let child = |key: &str| -> Result<Box<Schema>, String> {
        node.get(key)
            .ok_or_else(|| format!("{kind} without {key}"))
            .and_then(read)
            .map(Box::new)
    };
    let values = || -> Result<Vec<Value>, String> {
        node.get("values")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| format!("{kind} without values"))
    };
    Ok(match kind {
        "string" => Schema::String(checks()?),
        "number" => Schema::Number(checks()?),
        "boolean" => Schema::Boolean,
        "unknown" => Schema::Unknown,
        "literal" => Schema::Literal(values()?),
        "enum" => Schema::Enum(values()?),
        "optional" => Schema::Optional(child("inner")?),
        "array" => Schema::Array {
            item: child("item")?,
            checks: checks()?,
        },
        "record" => Schema::Record {
            value: child("value")?,
            checks: checks()?,
        },
        "object" => {
            let mode = node.get("mode").and_then(Value::as_str).unwrap_or("strip");
            if mode == "strict" {
                return Err("strict objects are not used by the tools".into());
            }
            let shape = node
                .get("shape")
                .and_then(Value::as_array)
                .ok_or("object without shape")?
                .iter()
                .map(|entry| {
                    let key = entry.get(0).and_then(Value::as_str).ok_or("shape key")?;
                    Ok((key.to_owned(), read(entry.get(1).ok_or("shape value")?)?))
                })
                .collect::<Result<_, String>>()?;
            Schema::Object {
                shape,
                loose: mode == "loose",
                checks: checks()?,
            }
        }
        "union" => Schema::Union {
            options: node
                .get("options")
                .and_then(Value::as_array)
                .ok_or("union without options")?
                .iter()
                .map(read)
                .collect::<Result<_, _>>()?,
            checks: checks()?,
        },
        "discriminated" => Schema::Discriminated {
            key: node
                .get("key")
                .and_then(Value::as_str)
                .ok_or("discriminated union without key")?
                .to_owned(),
            options: node
                .get("options")
                .and_then(Value::as_array)
                .ok_or("discriminated union without options")?
                .iter()
                .map(|o| {
                    let values = o
                        .get("values")
                        .and_then(Value::as_array)
                        .cloned()
                        .ok_or("option without values")?;
                    Ok((
                        values,
                        read(o.get("schema").ok_or("option without schema")?)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
            checks: checks()?,
        },
        "ref" => Schema::Ref(
            node.get("name")
                .and_then(Value::as_str)
                .ok_or("ref without name")?
                .to_owned(),
        ),
        other => return Err(format!("unknown schema type {other}")),
    })
}

fn read_check(node: &Value) -> Result<Check, String> {
    let kind = node
        .get("k")
        .and_then(Value::as_str)
        .ok_or("check without k")?;
    let message = node
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let value = || {
        node.get("value")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("{kind} without value"))
    };
    Ok(match kind {
        "min" => Check::Min(value()?, message),
        "max" => Check::Max(value()?, message),
        "gte" => Check::Gte(value()?, message),
        "gt" => Check::Gt(value()?, message),
        "lte" => Check::Lte(value()?, message),
        "lt" => Check::Lt(value()?, message),
        "int" => Check::Int(message),
        "regex" => {
            let source = node
                .get("source")
                .and_then(Value::as_str)
                .ok_or("regex source")?;
            Check::Regex {
                regex: Regex::new(source).map_err(|e| format!("pattern {source}: {e}"))?,
                shown: node
                    .get("shown")
                    .and_then(Value::as_str)
                    .ok_or("regex shown")?
                    .to_owned(),
                message,
            }
        }
        "custom" => {
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .ok_or("custom name")?;
            Check::Custom {
                refinement: Refinement::named(name)
                    .ok_or_else(|| format!("a refinement this build does not know: {name}"))?,
                message,
                path: node
                    .get("path")
                    .and_then(Value::as_array)
                    .map(|p| {
                        p.iter()
                            .map(|s| match s {
                                Value::Number(n) => Seg::Index(n.as_u64().unwrap_or(0) as usize),
                                other => Seg::Key(json::display(Some(other))),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        }
        other => return Err(format!("unknown check {other}")),
    })
}

#[cfg(test)]
mod tests;
