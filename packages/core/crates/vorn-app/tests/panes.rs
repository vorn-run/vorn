//! A session's git and panes driven as a user would, on a vornd with its
//! own HOME: the changes indicator opens the Changes panel and its commit,
//! the branch chip switches branches, the Files pane browses, edits, finds,
//! saves and pops a file out, the Terminals panel holds shells, and a
//! worktree is offered for removal when its last session ends.
//! `VORN_APP_SHOT=<prefix>` saves a frame of each surface.

#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::{git, key, seed_acme, wait_for, Shot, TestVornd};
use vorn_app::act::Act;
use vorn_app::app::MOD;
use vorn_app::editor::EditAct;
use vorn_app::git::{CommitOp, GitAct};
use vorn_app::panes::{tab_key, PaneAct};
use vorn_app::screen::panes::editor_id;
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
    config["agentCommands"]["claude"] = json!({ "command": "/bin/sh", "args": [] });
    app.rpc().call("config:save", config).expect("saved");
    let info = app
        .rpc()
        .call(
            "terminal:create",
            json!({ "agentType": "claude", "projectName": "acme", "projectPath": dir }),
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
    shot.send(&mut app, key("Enter", MOD, Some("\r")));
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
fn panes_browse_edit_save_and_hold_terminals() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut shot = Shot::new();
    let sid = agent(&mut shot, &mut app, &repo);

    // Browse files: the tree lists the repository.
    hover_card(&mut shot, &mut app, &sid);
    shot.click(
        &mut app,
        control("card", Act::Pane(PaneAct::ToggleFiles(sid.clone()))),
    );
    shot.until(&mut app, "the tree", |a, _| {
        a.st.panes.trees.get(&sid).and_then(|t| t.root.clone())
    });
    let src = repo.join("src").to_string_lossy().into_owned();
    shot.click(
        &mut app,
        control(
            "tree-dir",
            Act::Pane(PaneAct::Dir(sid.clone(), src.clone())),
        ),
    );
    shot.until(&mut app, "src", |a, _| {
        a.st.panes
            .trees
            .get(&sid)
            .and_then(|t| t.dirs.get(&src).cloned())
    });
    let file = repo.join("src/app.ts").to_string_lossy().into_owned();
    shot.click(
        &mut app,
        control(
            "tree-file",
            Act::Pane(PaneAct::File(sid.clone(), file.clone())),
        ),
    );
    let key_ = tab_key(&sid, &file);
    shot.until(&mut app, "the file in its editor", |a, _| {
        a.st.editors
            .buffers
            .get(&key_)
            .filter(|b| b.ready)
            .map(|_| ())
    });
    assert_eq!(
        app.st.panes.saved.files[&sid].preview.as_deref(),
        Some(file.as_str())
    );
    shot.save(&mut app, "p-files");

    // Typing marks it unsaved and pins the preview; Mod+S writes it.
    shot.click(&mut app, editor_id(&key_));
    shot.type_text(&mut app, "// edited\n");
    assert!(app.st.editors.dirty(&key_));
    shot.frame(&mut app);
    assert_eq!(app.st.panes.saved.files[&sid].preview, None);
    shot.save(&mut app, "p-dirty");
    shot.send(&mut app, key("KeyS", MOD, None));
    shot.until(&mut app, "the save", |a, _| {
        a.st.editors
            .buffers
            .get(&key_)
            .filter(|b| !b.dirty() && !b.saving)
            .map(|_| ())
    });
    let on_disk = std::fs::read_to_string(&file).expect("read");
    assert!(on_disk.contains("// edited"), "{on_disk}");

    // Find counts and steps through the matches.
    shot.click(
        &mut app,
        control("editor-tool", Act::Edit(EditAct::ToggleFind(key_.clone()))),
    );
    shot.type_text(&mut app, "name");
    let hits = app.st.editors.buffers[&key_].matches().len();
    assert_eq!(hits, 4, "names, name, names, name");
    shot.send(&mut app, key("Enter", 0, Some("\r")));
    assert_eq!(
        app.st.editors.buffers[&key_].find.as_ref().map(|f| f.index),
        Some(1)
    );
    shot.save(&mut app, "p-find");
    shot.send(&mut app, key("Escape", 0, None));
    assert!(app.st.editors.buffers[&key_].find.is_none());

    // An edit left unsaved asks before its tab closes, and a draft keeps it.
    // A second click on the same spot would be a double click selecting a word.
    shot.ui.set_focus(Some(editor_id(&key_)));
    shot.type_text(&mut app, "x");
    std::thread::sleep(std::time::Duration::from_millis(450));
    shot.frame(&mut app);
    let drafts = std::fs::read_to_string(v.home.join("desktop/drafts.json")).expect("drafts");
    assert!(drafts.contains(&key_), "{drafts}");
    // An unsaved tab shows its dot until hovered, then its close button.
    shot.hover(
        &mut app,
        control(
            "files-tab",
            Act::Pane(PaneAct::Tab(sid.clone(), file.clone())),
        ),
    );
    shot.click(
        &mut app,
        control(
            "files-tab-close",
            Act::Pane(PaneAct::CloseTab(sid.clone(), file.clone())),
        ),
    );
    assert_eq!(app.st.dialog, Some(Dialog::Discard));
    shot.save(&mut app, "p-discard");
    shot.click(
        &mut app,
        control("discard-cancel", Act::Edit(EditAct::CancelDiscard)),
    );
    assert!(app.st.panes.saved.files[&sid].tabs.contains(&file));

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
    let ratio = app.st.panes.terminal_ratio(&sid);
    assert!((ratio - 0.3).abs() < 0.05, "{ratio}");
    let view = std::fs::read_to_string(v.home.join("desktop/view.json")).expect("view");
    assert!(view.contains("\"terminal\""), "{view}");

    // A file pops out into a card of its own beside the session, and back.
    let readme = repo.join("README.md").to_string_lossy().into_owned();
    shot.click(
        &mut app,
        control(
            "tree-pop",
            Act::Pane(PaneAct::PopOut(sid.clone(), readme.clone())),
        ),
    );
    let card = app
        .st
        .panes
        .cards_of(&sid)
        .first()
        .map(|c| (*c).to_owned())
        .expect("a card");
    shot.until(&mut app, "the card's file", |a, _| {
        a.st.editors
            .buffers
            .get(&card)
            .filter(|b| b.ready)
            .map(|_| ())
    });
    shot.save(&mut app, "p-card");
    shot.click(
        &mut app,
        control("card-return", Act::Pane(PaneAct::ReturnCard(card.clone()))),
    );
    assert!(app.st.panes.saved.cards.is_empty());
    assert!(app.st.panes.saved.files[&sid].tabs.contains(&readme));

    // The terminals panel starts a shell of its own, kept out of the grid.
    hover_card(&mut shot, &mut app, &sid);
    shot.click(
        &mut app,
        control("card", Act::Pane(PaneAct::ToggleTerminals(sid.clone()))),
    );
    let held = shot.until(&mut app, "the panel's shell", |a, _| {
        a.st.panes
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
            Act::Pane(PaneAct::Extract(sid.clone(), held.clone())),
        ),
    );
    assert!(!app.st.panes.saved.panels.contains_key(&sid));
    assert!(app
        .st
        .visible(&app.store)
        .iter()
        .any(|&i| app.store.sessions[i].info.id == held));

    // The view survives the app: a new one finds the pane and the draft.
    drop(app);
    let mut again = support::connect(&v);
    let mut shot = Shot::new();
    shot.until(&mut again, "the restored draft", |a, _| {
        a.st.editors
            .buffers
            .get(&key_)
            .filter(|b| b.dirty())
            .map(|_| ())
    });
    let restored = &again.st.editors.buffers[&key_].draft;
    assert!(restored.contains("// edited"), "{restored:?}");
}

