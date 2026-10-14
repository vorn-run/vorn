//! Generates the extension API from `wit/extension.wit`: its types as serde
//! types, every function as a [`Method`] with its params, and the stdio
//! mapping as JSON. The WIT is the only definition; nothing here names a
//! method.
//!
//! The parser reads the subset of WIT the package uses (interfaces with
//! `use`, `type`, `record`, `variant`, `enum` and functions, and one world)
//! and fails the build, naming the line, on anything else.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

const WIT: &str = "wit/extension.wit";

fn main() {
    println!("cargo:rerun-if-changed={WIT}");
    let text = std::fs::read_to_string(WIT).expect("wit/extension.wit is readable");
    let package = match Parser::new(&text).package() {
        Ok(p) => p,
        Err(e) => panic!("{WIT}:{}: {}", e.0, e.1),
    };
    if let Err(e) = package.check() {
        panic!("{WIT}: {e}");
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("api.rs"), package.rust()).expect("OUT_DIR is writable");
}

// ---- the model ----

#[derive(Debug, Clone, PartialEq)]
enum Ty {
    Prim(String),
    Option(Box<Ty>),
    List(Box<Ty>),
    Named(String),
}

impl Ty {
    fn wit(&self) -> String {
        match self {
            Ty::Prim(p) | Ty::Named(p) => p.clone(),
            Ty::Option(t) => format!("option<{}>", t.wit()),
            Ty::List(t) => format!("list<{}>", t.wit()),
        }
    }
}

#[derive(Debug)]
enum Def {
    Alias(Ty),
    Record(Vec<(String, Ty)>),
    Variant(Vec<(String, Option<Ty>)>),
    Enum(Vec<String>),
}

#[derive(Debug)]
struct Func {
    name: String,
    params: Vec<(String, Ty)>,
    /// `Some(None)` for `result<_, e>`; `None` for no result at all.
    ok: Option<Option<Ty>>,
    err: Option<Ty>,
}

#[derive(Debug)]
struct Interface {
    name: String,
    funcs: Vec<Func>,
}

#[derive(Debug)]
struct Package {
    name: String,
    version: String,
    types: BTreeMap<String, Def>,
    /// In the order they are declared.
    type_order: Vec<String>,
    interfaces: Vec<Interface>,
    imports: Vec<String>,
    exports: Vec<String>,
}

// ---- the parser ----

struct Parser<'a> {
    toks: Vec<(usize, &'a str)>,
    at: usize,
}

type Fail = (usize, String);

const PUNCT: &[&str] = &[
    "->", "{", "}", "(", ")", "<", ">", ",", ";", ":", ".", "@", "=",
];

