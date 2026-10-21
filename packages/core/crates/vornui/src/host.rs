//! What a host runs an app through, on a desktop (`window`) or in a
//! browser (`web`): the [`App`] trait, the waker, the window's options and
//! one frame's drawing.

use std::sync::Arc;
use web_time::Instant;

use accesskit::ActionRequest;
use winit::event_loop::EventLoopProxy;
use winit::window::{CursorIcon, Window};

use crate::element::{Cursor, El};
use crate::input::Input;
use crate::interact::Event;
use crate::scene::{Rect, Rgba};
use crate::ui::{Laid, Ui};

/// What was dropped on the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// Files and folders on this machine.
    Files(Vec<std::path::PathBuf>),
    /// An image with no file of its own (dragged from a web page, or pasted
    /// in a browser): its bytes, and a file name that says its type.
    Image { name: String, bytes: Vec<u8> },
}

/// An app drawn by vornui.
pub trait App: 'static {
    /// The frame's element tree, built from the app's state and `ui`'s.
    fn view(&mut self, ui: &mut Ui) -> El;
    /// Paints the app's custom boxes after layout (into the window's layer,
    /// under menus and tooltips).
    fn paint(&mut self, _ui: &mut Ui, _laid: &Laid) {}
    /// Something the UI reported: a click, a choice, an edit.
    fn event(&mut self, _ui: &mut Ui, _event: Event) {}
    /// Input the UI did not use (a terminal takes what a field did not).
    fn input(&mut self, _ui: &mut Ui, _input: Input) {}
    /// Input before the UI sees it: true keeps it from the UI (global
    /// shortcuts, which win over a focused field).
    fn intercept(&mut self, _ui: &mut Ui, _input: &Input) -> bool {
        false
    }
    /// A screen-reader request for a node the UI does not own.
    fn access(&mut self, _ui: &mut Ui, _req: &ActionRequest) {}
    /// Text the page pasted (a browser hands it over in its `paste` event,
    /// not through the clipboard); it is on the clipboard already. The
    /// default presses the platform's paste chord.
    fn paste(&mut self, ui: &mut Ui, _text: String) {
        let input = Input::Key {
            code: "KeyV".into(),
            mods: crate::input::mods::primary(),
            text: None,
        };
        if !self.intercept(ui, &input) && !ui.key(&input) {
            self.input(ui, input);
        }
    }
    /// Something dropped on the window at `at` (logical pixels), where the
    /// platform says where; a browser's pasted image comes here too, with
    /// no place.
    fn dropped(&mut self, _ui: &mut Ui, _what: Dropped, _at: Option<(f32, f32)>) {}
    /// Whether the last frame showed what the last keystroke was waiting
    /// for. Apps whose input echoes asynchronously answer it; others are
    /// done once the key itself is drawn.
    fn echoed(&mut self) -> bool {
        true
    }
    /// Whether the app wants IME input (a terminal does).
    fn wants_ime(&self) -> bool {
        false
    }
    /// Where the app's text cursor is (logical pixels), for the IME.
    fn ime_area(&self) -> Option<Rect> {
        None
    }
    fn background(&self) -> Rgba {
        crate::theme::color::SURFACE_BASE.rgba()
    }
    /// A press landed on the window's own surface, not on a native child view:
    /// the time to take the keyboard back from such a view (WebView2 keeps it).
    fn pressed(&mut self) {}
    /// The window exists, before it is first shown: for an app that seats
    /// native views (a web view) in it.
    fn opened(&mut self, _window: &Arc<Window>) {}
    /// Runs each time the loop is about to sleep, for work a native view's
    /// toolkit needs pumped; returns when it wants to run again.
    fn tick(&mut self) -> Option<Instant> {
        None
    }
}

pub enum UserEvent {
    #[cfg(not(target_arch = "wasm32"))]
    Access(accesskit_winit::Event),
    Wake,
}

#[cfg(not(target_arch = "wasm32"))]
impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> UserEvent {
        UserEvent::Access(e)
    }
}

/// Asks the window for a frame from any thread (a terminal's reader).
#[derive(Clone)]
pub struct Waker(EventLoopProxy<UserEvent>);

impl Waker {
    pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Waker {
        Waker(proxy)
    }

    pub fn wake(&self) {
        // Only fails once the loop has exited, when no frame is wanted.
        let _ = self.0.send_event(UserEvent::Wake);
    }
}

/// How the window opens.
#[derive(Debug, Clone, Copy)]
pub struct WindowOptions {
    /// The first size, logical pixels.
    pub size: (f32, f32),
    pub min_size: Option<(f32, f32)>,
    /// The system's title bar and frame; without them the app draws its own.
    pub decorations: bool,
    /// macOS: no title bar drawn, the traffic lights at this logical point
    /// over the app's own top bar; the title still names the window for the
    /// Dock and the window switcher.
    pub inset_title: Option<(f32, f32)>,
    pub maximized: bool,
    /// Floats over every other window.
    pub always_on_top: bool,
    /// Kept out of the taskbar and the window switcher (Windows).
    pub skip_taskbar: bool,
    /// Clears to the app's background alpha where the surface allows it.
    pub transparent: bool,
    /// Opens this far (logical pixels) from the screen's bottom right
    /// corner rather than where the system places it.
    pub corner: Option<(f32, f32)>,
    /// Settings › Appearance › Materials was not Solid at launch: on Windows
    /// the renderer presents through DirectComposition so Mica can show
    /// through, which only a new launch can change.
    pub os_material: bool,
}

impl WindowOptions {
    pub fn sized(size: (f32, f32)) -> WindowOptions {
        WindowOptions {
            size,
            min_size: None,
            decorations: true,
            inset_title: None,
            maximized: false,
            always_on_top: false,
            skip_taskbar: false,
            transparent: false,
            corner: None,
            os_material: false,
        }
    }
}

pub(crate) fn cursor_icon(c: Cursor) -> CursorIcon {
    match c {
        Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
    }
}

/// Builds, lays out, paints and draws one frame of `app` at `px` physical
/// pixels into `target` (ignored by the CPU renderer).
pub(crate) fn draw<A: App>(
    ui: &mut Ui,
    app: &mut A,
    now: Instant,
    px: (u32, u32),
    target: Option<&wgpu::TextureView>,
) -> Laid {
    let k = ui.scale();
    let size = (px.0 as f32 / k, px.1 as f32 / k);
    // A full atlas empties itself and asks for the frame again; twice
    // means one frame's glyphs do not fit, and it is drawn as is.
    let mut tries = 0;
    ui.viewport = size;
    loop {
        ui.begin_at(app.background(), now);
        let root = app.view(ui);
        let laid = ui.layout(root, size);
        app.paint(ui, &laid);
        tries += 1;
        if ui.render_target(target, px) || tries == 2 {
            return laid;
        }
    }
}
