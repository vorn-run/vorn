//! The add box (`AddConnectionForm`, `SdkConnectorForm`, `HttpProfileForm`,
//! `DynamicField`): the questions a connector asks before it is connected.

use serde_json::Value;

use super::{act, glyph, sv, text_button, R};
use crate::plugins::{
    field_id, manifest_fields, pick_id, Adding, BuiltinForm, Field, Form, Pick, PlugAct,
    ProfileForm, Rung, SdkForm,
};
use crate::screen::{ico, white, Screen};
use crate::ui::theme::color as k;
use crate::ui::widgets::{picker, Choice, PickerLook};
use crate::ui::{div, field, text, textarea, Action, El, Role, TextStyle};

pub(super) fn view(s: &Screen<'_>, a: &Adding) -> El {
    match &a.form {
        Form::NoMcp => div().p(16.0).rounded(R).border(1.0, white(0.08)).child(
            text(
                format!(
                    "The MCP connector is not available in this build, so {} cannot be added here.",
                    a.name
                ),
                12.0,
                k::DANGER,
            )
            .line_height(18.0)
            .wrapping(),
        ),
        Form::Profile(f) => profile(s, a, f),
        // A catalog entry or a pack names its package: no lookup, a plainer box.
        Form::Sdk(f) if f.listing.is_some() => div()
            .col()
            .p(16.0)
            .rounded(R)
            .bg(white(0.03))
            .border(1.0, white(0.08))
            .child(
                text(format!("Add {} connection", a.name), 14.0, k::GRAY_200)
                    .weight(500)
                    .line_height(20.0)
                    .mb(12.0)
                    .role(Role::Heading, "Add connection"),
            )
            .child(sdk(s, a, f)),
        _ => connect_box(s, a),
    }
}

/// `AddConnectionForm`'s box: the heading, the MCP tabs, then the form.
fn connect_box(s: &Screen<'_>, a: &Adding) -> El {
    let connector = match &a.form {
        Form::Builtin(f) => f.connector.as_str(),
        _ => "mcp",
    };
    let listing = s.st.plugins.listing_by_id(connector);
    let head = div()
        .row()
        .items_center()
        .gap(8.0)
        .mb(12.0)
        .child(glyph(listing, connector, 14.0, k::GRAY_400))
        .child(
            text(format!("Connect {}", a.name), 14.0, k::GRAY_200)
                .weight(500)
                .line_height(20.0)
                .role(Role::Heading, "Connect"),
        );
    let mut col = div()
        .col()
        .mb(24.0)
        .px(16.0)
        .py(12.0)
        .rounded(R)
        .bg(white(0.03))
        .border(1.0, white(0.08))
        .child(head);
    if a.mcp_tabs {
        let package = matches!(a.form, Form::Sdk(_));
        let mut tabs = div().row().gap(4.0).mb(12.0);
        for (manual, label) in [(false, "From a package"), (true, "Manual setup")] {
            let id = s.acts.on("plug-mcp-tab", act(PlugAct::McpManual(manual)));
            let on = package != manual;
            let fg = if on {
                k::GRAY_200
            } else {
                k::GRAY_500.mix(k::GRAY_300, s.ui.hover(id))
            };
            tabs = tabs.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .ring(R)
                    .px(12.0)
                    .py(4.0)
                    .rounded(R)
                    .bg(if on { white(0.1) } else { white(0.0) })
                    .role(Role::Button, label)
                    .toggled(on)
                    .child(text(label, 12.0, fg).line_height(16.0).silent()),
            );
        }
        col = col.child(tabs);
    }
    match &a.form {
        Form::Builtin(f) => col.child(builtin(s, a, f)),
        Form::Sdk(f) => col.child(sdk(s, a, f)),
        _ => col,
    }
}

/// The form's input: `px-3 py-1.5 text-sm` on `white/[0.05]`.
fn input(s: &Screen<'_>, f: Field, value: &str, name: &str, placeholder: &str, mono: bool) -> El {
    let id = field_id(&f);
    s.acts.text(id, act(PlugAct::Text(f)), value);
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    let style = TextStyle {
        line_height: 20.0,
        mono,
        ..TextStyle::ui(14.0)
    };
    field(id, name, placeholder, style, k::GRAY_200)
        .placeholder_color(k::GRAY_600)
        .w_full()
        .px(12.0)
        .py(6.0)
        .rounded(R)
        .bg(white(0.05))
        .border(1.0, white(0.1).mix(white(0.2), focus))
}