fn tokens(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or_default();
        let mut rest = line;
        while !rest.is_empty() {
            let trimmed = rest.trim_start();
            if trimmed.is_empty() {
                break;
            }
            rest = trimmed;
            if let Some(p) = PUNCT.iter().find(|p| rest.starts_with(**p)) {
                out.push((n + 1, &rest[..p.len()]));
                rest = &rest[p.len()..];
                continue;
            }
            // A kebab name holds `-`, so only `->` ends one there.
            let end = rest
                .char_indices()
                .find(|&(i, c)| {
                    c.is_whitespace() || "{}()<>,;:.@=".contains(c) || rest[i..].starts_with("->")
                })
                .map_or(rest.len(), |(i, _)| i)
                .max(1);
            out.push((n + 1, &rest[..end]));
            rest = &rest[end..];
        }
    }
    out
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Parser<'a> {
        Parser {
            toks: tokens(text),
            at: 0,
        }
    }

    fn line(&self) -> usize {
        self.toks
            .get(self.at)
            .or(self.toks.last())
            .map_or(1, |t| t.0)
    }

    fn peek(&self) -> Option<&'a str> {
        self.toks.get(self.at).map(|t| t.1)
    }

    fn next(&mut self) -> Result<&'a str, Fail> {
        let t = self
            .peek()
            .ok_or((self.line(), "the file ends early".to_owned()))?;
        self.at += 1;
        Ok(t)
    }

    fn eat(&mut self, want: &str) -> Result<(), Fail> {
        let line = self.line();
        let got = self.next()?;
        if got == want {
            Ok(())
        } else {
            Err((line, format!("expected `{want}`, found `{got}`")))
        }
    }

    fn ident(&mut self) -> Result<String, Fail> {
        let line = self.line();
        let t = self.next()?;
        let ok = t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && t.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(t.to_owned())
        } else {
            Err((line, format!("expected a name, found `{t}`")))
        }
    }

    fn package(&mut self) -> Result<Package, Fail> {
        self.eat("package")?;
        let ns = self.ident()?;
        self.eat(":")?;
        let name = self.ident()?;
        self.eat("@")?;
        let mut version = String::new();
        while self.peek() != Some(";") {
            version.push_str(self.next()?);
        }
        self.eat(";")?;
        let mut pkg = Package {
            name: format!("{ns}:{name}"),
            version,
            types: BTreeMap::new(),
            type_order: Vec::new(),
            interfaces: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
        };
        while let Some(t) = self.peek() {
            match t {
                "interface" => self.interface(&mut pkg)?,
                "world" => self.world(&mut pkg)?,
                other => return Err((self.line(), format!("unexpected `{other}`"))),
            }
        }
        Ok(pkg)
    }

    fn world(&mut self, pkg: &mut Package) -> Result<(), Fail> {
        self.eat("world")?;
        self.ident()?;
        self.eat("{")?;
        while self.peek() != Some("}") {
            let line = self.line();
            match self.next()? {
                "import" => pkg.imports.push(self.ident()?),
                "export" => pkg.exports.push(self.ident()?),
                other => {
                    return Err((
                        line,
                        format!("a world holds imports and exports, not `{other}`"),
                    ))
                }
            }
            self.eat(";")?;
        }
        self.eat("}")
    }

    fn interface(&mut self, pkg: &mut Package) -> Result<(), Fail> {
        self.eat("interface")?;
        let name = self.ident()?;
        self.eat("{")?;
        let mut funcs = Vec::new();
        while self.peek() != Some("}") {
            let line = self.line();
            match self.peek() {
                Some("use") => {
                    self.next()?;
                    self.ident()?;
                    self.eat(".")?;
                    self.eat("{")?;
                    loop {
                        self.ident()?;
                        match self.next()? {
                            "," => continue,
                            "}" => break,
                            other => return Err((line, format!("unexpected `{other}` in use"))),
                        }
                    }
                    self.eat(";")?;
                }
                Some("type") => {
                    self.next()?;
                    let n = self.ident()?;
                    self.eat("=")?;
                    let ty = self.ty()?;
                    self.eat(";")?;
                    pkg.define(n, Def::Alias(ty), line)?;
                }
                Some("record") => {
                    self.next()?;
                    let n = self.ident()?;
                    self.eat("{")?;
                    let mut fields = Vec::new();
                    while self.peek() != Some("}") {
                        let f = self.ident()?;
                        self.eat(":")?;
                        fields.push((f, self.ty()?));
                        if self.peek() == Some(",") {
                            self.next()?;
                        }
                    }
                    self.eat("}")?;
                    pkg.define(n, Def::Record(fields), line)?;
                }
                Some("variant") => {
                    self.next()?;
                    let n = self.ident()?;
                    self.eat("{")?;
                    let mut cases = Vec::new();
                    while self.peek() != Some("}") {
                        let c = self.ident()?;
                        let payload = if self.peek() == Some("(") {
                            self.next()?;
                            let t = self.ty()?;
                            self.eat(")")?;
                            Some(t)
                        } else {
                            None
                        };
                        cases.push((c, payload));
                        if self.peek() == Some(",") {
                            self.next()?;
                        }
                    }
                    self.eat("}")?;
                    pkg.define(n, Def::Variant(cases), line)?;
                }
                Some("enum") => {
                    self.next()?;
                    let n = self.ident()?;
                    self.eat("{")?;
                    let mut cases = Vec::new();
                    while self.peek() != Some("}") {
                        cases.push(self.ident()?);
                        if self.peek() == Some(",") {
                            self.next()?;
                        }
                    }
                    self.eat("}")?;
                    pkg.define(n, Def::Enum(cases), line)?;
                }
                _ => funcs.push(self.func()?),
            }
        }
        self.eat("}")?;
        pkg.interfaces.push(Interface { name, funcs });
        Ok(())
    }

    fn func(&mut self) -> Result<Func, Fail> {
        let name = self.ident()?;
        self.eat(":")?;
        self.eat("func")?;
        self.eat("(")?;
        let mut params = Vec::new();
        while self.peek() != Some(")") {
            let p = self.ident()?;
            self.eat(":")?;
            params.push((p, self.ty()?));
            if self.peek() == Some(",") {
                self.next()?;
            }
        }
        self.eat(")")?;
        let (mut ok, mut err) = (None, None);
        if self.peek() == Some("->") {
            self.next()?;
            let line = self.line();
            if self.peek() != Some("result") {
                return Err((
                    line,
                    "a function returns a result<_, ext-error> here".into(),
                ));
            }
            self.next()?;
            self.eat("<")?;
            ok = Some(if self.peek() == Some("_") {
                self.next()?;
                None
            } else {
                Some(self.ty()?)
            });
            self.eat(",")?;
            err = Some(self.ty()?);
            self.eat(">")?;
        }
        self.eat(";")?;
        Ok(Func {
            name,
            params,
            ok,
            err,
        })
    }

    fn ty(&mut self) -> Result<Ty, Fail> {
        let line = self.line();
        let t = self.next()?;
        Ok(match t {
            "string" | "bool" | "u8" | "u16" | "u32" | "u64" | "s32" | "s64" | "f64" => {
                Ty::Prim(t.to_owned())
            }
            "option" | "list" => {
                self.eat("<")?;
                let inner = Box::new(self.ty()?);
                self.eat(">")?;
                if t == "option" {
                    Ty::Option(inner)
                } else {
                    Ty::List(inner)
                }
            }
            "_" | "result" | "tuple" => return Err((line, format!("`{t}` is not used here"))),
            _ => {
                self.at -= 1;
                Ty::Named(self.ident()?)
            }
        })
    }
}

