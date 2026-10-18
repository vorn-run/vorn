//! A session's git and panes driven as a user would, on a vornd with its
//! own HOME: the changes indicator opens the Changes panel and its commit,
//! the branch chip switches branches, the panes split the card, the
//! Terminals panel holds shells, and a worktree is offered for removal when
//! its last session ends. The Files pane's own steps are in `files_blocks.rs`.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each surface.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{git, key, seed_acme, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::git::{CommitOp, GitAct};
use vorn_app::points::PointAct;
use vorn_app::side::SideAct;
use vorn_app::state::Dialog;
use vorn_app::App;

fn control(place: &str, act: Act) -> vorn_app::ui::Id {
    App::control(place, &act)
}

/// A shell in `dir`, selected, with its screen on the card.
fn shell(shot: &mut Shot, app: &mut App, dir: &std::path::Path) -> String {
    let info = app.rpc().call("shell:create", json!(dir)).expect("shell");
    let id = info["id"].as_str().expect("id").to_owned();
    shot.until(app, "the shell's card", |a, _| {
        a.store.session(&id).map(|_| ())
    });
    shot.click(app, control("card-box", Act::Select(id.clone())));
    id
}

/// An agent session in `dir`, its agent a plain shell, selected.
fn agent(shot: &mut Shot, app: &mut App, dir: &std::path::Path) -> String {
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"]["acme"] = json!({ "command": "/bin/sh", "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "acme", "projectPath": dir }),
        )
        .expect("agent");
    let id = info["id"].as_str().expect("id").to_owned();
    shot.until(app, "the agent's card", |a, _| {
        a.store.session(&id).map(|_| ())
    });
    shot.click(app, control("card-box", Act::Select(id.clone())));
    id
}

/// Shows the card header's actions, which appear on hover.
fn hover_card(shot: &mut Shot, app: &mut App, id: &str) {
    shot.hover(app, control("card-box", Act::Select(id.to_owned())));
}