/// A `<select>` of `labels`, `chosen` among them.
fn select(s: &Screen<'_>, pick: Pick, name: &str, labels: &[String], chosen: usize) -> El {
    let choices: Vec<Choice<'_>> = labels
        .iter()
        .map(|l| Choice {
            label: l,
            hint: None,
        })
        .collect();
    picker(
        s.ui,
        pick_id(&pick),
        name,
        PickerLook::Field,
        &choices,
        chosen,
    )
}

/// `block text-xs text-gray-500 mb-1`.
fn label(t: &str) -> El {
    text(t, 12.0, k::GRAY_500)
        .line_height(16.0)
        .mb(4.0)
        .role(Role::Label, t)
}

/// The label row: name, a red star when required, "· encrypted" for secrets.
fn label_row(name: El, required: bool, secret: bool) -> El {
    let mut row = div().row().items_center().gap(6.0).mb(4.0).child(name);
    if required {
        row = row.child(text("*", 12.0, k::RED_400).line_height(16.0).silent());
    }
    if secret {
        row = row.child(
            text("· ENCRYPTED", 9.0, k::GRAY_600)
                .tracking(0.05)
                .line_height(12.0)
                .silent(),
        );
    }
    row
}

/// The project `<select>`.
fn project(s: &Screen<'_>, a: &Adding) -> El {
    let names: Vec<String> = s.store.projects.iter().map(|p| p.name.clone()).collect();
    let chosen = a
        .project
        .as_ref()
        .and_then(|p| names.iter().position(|n| n == p))
        .unwrap_or(0);
    div()
        .col()
        .child(label("Project"))
        .child(select(s, Pick::Project, "Project", &names, chosen))
}

/// The red line a failed save leaves.
fn error_line(e: &str) -> El {
    div()
        .row()
        .items_start()
        .gap(4.0)
        .child(
            div()
                .mt(2.0)
                .child(ico("circle-alert", 12.0, 2.0, k::RED_400)),
        )
        .child(
            text(e, 11.0, k::RED_400)
                .line_height(16.0)
                .wrapping()
                .grow()
                .min_w0(),
        )
}

/// The filled "Connect" beside a bare "Cancel".
fn buttons(s: &Screen<'_>, label: &str, enabled: bool, busy: bool, done: PlugAct) -> El {
    let mut b = div()
        .row()
        .items_center()
        .justify_center()
        .gap(6.0)
        .px(16.0)
        .py(6.0)
        .rounded(R)
        .role(Role::Button, label);
    let mut fg = k::WHITE;
    if enabled {
        let id = s.acts.on("plug-save", act(done));
        let hot = s.ui.hover(id);
        b = b
            .id(id)
            .on(Action::Click)
            .ring(R)
            .bg(white(0.1 + 0.05 * hot));
    } else {
        fg = k::WHITE.alpha(0.5);
        b = b.bg(white(0.05)).disabled(true);
    }
    if busy {
        b = b.child(ico("loader-circle", 12.0, 2.0, fg));
    }
    b = b.child(text(label, 14.0, fg).line_height(20.0).silent());
    div()
        .row()
        .gap(8.0)
        .pt(4.0)
        .child(b)
        .child(div().px(16.0).py(6.0).child(text_button(
            s,
            "plug-cancel",
            PlugAct::CloseForm,
            "Cancel",
            14.0,
            k::GRAY_500,
            k::GRAY_300,
        )))
}