impl Package {
    fn define(&mut self, name: String, def: Def, line: usize) -> Result<(), Fail> {
        if self.types.contains_key(&name) {
            return Err((line, format!("`{name}` is defined twice")));
        }
        self.type_order.push(name.clone());
        self.types.insert(name, def);
        Ok(())
    }

    /// Every named type is defined; every interface the world names exists.
    fn check(&self) -> Result<(), String> {
        let mut used = Vec::new();
        for def in self.types.values() {
            match def {
                Def::Alias(t) => used.push(t),
                Def::Record(fields) => used.extend(fields.iter().map(|f| &f.1)),
                Def::Variant(cases) => used.extend(cases.iter().filter_map(|c| c.1.as_ref())),
                Def::Enum(_) => {}
            }
        }
        for f in self.interfaces.iter().flat_map(|i| &i.funcs) {
            used.extend(f.params.iter().map(|p| &p.1));
            used.extend(f.ok.iter().flatten());
            used.extend(f.err.iter());
        }
        for t in used {
            let mut t = t;
            loop {
                match t {
                    Ty::Option(i) | Ty::List(i) => t = i,
                    Ty::Named(n) if !self.types.contains_key(n) => {
                        return Err(format!("`{n}` is used but never defined"))
                    }
                    _ => break,
                }
            }
        }
        for i in self.imports.iter().chain(&self.exports) {
            if !self.interfaces.iter().any(|x| &x.name == i) {
                return Err(format!("the world names `{i}`, which is not an interface"));
            }
        }
        Ok(())
    }