#[test]
fn git_follows_a_sessions_changes_into_a_commit_and_a_branch() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    git(&repo, &["branch", "release"]);
    let mut shot = Shot::new();
    let sid = shell(&mut shot, &mut app, &repo);

    // The poll reads the changes; the card shows them and opens the panel.
    shot.until(&mut app, "the changes count", |a, _| {
        a.st.git.stats.get(&sid).filter(|s| s.changed()).map(|_| ())
    });
    let stat = app.st.git.stats[&sid];
    assert_eq!(
        (stat.insertions, stat.deletions, stat.files),
        (4, 2, 2),
        "{stat:?}"
    );
    shot.click(
        &mut app,
        control("git-indicator", Act::Git(GitAct::Open(sid.clone()))),
    );
    shot.until(&mut app, "the diff", |a, _| {
        (!a.st.git.diff.files.is_empty()).then_some(())
    });
    let paths: Vec<&str> = app
        .st
        .git
        .diff
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(
        paths.contains(&"src/app.ts") && paths.contains(&"README.md"),
        "{paths:?}"
    );
    shot.save(&mut app, "p-changes");

    // Its left edge drags the width, following the pointer.
    let handle = vorn_app::screen::git::changes_resize_id();
    shot.frame(&mut app);
    let r = shot.ui.rect_of(handle).expect("the resize handle");
    let before = app.st.git.width();
    shot.ui.pointer_move(r.x + 2.0, r.y + 40.0);
    shot.ui.pointer_down();
    shot.ui.pointer_move(r.x - 100.0, r.y + 40.0);
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let after = app.st.git.width();
    assert!((after - before - 100.0).abs() < 8.0, "{before} -> {after}");

    // A review comment on a changed line, sent to the session.
    let line = app.st.git.diff.files[0]
        .diff
        .split('\n')
        .position(|l| l.starts_with('+') && !l.starts_with("+++"))
        .expect("an added line");
    let file = app.st.git.diff.files[0].path.clone();
    shot.click(
        &mut app,
        control(
            "git-diff-line",
            Act::Git(GitAct::Diff(vorn_app::git::DiffOp::Line(file, line))),
        ),
    );
    shot.type_text(&mut app, "Name this better");
    shot.send(&mut app, key("Enter", MOD(), Some("\r")));
    assert_eq!(app.st.git.diff.comments.len(), 1);
    shot.save(&mut app, "p-review");
    shot.click(
        &mut app,
        control(
            "changes-clear",
            Act::Git(GitAct::Diff(vorn_app::git::DiffOp::ClearComments)),
        ),
    );

    // Commit with the unstaged changes: the panel empties and says so.
    shot.click(
        &mut app,
        control("changes-commit", Act::Git(GitAct::Commit(CommitOp::Open))),
    );
    assert_eq!(app.st.dialog, Some(Dialog::GitCommit));
    shot.type_text(&mut app, "Greet the team");
    shot.save(&mut app, "p-commit");
    shot.click(
        &mut app,
        control("git-commit-go", Act::Git(GitAct::Commit(CommitOp::Commit))),
    );
    shot.until(&mut app, "the commit", |a, _| {
        assert_eq!(a.st.git.commit.error, None, "the commit failed");
        (a.st.dialog.is_none() && a.st.toasts.iter().any(|t| t.text == "Changes committed"))
            .then_some(())
    });
    shot.until(&mut app, "the empty diff", |a, _| {
        (!a.st.git.diff.loading && a.st.git.diff.files.is_empty()).then_some(())
    });
    let log = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(&repo)
        .output()
        .expect("git log");
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "Greet the team"
    );
    shot.click(&mut app, control("changes", Act::Git(GitAct::Close)));
    assert!(app.st.git.panel.is_none());

    // The branch chip lists the branches and switches the checkout.
    shot.until(&mut app, "the branch", |a, _| {
        a.store.session(&sid).and_then(|s| s.info.branch.clone())
    });
    shot.click(
        &mut app,
        control("branch-chip", Act::Git(GitAct::Branches(sid.clone()))),
    );
    shot.until(&mut app, "the branches", |a, _| {
        a.st.git
            .branches
            .as_ref()
            .filter(|b| !b.loading)
            .map(|_| ())
    });
    let names = app
        .st
        .git
        .branches
        .as_ref()
        .map(|b| b.local.clone())
        .unwrap_or_default();
    assert!(names.contains(&"release".to_owned()), "{names:?}");
    shot.save(&mut app, "c-branch");
    shot.click(
        &mut app,
        control("branch-pick", Act::Git(GitAct::Switch("release".into()))),
    );
    shot.until(&mut app, "the switch", |a, _| {
        a.st.toasts
            .iter()
            .any(|t| t.text == "Switched to 'release'")
            .then_some(())
    });
    let head = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&repo)
        .output()
        .expect("git branch");
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "release");
    assert_eq!(
        app.store
            .session(&sid)
            .and_then(|s| s.info.branch.clone())
            .as_deref(),
        Some("release")
    );

    // Open in: the editors vornd found, when it found any here.
    shot.until(&mut app, "the editors", |a, _| {
        a.st.git.ides.as_ref().map(|_| ())
    });
    if app.st.git.ides.as_ref().is_some_and(|l| !l.is_empty()) {
        shot.click(
            &mut app,
            control("open-in-more", Act::Git(GitAct::OpenInMenu(sid.clone()))),
        );
        let first = app
            .st
            .git
            .ides
            .as_ref()
            .and_then(|l| l.first())
            .map(|i| i.id.clone());
        let pick = Act::Git(GitAct::OpenIn(repo.to_string_lossy().into_owned(), first));
        assert!(
            shot.ui.rect_of(control("open-in-pick", pick)).is_some(),
            "the editors are listed"
        );
        shot.save(&mut app, "c-openin");
    }
}