/// `DynamicField`: one field a built-in's manifest declared.
fn dynamic(s: &Screen<'_>, f: &BuiltinForm, spec: &Value) -> Option<El> {
    let key = sv(spec, "key")?;
    let name = sv(spec, "label").unwrap_or(key);
    let kind = sv(spec, "type").unwrap_or("text");
    let required = spec.get("required") == Some(&Value::Bool(true));
    let value = f.values.get(key).map_or("", String::as_str);
    let mut col = div().col().child(label_row(
        text(name, 12.0, k::GRAY_500)
            .line_height(16.0)
            .role(Role::Label, name),
        required,
        kind == "password",
    ));
    let placeholder = sv(spec, "placeholder").unwrap_or("");
    col = col.child(match kind {
        "select" => {
            let options: Vec<&Value> = spec
                .get("options")
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            let mut labels = vec!["—".to_owned()];
            let mut chosen = 0;
            for (i, o) in options.iter().enumerate() {
                let v = sv(o, "value").or_else(|| o.as_str()).unwrap_or("");
                labels.push(sv(o, "label").unwrap_or(v).to_owned());
                if v == value && !value.is_empty() {
                    chosen = i + 1;
                }
            }
            select(s, Pick::Select(key.to_owned()), name, &labels, chosen)
        }
        "textarea" => {
            let fld = Field::Auth(key.to_owned());
            let id = field_id(&fld);
            s.acts.text(id, act(PlugAct::Text(fld)), value);
            let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
            let style = TextStyle {
                line_height: 20.0,
                ..TextStyle::ui(14.0)
            };
            textarea(id, name, placeholder, style, k::GRAY_200)
                .placeholder_color(k::GRAY_600)
                .w_full()
                .h(74.0)
                .px(12.0)
                .py(6.0)
                .rounded(R)
                .bg(white(0.05))
                .border(1.0, white(0.1).mix(white(0.2), focus))
        }
        _ => input(
            s,
            Field::Auth(key.to_owned()),
            value,
            name,
            placeholder,
            false,
        ),
    });
    if let Some(d) = sv(spec, "description").filter(|d| !d.is_empty()) {
        col = col.child(
            text(d, 10.0, k::GRAY_600)
                .line_height(15.0)
                .mt(2.0)
                .wrapping(),
        );
    }
    Some(col)
}

/// `AddConnectionForm` for a built-in: project, repo, declared fields.
fn builtin(s: &Screen<'_>, a: &Adding, f: &BuiltinForm) -> El {
    let manifest =
        s.st.plugins
            .builtin(&f.connector)
            .and_then(|b| b.get("manifest"));
    let detects = manifest.is_some_and(|m| m.get("detectRepo") == Some(&Value::Bool(true)));
    let mut col = div().col().gap(12.0).child(project(s, a));
    if detects {
        let line = match &f.detected {
            None => Some(
                text("Detecting repository...", 12.0, k::GRAY_500)
                    .line_height(16.0)
                    .silent(),
            ),
            Some(Some((owner, repo))) => Some(
                div()
                    .row()
                    .items_center()
                    .gap(4.0)
                    .child(ico("check", 12.0, 2.0, k::GREEN_400))
                    .child(
                        text(format!("Detected: {owner}/{repo}"), 12.0, k::GREEN_400)
                            .line_height(16.0),
                    ),
            ),
            Some(None) if a.project.is_some() => Some(
                text(
                    "No GitHub repo detected. Enter the repo manually below.",
                    12.0,
                    k::AMBER_400,
                )
                .line_height(16.0)
                .wrapping(),
            ),
            Some(None) => None,
        };
        col = col.child_if(line);
        if matches!(f.detected, Some(None)) {
            let half = |l: &str, fld: Field, v: &str, ph: &str| {
                div()
                    .col()
                    .flex(1.0)
                    .min_w0()
                    .child(label(l))
                    .child(input(s, fld, v, l, ph, false))
            };
            col = col.child(
                div()
                    .row()
                    .gap(8.0)
                    .child(half("Owner", Field::Owner, &f.owner, "e.g. octocat"))
                    .child(half("Repository", Field::Repo, &f.repo, "e.g. hello-world")),
            );
        }
    }
    for key in ["auth", "taskFilters"] {
        for spec in manifest_fields(manifest, key) {
            col = col.child_if(dynamic(s, f, spec));
        }
    }
    if let Some(e) = &a.error {
        col = col.child(error_line(e));
    }
    let label = if a.saving { "Connecting…" } else { "Connect" };
    col.child(buttons(
        s,
        label,
        !a.saving && f.can_save(manifest),
        a.saving,
        PlugAct::Save,
    ))
}

/// A bordered `px-3 py-2` panel inside the SDK form.
fn panel() -> El {
    div()
        .col()
        .px(12.0)
        .py(8.0)
        .rounded(R)
        .bg(white(0.03))
        .border(1.0, white(0.08))
}

fn small(t: impl Into<String>, color: crate::ui::Rgba) -> El {
    text(t, 11.0, color).line_height(16.0).wrapping()
}