#[test]
fn a_worktree_whose_last_session_ends_is_offered_for_removal() {
    let Some(v) = TestVornd::start() else {
        return;
    };
    let mut app = support::connect(&v);
    let repo = seed_acme(&mut app, &v);
    let mut config = app.rpc().call("config:load", Value::Null).expect("config");
    config["agentCommands"]["claude"] = json!({ "command": "/bin/sh", "args": [] });
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
            json!({ "agentType": "claude", "projectName": "acme", "projectPath": repo, "existingWorktreePath": wt }),
        )
        .expect("a session in the worktree");
    let sid = info["id"].as_str().expect("id").to_owned();
    shot.until(&mut app, "the worktree row", |a, _| {
        a.st.worktrees
            .get(&repo.to_string_lossy().into_owned())
            .filter(|l| l.iter().any(|w| w.path == wt))
            .map(|_| ())
    });
    let remove = Act::Git(GitAct::Remove {
        project: repo.to_string_lossy().into_owned(),
        worktree: wt.clone(),
    });
    app.click(&mut shot.ui, App::control("sidebar-worktree", &remove));
    shot.frame(&mut app);
    if app.st.dialog.is_none() {
        shot.hover(
            &mut app,
            vorn_app::act::Acts::id("sidebar-worktree", &Act::Nothing).child(&wt),
        );
        shot.click(&mut app, control("sidebar-worktree", remove));
    }
    shot.until(&mut app, "the removal dialog", |a, _| {
        a.st.git
            .removal
            .as_ref()
            .filter(|r| r.dirty != vorn_app::git::Dirty::Checking)
            .map(|_| ())
    });
    assert_eq!(
        app.st.git.removal.as_ref().map(|r| r.sessions.clone()),
        Some(vec![sid.clone()])
    );
    shot.save(&mut app, "g-dialog");
    shot.click(
        &mut app,
        control("removal-cancel", Act::Git(GitAct::CancelRemoval)),
    );

    // Closing its last session offers the worktree; Remove takes it away.
    app.rpc().call("terminal:kill", json!(sid)).expect("kill");
    shot.until(&mut app, "the offer", |a, _| {
        a.st.git
            .offers
            .iter()
            .find(|o| o.worktree == wt && o.dirty.is_some())
            .map(|_| ())
    });
    shot.save(&mut app, "g-toast");
    shot.click(
        &mut app,
        control("toast-action", Act::Git(GitAct::RemoveOffered(wt.clone()))),
    );
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
        Act::Set(vorn_app::settings::SetAct::CheckUpdates),
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