    fn side(&self, interface: &str) -> &'static str {
        if self.exports.iter().any(|e| e == interface) {
            "Guest"
        } else {
            "Host"
        }
    }

    // ---- the Rust ----

    fn rust_ty(&self, t: &Ty) -> String {
        match t {
            Ty::Prim(p) => match p.as_str() {
                "string" => "String".into(),
                "bool" => "bool".into(),
                "s32" => "i32".into(),
                "s64" => "i64".into(),
                other => other.into(),
            },
            Ty::Option(i) => format!("Option<{}>", self.rust_ty(i)),
            Ty::List(i) => format!("Vec<{}>", self.rust_ty(i)),
            Ty::Named(n) => match self.types.get(n) {
                // `json` is any value; other aliases are what they name.
                Some(Def::Alias(_)) if n == "json" => "serde_json::Value".into(),
                Some(Def::Alias(t)) => self.rust_ty(t),
                _ => camel(n),
            },
        }
    }

    fn field(&self, out: &mut String, name: &str, t: &Ty, public: bool) {
        let rust = snake(name);
        if rust != name {
            let _ = writeln!(out, "    #[serde(rename = \"{name}\")]");
        }
        match t {
            Ty::Option(_) => {
                let _ = writeln!(out, "    #[serde(default)]");
            }
            Ty::List(_) => {
                let _ = writeln!(out, "    #[serde(default)]");
            }
            Ty::Named(n) if n == "json" => {
                let _ = writeln!(out, "    #[serde(default)]");
            }
            _ => {}
        }
        let vis = if public { "pub " } else { "" };
        let _ = writeln!(out, "    {vis}{}: {},", rust, self.rust_ty(t));
    }

    fn rust(&self) -> String {
        let mut out = String::new();
        let api = self
            .version
            .rsplit_once('.')
            .map_or(self.version.as_str(), |(head, _)| head);
        let _ = writeln!(out, "/// The WIT package this build speaks.");
        let _ = writeln!(
            out,
            "pub const PACKAGE: &str = \"{}@{}\";",
            self.name, self.version
        );
        let _ = writeln!(
            out,
            "/// What a manifest's `api` names: the package without its patch."
        );
        let _ = writeln!(out, "pub const API: &str = \"{}@{api}\";\n", self.name);
        let _ = writeln!(out, "/// The WIT's types, as they travel over stdio.");
        let _ = writeln!(out, "pub mod types {{");
        let _ = writeln!(out, "    use serde::{{Deserialize, Serialize}};");
        for name in &self.type_order {
            let def = &self.types[name];
            let rust = camel(name);
            match def {
                Def::Alias(_) => {}
                Def::Record(fields) => {
                    let _ = writeln!(
                        out,
                        "\n    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]"
                    );
                    let _ = writeln!(out, "    pub struct {rust} {{");
                    let mut body = String::new();
                    for (f, t) in fields {
                        self.field(&mut body, f, t, true);
                    }
                    for line in body.lines() {
                        let _ = writeln!(out, "    {line}");
                    }
                    let _ = writeln!(out, "    }}");
                }
                Def::Variant(cases) => {
                    let _ = writeln!(
                        out,
                        "\n    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]"
                    );
                    let _ = writeln!(out, "    #[serde(rename_all = \"kebab-case\")]");
                    let _ = writeln!(out, "    pub enum {rust} {{");
                    for (c, payload) in cases {
                        match payload {
                            Some(t) => {
                                let _ = writeln!(out, "        {}({}),", camel(c), self.rust_ty(t));
                            }
                            None => {
                                let _ = writeln!(out, "        {},", camel(c));
                            }
                        }
                    }
                    let _ = writeln!(out, "    }}");
                }
                Def::Enum(cases) => {
                    let _ = writeln!(out, "\n    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]");
                    let _ = writeln!(out, "    #[serde(rename_all = \"kebab-case\")]");
                    let _ = writeln!(out, "    pub enum {rust} {{");
                    for c in cases {
                        let _ = writeln!(out, "        {},", camel(c));
                    }
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        let _ = writeln!(out, "}}\n");

        // Params, one struct per function.
        let _ = writeln!(
            out,
            "/// Each function's params, keyed by the WIT's parameter names."
        );
        let _ = writeln!(out, "pub mod params {{");
        let _ = writeln!(out, "    #[allow(unused_imports)]");
        let _ = writeln!(out, "    use super::types::*;");
        let _ = writeln!(out, "    use serde::{{Deserialize, Serialize}};");
        for i in &self.interfaces {
            for f in &i.funcs {
                let _ = writeln!(out, "\n    /// `{}.{}`", i.name, f.name);
                let _ = writeln!(
                    out,
                    "    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]"
                );
                let _ = writeln!(
                    out,
                    "    pub struct {}{} {{",
                    camel(&i.name),
                    camel(&f.name)
                );
                let mut body = String::new();
                for (p, t) in &f.params {
                    self.field(&mut body, p, t, true);
                }
                for line in body.lines() {
                    let _ = writeln!(out, "    {line}");
                }
                let _ = writeln!(out, "    }}");
            }
        }
        let _ = writeln!(out, "}}\n");

        // The methods.
        let all: Vec<(&Interface, &Func)> = self
            .interfaces
            .iter()
            .filter(|i| self.imports.contains(&i.name) || self.exports.contains(&i.name))
            .flat_map(|i| i.funcs.iter().map(move |f| (i, f)))
            .collect();
        let _ = writeln!(
            out,
            "/// Every function of the world, as a JSON-RPC method over stdio."
        );
        let _ = writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]");
        let _ = writeln!(out, "pub enum Method {{");
        for (i, f) in &all {
            let _ = writeln!(out, "    /// `{}.{}`", i.name, f.name);
            let _ = writeln!(out, "    {}{},", camel(&i.name), camel(&f.name));
        }
        let _ = writeln!(out, "}}\n");
        let _ = writeln!(out, "impl Method {{");
        let _ = writeln!(out, "    pub const ALL: [Method; {}] = [", all.len());
        for (i, f) in &all {
            let _ = writeln!(out, "        Method::{}{},", camel(&i.name), camel(&f.name));
        }
        let _ = writeln!(out, "    ];\n");
        let _ = writeln!(
            out,
            "    /// The JSON-RPC method name: `interface.function`."
        );
        let _ = writeln!(out, "    pub fn name(self) -> &'static str {{");
        let _ = writeln!(out, "        match self {{");
        for (i, f) in &all {
            let _ = writeln!(
                out,
                "            Method::{}{} => \"{}.{}\",",
                camel(&i.name),
                camel(&f.name),
                i.name,
                f.name
            );
        }
        let _ = writeln!(out, "        }}\n    }}\n");
        let _ = writeln!(out, "    /// Which end answers it.");
        let _ = writeln!(out, "    pub fn side(self) -> Side {{");
        let _ = writeln!(out, "        match self {{");
        for (i, f) in &all {
            let _ = writeln!(
                out,
                "            Method::{}{} => Side::{},",
                camel(&i.name),
                camel(&f.name),
                self.side(&i.name)
            );
        }
        let _ = writeln!(out, "        }}\n    }}\n");
        let _ = writeln!(
            out,
            "    /// Whether it is sent without an id and never answered."
        );
        let _ = writeln!(out, "    pub fn is_notification(self) -> bool {{");
        let _ = writeln!(out, "        match self {{");
        for (i, f) in &all {
            let _ = writeln!(
                out,
                "            Method::{}{} => {},",
                camel(&i.name),
                camel(&f.name),
                f.ok.is_none()
            );
        }
        let _ = writeln!(out, "        }}\n    }}\n");
        let _ = writeln!(out, "    pub fn from_name(name: &str) -> Option<Method> {{");
        let _ = writeln!(
            out,
            "        Method::ALL.into_iter().find(|m| m.name() == name)"
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}\n");

        // The stdio mapping as JSON, for authors and the conformance suite.
        let mut stdio = String::from("{\n");
        let _ = writeln!(stdio, "  \"package\": \"{}@{}\",", self.name, self.version);
        let _ = writeln!(stdio, "  \"api\": \"{}@{api}\",", self.name);
        stdio.push_str("  \"types\": {\n");
        let types: Vec<String> = self
            .type_order
            .iter()
            .map(|n| {
                let body = match &self.types[n] {
                    Def::Alias(_) if n == "json" => "{ \"inline\": \"any JSON value\" }".to_owned(),
                    Def::Alias(t) => format!("{{ \"alias\": \"{}\" }}", t.wit()),
                    Def::Record(fields) => format!(
                        "{{ \"record\": [{}] }}",
                        fields
                            .iter()
                            .map(|(f, t)| format!("[\"{f}\", \"{}\"]", t.wit()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Def::Variant(cases) => format!(
                        "{{ \"variant\": [{}] }}",
                        cases
                            .iter()
                            .map(|(c, t)| match t {
                                Some(t) => format!("[\"{c}\", \"{}\"]", t.wit()),
                                None => format!("[\"{c}\", null]"),
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Def::Enum(cases) => format!(
                        "{{ \"enum\": [{}] }}",
                        cases
                            .iter()
                            .map(|c| format!("\"{c}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                format!("    \"{n}\": {body}")
            })
            .collect();
        stdio.push_str(&types.join(",\n"));
        stdio.push_str("\n  },\n  \"methods\": [\n");
        let methods: Vec<String> = all
            .iter()
            .map(|(i, f)| {
                let params = f
                    .params
                    .iter()
                    .map(|(p, t)| format!("[\"{p}\", \"{}\"]", t.wit()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let result = match &f.ok {
                    None => "null".to_owned(),
                    Some(None) => "\"_\"".to_owned(),
                    Some(Some(t)) => format!("\"{}\"", t.wit()),
                };
                let error = f
                    .err
                    .as_ref()
                    .map_or_else(|| "null".to_owned(), |t| format!("\"{}\"", t.wit()));
                format!(
                    "    {{ \"method\": \"{}.{}\", \"answers\": \"{}\", \"params\": [{params}], \"result\": {result}, \"error\": {error} }}",
                    i.name,
                    f.name,
                    self.side(&i.name).to_lowercase()
                )
            })
            .collect();
        stdio.push_str(&methods.join(",\n"));
        stdio.push_str("\n  ]\n}\n");
        let _ = writeln!(
            out,
            "/// The stdio mapping, as `wit/stdio.json` publishes it."
        );
        let _ = writeln!(out, "pub const STDIO: &str = r#\"{stdio}\"#;");
        out
    }
}

fn camel(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn snake(kebab: &str) -> String {
    let s = kebab.replace('-', "_");
    match s.as_str() {
        "type" | "use" | "enum" | "struct" | "move" | "ref" | "box" | "match" => format!("r#{s}"),
        _ => s,
    }
}