/// `SdkConnectorForm`: look a package up, then ask what it needs.
fn sdk(s: &Screen<'_>, a: &Adding, f: &SdkForm) -> El {
    let mut col = div().col().gap(12.0);
    if f.listing.is_none() {
        let can = !f.probing && !f.spec.trim().is_empty();
        let mut look_up = div()
            .row()
            .items_center()
            .gap(6.0)
            .px(12.0)
            .py(6.0)
            .shrink0()
            .rounded(R)
            .role(Role::Button, "Look up");
        let mut fg = k::WHITE;
        if can {
            let id = s.acts.on("plug-probe", act(PlugAct::Probe));
            look_up = look_up
                .id(id)
                .on(Action::Click)
                .ring(R)
                .bg(white(0.1 + 0.05 * s.ui.hover(id)));
        } else {
            fg = k::WHITE.alpha(0.5);
            look_up = look_up.bg(white(0.05)).disabled(true);
        }
        let (g, l) = if f.probing {
            ("loader-circle", "Reading…")
        } else {
            ("search", "Look up")
        };
        look_up = look_up
            .child(ico(g, 12.0, 2.0, fg))
            .child(text(l, 14.0, fg).line_height(20.0).silent());
        col = col.child(
            div()
                .col()
                .child(label("Connector package"))
                .child(
                    div()
                        .row()
                        .gap(8.0)
                        .child(
                            div().grow().min_w0().child(input(
                                s,
                                Field::Spec,
                                &f.spec,
                                "Connector package",
                                "@vornrun/connector-kusto",
                                false,
                            )),
                        )
                        .child(look_up),
                )
                .child(
                    small(
                        "An npm package name, or a command to run a connector from a local checkout.",
                        k::GRAY_600,
                    )
                    .mt(4.0),
                ),
        );
    }
    if f.probing {
        col = col.child(small(
            "Downloading and starting the connector to read what it needs. This can take a moment the first time.",
            k::GRAY_500,
        ));
    }
    let rung = f.rung();
    if let Some(m) = &f.manifest {
        col = col.child(summary(m));
        if rung == Some(Rung::None) {
            col = col.child(small(
                "This connector asks for no sign-in — installing it was the whole setup, and its actions are ready to use in a workflow.",
                k::GRAY_500,
            ));
        }
        let triggers: Vec<&Value> = m
            .get("triggers")
            .and_then(Value::as_array)
            .map(|t| t.iter().collect())
            .unwrap_or_default();
        if !triggers.is_empty() && rung != Some(Rung::None) {
            let labels: Vec<String> = triggers
                .iter()
                .map(|t| sv(t, "label").unwrap_or("").to_owned())
                .collect();
            let mut t = div().col().child(label("Trigger")).child(select(
                s,
                Pick::Trigger,
                "Trigger",
                &labels,
                f.trigger,
            ));
            if let Some(d) = f.trigger().and_then(|t| sv(t, "description")) {
                t = t.child(small(d, k::GRAY_600).mt(4.0));
            }
            col = col.child(t);
        }
        if rung != Some(Rung::None) {
            col = col.child(project(s, a));
        }
        if rung == Some(Rung::Cli) {
            col = col.child(cli(s, f, m));
        }
        if rung == Some(Rung::Browser) {
            col = col.child_if(browser(m));
        }
        for e in f.fields() {
            let Some(name) = sv(e, "name") else { continue };
            let secret = e.get("secret") == Some(&Value::Bool(true));
            let required = e.get("required") == Some(&Value::Bool(true));
            let value = f.values.get(name).map_or("", String::as_str);
            let mut fcol = div()
                .col()
                .child(label_row(
                    text(name, 11.0, k::GRAY_500)
                        .mono()
                        .line_height(16.0)
                        .role(Role::Label, name),
                    required,
                    secret,
                ))
                .child(input(
                    s,
                    Field::Env(name.to_owned()),
                    value,
                    name,
                    "",
                    false,
                ));
            if let Some(d) = sv(e, "description").filter(|d| !d.is_empty()) {
                fcol = fcol.child(small(d, k::GRAY_600).mt(4.0));
            }
            col = col.child(fcol);
        }
    }
    if let Some(e) = &a.error {
        col = col.child(error_line(e));
    }
    let (label, done) = match rung {
        Some(Rung::None) => ("Done", PlugAct::CloseForm),
        _ if a.saving => ("Connecting…", PlugAct::Save),
        Some(Rung::Browser) => ("Connect and sign in", PlugAct::Save),
        _ => ("Connect", PlugAct::Save),
    };
    let enabled = f.manifest.is_some() && !a.saving && (rung == Some(Rung::None) || !f.missing());
    col.child(buttons(s, label, enabled, a.saving, done))
}

