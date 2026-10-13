//! The floating widget as the app sees it (`showWidget`, `toggleWidget`):
//! a process of its own, started when asked for or when an agent asks for
//! permission while the window is in the background, and what it asks back.

use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use serde_json::Value;

use super::App;
use crate::ui::{Ui, WindowRequest};
use crate::widget::{parse, Request};

/// The running widget and the lines it writes.
pub(super) struct Running {
    child: Child,
    lines: Receiver<Request>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    /// The widget is shown when the app is not, if Settings › General allows it.
    fn widget_enabled(&self) -> bool {
        self.store
            .config
            .pointer("/defaults/widgetEnabled")
            .and_then(Value::as_bool)
            == Some(true)
    }

    /// Mod+Shift+W in the desktop's Window menu, here the palette's.
    pub(super) fn toggle_widget(&mut self) {
        if self.widget.take().is_none() {
            self.start_widget();
        }
    }

    fn start_widget(&mut self) {
        if self.widget.is_some() || !self.windowed {
            return;
        }
        let (Ok(exe), Some(dir)) = (std::env::current_exe(), self.data_dir.clone()) else {
            return;
        };
        let mut cmd = vorn_spawn::command(exe);
        cmd.arg("widget")
            .arg("--data-dir")
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return;
        };
        let (tx, lines) = channel();
        if let Some(out) = child.stdout.take() {
            let wake = Arc::clone(&self.wake);
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(r) = parse(&line) {
                        if tx.send(r).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            });
        }
        self.widget = Some(Running { child, lines });
    }

    /// An agent asked for permission: the widget shows if the window is behind.
    pub(super) fn widget_asked(&mut self, ui: &Ui) {
        if self.widget_enabled() && !ui.window_focused() {
            self.start_widget();
        }
    }

    /// Once a frame: what the widget asked, and whether it is still there.
    pub(super) fn widget_tick(&mut self, ui: &mut Ui) {
        let Some(w) = &mut self.widget else {
            return;
        };
        let asked: Vec<Request> = w.lines.try_iter().collect();
        let gone = matches!(w.child.try_wait(), Ok(Some(_)));
        if gone {
            self.widget = None;
        }
        if !self.widget_enabled() && self.widget.is_some() && asked.is_empty() {
            // Turning the setting off hides it, as the desktop did.
            if self.widget_was_enabled {
                self.widget = None;
            }
        }
        self.widget_was_enabled = self.widget_enabled();
        for r in asked {
            match r {
                Request::Focus(id) => {
                    ui.request(WindowRequest::Focus);
                    if self.store.session(&id).is_some() {
                        self.st.view = crate::screen::MainView::Sessions;
                        self.st.selected = Some(id);
                    }
                }
                Request::Show => ui.request(WindowRequest::Focus),
                Request::Hide => self.widget = None,
            }
        }
    }
}