#[test]
fn panes_split_the_card_and_hold_terminals() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app, &repo);

    // The Files pane opens beside the terminal: the Files extension's, drawn from its tree.
    hover_card(&mut shot, &mut app, &sid);
    let browse = wait_for("Browse files", || {
        app.pump();
        shot.button(&mut app, "Browse files")
    });
    shot.click(&mut app, browse);
    wait_for("the tree", || {
        app.pump();
        shot.labelled(&mut app, "TreeItem", "src")
    });

    // The divider between the terminal and the panes moves and is kept.
    let divider = vorn_app::ui::Id::new("card-divider").child(&sid);
    shot.frame(&mut app);
    let r = shot.ui.rect_of(divider).expect("the card divider");
    let card = shot
        .ui
        .rect_of(control("card-box", Act::Select(sid.clone())))
        .expect("the card");
    shot.ui.pointer_move(r.x + r.w / 2.0, r.y + r.h / 2.0);
    shot.ui.pointer_down();
    shot.ui.pointer_move(card.x + card.w * 0.3, r.y + r.h / 2.0);
    shot.ui.pointer_up();
    app.events(&mut shot.ui);
    let ratio = app.st.side.terminal_ratio(&sid);
    assert!((ratio - 0.3).abs() < 0.05, "{ratio}");
    let view = std::fs::read_to_string(v.home.join("desktop/view.json")).expect("view");
    assert!(view.contains("\"terminal\""), "{view}");

    // The terminals panel starts a shell of its own, kept out of the grid.
    hover_card(&mut shot, &mut app, &sid);
    shot.click(
        &mut app,
        control("card", Act::Side(SideAct::ToggleTerminals(sid.clone()))),
    );
    let held = shot.until(&mut app, "the panel's shell", |a, _| {
        a.st.side
            .saved
            .panels
            .get(&sid)
            .and_then(|p| p.terminals.first().cloned())
    });
    shot.until(&mut app, "its screen", |a, _| a.pane(&held).map(|_| ()));
    let visible: Vec<String> = app
        .st
        .visible(&app.store)
        .into_iter()
        .map(|i| app.store.sessions[i].info.id.clone())
        .collect();
    assert!(!visible.contains(&held), "a held shell is not a grid card");
    shot.save(&mut app, "p-terminals");
    shot.click(
        &mut app,
        control(
            "terminals-tab-extract",
            Act::Side(SideAct::Extract(sid.clone(), held.clone())),
        ),
    );
    assert!(!app.st.side.saved.panels.contains_key(&sid));
    assert!(app
        .st
        .visible(&app.store)
        .iter()
        .any(|&i| app.store.sessions[i].info.id == held));

}

#[test]
fn the_worktree_cleanup_extension_asks_then_offers_idle_worktrees() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"]["acme"] = json!({ "command": "/bin/sh", "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let list = app
        .rpc()
        .call("git:listWorktrees", json!(repo))
        .expect("worktrees");
    let wt = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|w| w["isMain"] != true)
        .and_then(|w| w["path"].as_str())
        .expect("the seed's worktree")
        .to_owned();
    let mut shot = Shot::new();

    // The sidebar lists the worktree; its Remove asks first while a
    // session runs in it.
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "acme", "projectName": "acme", "projectPath": repo, "existingWorktreePath": wt }),
        )
        .expect("a session in the worktree");
    let sid = info["id"].as_str().expect("id").to_owned();
    shot.until(&mut app, "the worktree row", |a, _| {
        a.st.worktrees
            .get(&repo.to_string_lossy().into_owned())
            .filter(|l| l.iter().any(|w| w.path == wt))
            .map(|_| ())
    });
    let key = "vorn.worktrees/menus/remove".to_owned();
    shot.until(&mut app, "the extension's menu item", |a, _| {
        a.st.points.get(&key).map(|_| ())
    });
    let project = repo.to_string_lossy().into_owned();
    let args = json!({ "worktree": wt, "project": project }).to_string();
    let remove = Act::Point(PointAct::Choose { key, args });
    shot.frame(&mut app);
    if !app.click(&mut shot.ui, App::control("sidebar-worktree", &remove)) {
        // First seen without sessions, the project starts folded.
        app.st.collapsed.remove("acme");
        shot.hover(
            &mut app,
            vorn_app::act::Acts::id(
                "sidebar-worktree",
                &Act::Worktree {
                    project: "acme".into(),
                    path: wt.clone(),
                },
            ),
        );
        shot.click(&mut app, control("sidebar-worktree", remove));
    }
    let asked = |a: &App| {
        a.st.points
            .rows_of("vorn.worktrees")
            .get("asked")
            .is_some_and(|r| !r.is_empty())
    };
    let dialog = "vorn.worktrees/dialogs/confirm";
    shot.until(&mut app, "the removal dialog", |a, _| {
        let open = a.st.points.dialog.as_deref() == Some(dialog);
        (open && asked(a) && support::tree_says(a, dialog, "closed first")).then_some(())
    });
    shot.save(&mut app, "g-dialog");
    support::every_size(
        &mut app,
        "g-dialog",
        &["Remove feature-greet?", "closed first", "Cancel"],
    );
    let cancel = shot
        .button(&mut app, "Cancel")
        .expect("the dialog's Cancel");
    shot.click(&mut app, cancel);
    shot.until(&mut app, "the dialog closed", |a, _| {
        a.st.points.dialog.is_none().then_some(())
    });

    // Closing its last session lists the worktree as idle; Remove takes it away.
    app.rpc().call("terminal:kill", json!(sid)).expect("kill");
    let idle = "vorn.worktrees/sidebar/idle";
    shot.until(&mut app, "the idle worktree", |a, _| {
        support::tree_says(a, idle, &format!("remove:{wt}")).then_some(())
    });
    shot.save(&mut app, "g-idle");
    support::every_size(&mut app, "g-idle", &["Idle worktrees", "Keep", "Remove"]);
    let remove = shot
        .button(&mut app, "Remove")
        .expect("the idle row's Remove");
    shot.click(&mut app, remove);
    shot.until(&mut app, "the removal", |a, _| {
        a.st.toasts
            .iter()
            .any(|t| t.text == "Worktree removed")
            .then_some(())
    });
    wait_for("the directory to go", || {
        (!std::path::Path::new(&wt).exists()).then_some(())
    });
}