/// The probed connector: glyph, name, version, description, actions.
fn summary(m: &Value) -> El {
    let name = sv(m, "name").unwrap_or("");
    let version = sv(m, "version").unwrap_or("");
    let icon = m
        .get("icon")
        .filter(|i| !i.is_null())
        .and_then(crate::library::parse_icon);
    let mark = match icon {
        Some(i) => crate::ui::icon(
            crate::library::mark_svg(&crate::library::Look {
                connector: "sdk".to_owned(),
                packaged: true,
                icon: Some(i),
            })
            .unwrap_or_default(),
            13.0,
            k::GRAY_200,
        ),
        None => ico("check", 12.0, 2.0, k::GREEN_400),
    };
    let mut p = panel().child(
        div()
            .row()
            .items_center()
            .gap(6.0)
            .child(mark)
            .child(text(name, 14.0, k::GRAY_200).weight(500).line_height(20.0))
            .child(text(format!("v{version}"), 11.0, k::GRAY_500).line_height(16.0)),
    );
    if let Some(d) = sv(m, "description").filter(|d| !d.is_empty()) {
        p = p.child(small(d, k::GRAY_500).mt(4.0));
    }
    let actions = m
        .get("actions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if actions > 0 {
        let plural = if actions == 1 { "" } else { "s" };
        p = p.child(
            small(
                format!("{actions} action{plural} available to workflow steps."),
                k::GRAY_600,
            )
            .mt(4.0),
        );
    }
    p
}

