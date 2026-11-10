//! The global shortcut through Carbon's RegisterEventHotKey: a system-wide shortcut that needs no
//! Accessibility permission (unlike a key event monitor). The handler runs on the main thread
//! from the app's run loop. Settings changes the combination (`register` again, `unregister`
//! while a new one is being pressed, `probe` for a free one); the default is Ctrl+Option+Space.
//!
//! Keys are the shortcut's Windows virtual keys (settings.json keeps them the same on both
//! systems) mapped to the Mac's key codes, which name a key by its place on an ANSI keyboard.

use std::cell::Cell;
use std::ffi::c_void;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyID {
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
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// Carbon's modifier bits.
pub const CMD_KEY: u32 = 1 << 8;
pub const SHIFT_KEY: u32 = 1 << 9;
pub const OPTION_KEY: u32 = 1 << 11;
pub const CONTROL_KEY: u32 = 1 << 12;

static ON_PRESS: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

thread_local! {
    /// The registered shortcut (EventHotKeyRef), null when none.
    static HOT: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
}

extern "C" fn on_event(_call: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    // Only our one hot key stays registered (a probe lets go at once), so any press is it.
    if let Some(f) = ON_PRESS.get() {
        f();
    }
    0
}

/// Installs the press handler once; `f` runs on each press. Returns false if Carbon refused.
pub fn install(f: fn()) -> bool {
    if ON_PRESS.set(f).is_err() {
        return true;
    }
    unsafe {
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        let mut handler = std::ptr::null_mut();
        InstallEventHandler(
            GetApplicationEventTarget(),
            on_event,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        ) == 0
    }
}

fn register_id(code: u32, mods: u32, id: u32) -> Option<*mut c_void> {
    let id = EventHotKeyID {
        signature: u32::from_be_bytes(*b"orby"),
        id,
    };
    let mut hk = std::ptr::null_mut();
    let ok = unsafe {
        RegisterEventHotKey(code, mods, id, GetApplicationEventTarget(), 0, &mut hk) == 0
    };
    (ok && !hk.is_null()).then_some(hk)
}

/// Registers the shortcut (`code` a Mac key code, `mods` Carbon bits), replacing the one held.
/// Returns false if it is taken.
pub fn register(code: u32, mods: u32) -> bool {
    unregister();
    match register_id(code, mods, 1) {
        Some(hk) => {
            HOT.with(|h| h.set(hk));
            true
        }
        None => false,
    }
}

/// Lets the shortcut go (while a new one is pressed in Settings).
pub fn unregister() {
    let hk = HOT.with(|h| h.replace(std::ptr::null_mut()));
    if !hk.is_null() {
        unsafe {
            UnregisterEventHotKey(hk);
        }
    }
}

/// Whether the combination could be registered now: registered under a spare id and let go.
pub fn probe(code: u32, mods: u32) -> bool {
    match register_id(code, mods, 2) {
        Some(hk) => {
            unsafe {
                UnregisterEventHotKey(hk);
            }
            true
        }
        None => false,
    }
}

/// (Windows virtual key, Mac key code) for every key a shortcut may end in (settings_model's
/// `key_name`), plus the editing keys the capture sees. ANSI positions.
const KEYS: &[(u16, u16)] = &[
    (0x41, 0),
    (0x53, 1),
    (0x44, 2),
    (0x46, 3),
    (0x48, 4),
    (0x47, 5),
    (0x5A, 6),
    (0x58, 7),
    (0x43, 8),
    (0x56, 9),
    (0x42, 11),
    (0x51, 12),
    (0x57, 13),
    (0x45, 14),
    (0x52, 15),
    (0x59, 16),
    (0x54, 17),
    (0x31, 18),
    (0x32, 19),
    (0x33, 20),
    (0x34, 21),
    (0x36, 22),
    (0x35, 23),
    (0xBB, 24), // =
    (0x39, 25),
    (0x37, 26),
    (0xBD, 27), // -
    (0x38, 28),
    (0x30, 29),
    (0xDD, 30), // ]
    (0x4F, 31),
    (0x55, 32),
    (0xDB, 33), // [
    (0x49, 34),
    (0x50, 35),
    (0x0D, 36), // Return
    (0x4C, 37),
    (0x4A, 38),
    (0xDE, 39), // '
    (0x4B, 40),
    (0xBA, 41), // ;
    (0xDC, 42), // \
    (0xBC, 43), // ,
    (0xBF, 44), // /
    (0x4E, 45),
    (0x4D, 46),
    (0xBE, 47), // .
    (0x09, 48), // Tab
    (0x20, 49), // Space
    (0xC0, 50), // `
    (0x08, 51), // Delete (Backspace)
    (0x1B, 53), // Esc
    (0x70, 122),
    (0x71, 120),
    (0x72, 99),
    (0x73, 118),
    (0x74, 96),
    (0x75, 97),
    (0x76, 98),
    (0x77, 100),
    (0x78, 101),
    (0x79, 109),
    (0x7A, 103),
    (0x7B, 111),
    (0x7C, 105),
    (0x7D, 107),
    (0x7E, 113),
    (0x2D, 114), // Help, where PC keyboards have Insert
    (0x24, 115), // Home
    (0x21, 116), // Page Up
    (0x2E, 117), // forward Delete
    (0x23, 119), // End
    (0x22, 121), // Page Down
    (0x25, 123),
    (0x27, 124),
    (0x28, 125),
    (0x26, 126),
];

/// The Mac key code for a Windows virtual key.
pub fn code_of_vk(vk: u32) -> Option<u32> {
    KEYS.iter()
        .find(|(v, _)| *v as u32 == vk)
        .map(|(_, c)| *c as u32)
}

/// The Windows virtual key for a Mac key code.
pub fn vk_of_code(code: u16) -> Option<u16> {
    KEYS.iter().find(|(_, c)| *c == code).map(|(v, _)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_both_ways() {
        assert_eq!(code_of_vk(0x20), Some(49));
        assert_eq!(code_of_vk(0x4B), Some(40));
        assert_eq!(vk_of_code(40), Some(0x4B));
        assert_eq!(vk_of_code(122), Some(0x70));
        for &(v, c) in KEYS {
            assert_eq!(vk_of_code(c), Some(v));
            assert_eq!(code_of_vk(v as u32), Some(c as u32));
        }
    }
}
