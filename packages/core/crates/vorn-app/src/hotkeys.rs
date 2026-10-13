//! The approve and deny keys held system-wide (`globalShortcut` in the
//! desktop's main process): Mod+Shift+A and Mod+Shift+D answer the oldest
//! permission request even while another app is in front. macOS registers
//! them with Carbon, Windows with `RegisterHotKey`, X11 with a key grab on
//! the root window; where none of these exists (Wayland) the keys work only
//! while the window has focus.

use std::sync::mpsc::{channel, Receiver, Sender};

use crate::client::rpc::Wake;

/// Presses of the two keys: `true` approves, `false` denies.
pub type Presses = Receiver<bool>;

/// Holds the keys for the life of the app; `None` when this system cannot.
pub fn register(wake: Wake) -> Option<Presses> {
    let (tx, rx) = channel();
    platform::register(tx, wake).then_some(rx)
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn press(tx: &Sender<bool>, wake: &Wake, allow: bool) {
    if tx.send(allow).is_ok() {
        wake();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::sync::{Mutex, OnceLock, PoisonError};

    use super::{press, Sender, Wake};

    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct HotKeyId {
        signature: u32,
        id: u32,
    }

    type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(
            target: *mut c_void,
            handler: Handler,
            count: u32,
            list: *const EventTypeSpec,
            user: *mut c_void,
            out: *mut *mut c_void,
        ) -> i32;
        fn RegisterEventHotKey(
            code: u32,
            modifiers: u32,
            id: HotKeyId,
            target: *mut c_void,
            options: u32,
            out: *mut *mut c_void,
        ) -> i32;
        fn GetEventParameter(
            event: *mut c_void,
            name: u32,
            desired: u32,
            actual: *mut u32,
            size: usize,
            actual_size: *mut usize,
            data: *mut c_void,
        ) -> i32;
    }

    const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
    const HOT_KEY_PRESSED: u32 = 5;
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    const HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
    const SIGNATURE: u32 = u32::from_be_bytes(*b"Vorn");
    const CMD_SHIFT: u32 = 0x0100 | 0x0200;
    /// `kVK_ANSI_A` and `kVK_ANSI_D`.
    const KEYS: [(u32, u32); 2] = [(0x00, 1), (0x02, 2)];

    static TARGET: OnceLock<Mutex<(Sender<bool>, Wake)>> = OnceLock::new();

    extern "C" fn handle(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
        let mut id = HotKeyId {
            signature: 0,
            id: 0,
        };
        // SAFETY: Carbon hands a live hot-key event; `id` is a HotKeyId-sized buffer.
        let status = unsafe {
            GetEventParameter(
                event,
                DIRECT_OBJECT,
                HOT_KEY_ID,
                std::ptr::null_mut(),
                std::mem::size_of::<HotKeyId>(),
                std::ptr::null_mut(),
                (&raw mut id).cast(),
            )
        };
        if status == 0 && id.signature == SIGNATURE {
            if let Some(t) = TARGET.get() {
                let t = t.lock().unwrap_or_else(PoisonError::into_inner);
                press(&t.0, &t.1, id.id == 1);
            }
        }
        0
    }

    /// Runs on the main thread, where the app's event loop is.
    pub fn register(tx: Sender<bool>, wake: Wake) -> bool {
        if TARGET.set(Mutex::new((tx, wake))).is_err() {
            return false;
        }
        let spec = EventTypeSpec {
            class: KEYBOARD,
            kind: HOT_KEY_PRESSED,
        };
        // SAFETY: plain Carbon calls with valid pointers; the handler is a static function.
        unsafe {
            let target = GetApplicationEventTarget();
            let mut handler = std::ptr::null_mut();
            if InstallEventHandler(target, handle, 1, &spec, std::ptr::null_mut(), &mut handler)
                != 0
            {
                return false;
            }
            let mut ok = true;
            for (code, id) in KEYS {
                let mut out = std::ptr::null_mut();
                let key = HotKeyId {
                    signature: SIGNATURE,
                    id,
                };
                ok &= RegisterEventHotKey(code, CMD_SHIFT, key, target, 0, &mut out) == 0;
            }
            ok
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    use super::{press, Sender, Wake};

    /// A thread of its own holds the keys and waits on its message queue.
    pub fn register(tx: Sender<bool>, wake: Wake) -> bool {
        let (ready, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mods = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
            // SAFETY: a null window ties the keys to this thread's queue.
            let ok = unsafe {
                RegisterHotKey(std::ptr::null_mut(), 1, mods, u32::from(b'A')) != 0
                    && RegisterHotKey(std::ptr::null_mut(), 2, mods, u32::from(b'D')) != 0
            };
            let _ = ready.send(ok);
            if !ok {
                return;
            }
            // SAFETY: MSG is plain data; GetMessageW fills it.
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            // SAFETY: `msg` is a valid MSG this thread owns.
            while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                if msg.message == WM_HOTKEY {
                    press(&tx, &wake, msg.wParam == 1);
                }
            }
        });
        answer.recv().unwrap_or(false)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    use x11rb::protocol::Event;

    use super::{press, Sender, Wake};

    /// Grabs the keys on the root window, with and without Caps and Num Lock.
    pub fn register(tx: Sender<bool>, wake: Wake) -> bool {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return false;
        }
        let Ok((conn, screen)) = x11rb::connect(None) else {
            return false;
        };
        let root = conn.setup().roots[screen].root;
        let Some(codes) = keycodes(&conn) else {
            return false;
        };
        let base = ModMask::CONTROL | ModMask::SHIFT;
        let mut ok = true;
        for code in codes {
            for extra in [
                ModMask::from(0u16),
                ModMask::LOCK,
                ModMask::M2,
                ModMask::LOCK | ModMask::M2,
            ] {
                ok &= conn
                    .grab_key(
                        true,
                        root,
                        base | extra,
                        code,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )
                    .ok()
                    .and_then(|c| c.check().ok())
                    .is_some();
            }
        }
        if !ok || conn.flush().is_err() {
            return false;
        }
        std::thread::spawn(move || {
            while let Ok(event) = conn.wait_for_event() {
                if let Event::KeyPress(k) = event {
                    press(&tx, &wake, Some(k.detail) == codes.first().copied());
                }
            }
        });
        true
    }

    /// The keycodes of A and D on this keyboard.
    fn keycodes(conn: &impl Connection) -> Option<[u8; 2]> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .ok()?
            .reply()
            .ok()?;
        let per = usize::from(map.keysyms_per_keycode);
        let find = |sym: u32| {
            map.keysyms
                .chunks(per.max(1))
                .position(|c| c.contains(&sym))
                .and_then(|i| u8::try_from(i).ok())
                .map(|i| min + i)
        };
        Some([find(u32::from(b'a'))?, find(u32::from(b'd'))?])
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    use super::{Sender, Wake};

    pub fn register(_: Sender<bool>, _: Wake) -> bool {
        false
    }
}