/// The borrowed CLI login: who it is signed in as, and the token switch.
fn cli(s: &Screen<'_>, f: &SdkForm, m: &Value) -> El {
    let probe = m.pointer("/auth/probe");
    let command = probe.and_then(|p| sv(p, "command"));
    let mut p = panel();
    let ok = f
        .identity
        .as_ref()
        .and_then(|i| i.get("ok"))
        .and_then(Value::as_bool)
        == Some(true);
    if ok {
        let who = f
            .identity
            .as_ref()
            .and_then(|i| sv(i, "identity"))
            .map_or("Signed in".to_owned(), |w| format!("Signed in as {w}"));
        let mut line = div()
            .row()
            .items_center()
            .gap(6.0)
            .child(super::dot(k::STATUS_SAGE))
            .child(text(who, 14.0, k::GRAY_200).line_height(20.0));
        if let Some(c) = command {
            let args: Vec<&str> = probe
                .and_then(|p| p.get("args"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            line = line.child(
                text(format!("· from {c} {}", args.join(" ")), 11.0, k::GRAY_500)
                    .mono()
                    .line_height(16.0),
            );
        }
        p = p.child(line);
    } else {
        let said = match &f.identity {
            None => div()
                .row()
                .items_center()
                .gap(6.0)
                .child(ico("loader-circle", 11.0, 2.0, k::GRAY_400))
                .child(small(
                    format!(
                        "Checking whether {} is signed in…",
                        command.unwrap_or("the tool it borrows")
                    ),
                    k::GRAY_400,
                )),
            Some(i) => small(
                sv(i, "message").unwrap_or("Nothing to check for this connector."),
                k::GRAY_400,
            ),
        };
        p = p.child(said);
        if let Some(h) = f.identity.as_ref().and_then(|i| sv(i, "installHint")) {
            p = p.child(small(h, k::GRAY_600).mt(4.0));
        }
    }
    let borrowed = if f.use_token {
        Vec::new()
    } else {
        crate::plugins::borrowable(m)
    };
    if !borrowed.is_empty() {
        p = p.child(
            small(
                format!(
                    "Hands over {} from {}.",
                    borrowed.join(", "),
                    command.unwrap_or("")
                ),
                k::GRAY_600,
            )
            .mt(4.0),
        );
    }
    let swap = if f.use_token {
        "Borrow the signed-in tool instead"
    } else {
        "Use a token instead"
    };
    p.child(div().row().mt(6.0).child(text_button(
        s,
        "plug-token",
        PlugAct::UseToken,
        swap,
        11.0,
        k::GRAY_500,
        k::GRAY_300,
    )))
}

/// A browser sign-in: where the window opens and which origins it acts on.
fn browser(m: &Value) -> Option<El> {
    let b = m.pointer("/auth/browser")?;
    let url = sv(b, "signInUrl")?;
    let host = url
        .split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or(url);
    let origins: Vec<&str> = b
        .get("origins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|o| o.strip_prefix("https://").unwrap_or(o))
        .collect();
    Some(
        panel()
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(6.0)
                    .child(super::dot(k::INK_GHOST))
                    .child(
                        text("Signs in through a Vorn window", 14.0, k::GRAY_200)
                            .line_height(20.0),
                    ),
            )
            .child(
                small(
                    format!(
                        "After Connect, a window opens on {host}. The login stays in a browser profile only this connection uses, and it acts as you on {}.",
                        origins.join(", ")
                    ),
                    k::GRAY_600,
                )
                .mt(4.0),
            ),
    )
}

/// `HttpProfileForm`: name, base URL, auth header and secret, then a test.
fn profile(s: &Screen<'_>, a: &Adding, f: &ProfileForm) -> El {
    let fieldset = |l: &str, fld: Field, v: &str, ph: &str, mono: bool| {
        let id = field_id(&fld);
        s.acts.text(id, act(PlugAct::Text(fld)), v);
        let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
        let style = TextStyle {
            line_height: 16.0,
            mono,
            ..TextStyle::ui(12.0)
        };
        div()
            .col()
            .mb(8.0)
            .child(
                text(l, 11.0, k::GRAY_500)
                    .line_height(16.0)
                    .mb(4.0)
                    .role(Role::Label, l),
            )
            .child(
                field(id, l, ph, style, k::GRAY_200)
                    .placeholder_color(k::GRAY_600)
                    .w_full()
                    .px(8.0)
                    .py(6.0)
                    .rounded(R)
                    .bg(white(0.05))
                    .border(1.0, white(0.1).mix(white(0.2), focus)),
            )
    };
    let mut col = div()
        .col()
        .px(12.0)
        .py(10.0)
        .rounded(R)
        .bg(white(0.03))
        .border(1.0, white(0.08))
        .child(
            text("HTTP profile", 12.0, k::GRAY_200)
                .weight(500)
                .line_height(16.0)
                .mb(8.0)
                .role(Role::Heading, "HTTP profile"),
        )
        .child(fieldset("Name", Field::ProfileName, &f.name, "", false))
        .child(fieldset(
            "Base URL",
            Field::ProfileUrl,
            &f.base_url,
            "https://api.example.com",
            false,
        ))
        .child(fieldset(
            "Auth header",
            Field::ProfileHeader,
            &f.header,
            "",
            true,
        ))
        .child(fieldset(
            "Secret",
            Field::ProfileSecret,
            &f.secret,
            "",
            false,
        ));
    if let Some((ok, message)) = &f.result {
        let (g, color) = if *ok {
            ("check", k::STATUS_SAGE)
        } else {
            ("circle-alert", k::DANGER)
        };
        col = col.child(
            div()
                .row()
                .items_start()
                .gap(6.0)
                .mt(8.0)
                .child(div().mt(2.0).child(ico(g, 12.0, 2.0, color)))
                .child(small(message.as_str(), color).grow().min_w0()),
        );
    }
    if let Some(e) = &a.error {
        col = col.child(small(e.as_str(), k::DANGER).mt(8.0));
    }
    let ready = !a.saving && f.ready();
    let label = if a.saving {
        "Testing…"
    } else {
        "Save and test"
    };
    let mut save = div()
        .row()
        .px(10.0)
        .py(4.0)
        .rounded(R)
        .border(1.0, white(0.1))
        .role(Role::Button, label);
    let mut fg = k::GRAY_200;
    if ready {
        let id = s.acts.on("plug-save", act(PlugAct::Save));
        let hot = s.ui.hover(id);
        fg = fg.mix(k::WHITE, hot);
        save = save.id(id).on(Action::Click).ring(R).bg(white(0.06 * hot));
    } else {
        save = save.opacity(0.5).disabled(true);
    }
    let cancel_id = s.acts.on("plug-cancel", act(PlugAct::CloseForm));
    let chot = s.ui.hover(cancel_id);
    let cancel = div()
        .id(cancel_id)
        .on(Action::Click)
        .ring(R)
        .px(10.0)
        .py(4.0)
        .rounded(R)
        .bg(white(0.06 * chot))
        .role(Role::Button, "Cancel")
        .child(
            text("Cancel", 11.0, k::GRAY_400.mix(k::WHITE, chot))
                .line_height(16.0)
                .silent(),
        );
    col.child(
        div()
            .row()
            .justify_end()
            .gap(8.0)
            .mt(12.0)
            .child(cancel)
            .child(save.child(text(label, 11.0, fg).line_height(16.0).silent())),
    )
}