#[test]
fn a_ready_update_is_offered_in_the_sidebar_until_dismissed() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut shot = Shot::new();
    let _ = shell(&mut shot, &mut app, &repo);
    app.st.settings.update = Some(vorn_app::settings::UpdateStatus::Ready {
        version: "0.8.1".into(),
    });
    let restart = control(
        "update-notice",
        Act::Set(vorn_app::settings::SetAct::UpdateNow),
    );
    shot.frame(&mut app);
    assert!(shot.ui.rect_of(restart).is_some(), "the notice shows");
    shot.save(&mut app, "p-update");
    shot.click(
        &mut app,
        control(
            "update-notice",
            Act::Set(vorn_app::settings::SetAct::DismissUpdate),
        ),
    );
    assert!(shot.ui.rect_of(restart).is_none(), "the notice is gone");
    // The gear keeps a dot and opens the Updates page.
    shot.click(
        &mut app,
        control(
            "sidebar-footer",
            Act::Set(vorn_app::settings::SetAct::Open(
                vorn_app::settings::Category::Updates,
            )),
        ),
    );
    assert!(app.st.settings.open);
    assert_eq!(
        app.st.settings.category,
        vorn_app::settings::Category::Updates
    );
}

#[test]
fn what_is_typed_in_a_panel_terminal_stays_on_screen() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    // bash gets the shell integration, so its commands show as blocks.
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["defaults"]["shell"] = json!("/bin/bash");
    app.rpc().call("config:save", config).expect("saved");
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app, &repo);
    hover_card(&mut shot, &mut app, &sid);
    shot.click(
        &mut app,
        control("card", Act::Side(SideAct::ToggleTerminals(sid.clone()))),
    );
    let held = shot.until(&mut app, "the panel's shell", |a, _| {
        a.st.side
            .saved
            .panels
            .get(&sid)
            .and_then(|p| p.terminals.first().cloned())
    });
    shot.until(&mut app, "its first prompt", |a, _| {
        a.st.shells
            .get(&held)
            .is_some_and(|sh| sh.integrated())
            .then_some(())
    });
    shot.click(&mut app, vorn_app::screen::term_id(&held));
    shot.until(&mut app, "the marked prompt", |a, _| {
        a.is_split(&held).then_some(())
    });
    shot.type_line(&mut app, "echo first-$((1+1))");
    shot.type_line(&mut app, "echo second-$((1+2))");
    // Finished commands stay on screen in the panel, in its log of blocks.
    let log = vorn_app::screen::log_id(&held);
    shot.until(&mut app, "the panel's log of finished commands", |_, ui| {
        ui.rect_of(log).filter(|r| r.h > 1.0).map(|_| ())
    });
    shot.until(&mut app, "both commands as blocks", |a, _| {
        let blocks = &a.st.shells.get(&held)?.blocks;
        let done = |c: &str| blocks.iter().any(|b| b.command.as_deref() == Some(c));
        (done("echo first-$((1+1))") && done("echo second-$((1+2))")).then_some(())
    });
    for _ in 0..30 {
        shot.frame(&mut app);
        app.pump();
        assert!(shot.ui.rect_of(log).is_some_and(|r| r.h > 1.0));
    }
    shot.save(&mut app, "p-terminal-blocks");
}
