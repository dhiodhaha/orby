//! Ctrl+Option+Space through Carbon's RegisterEventHotKey: a system-wide shortcut that needs no
//! Accessibility permission (unlike a key event monitor). The handler runs on the main thread
//! from the app's run loop.

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
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const CONTROL_KEY: u32 = 1 << 12;
const OPTION_KEY: u32 = 1 << 11;
const KEY_SPACE: u32 = 49;

static ON_PRESS: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

extern "C" fn on_event(_call: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    // Only our one hot key is registered, so any press is it.
    if let Some(f) = ON_PRESS.get() {
        f();
    }
    0
}

/// Registers Ctrl+Option+Space; `f` runs on each press. Returns false if the shortcut is taken.
pub fn register(f: fn()) -> bool {
    let _ = ON_PRESS.set(f);
    unsafe {
        let target = GetApplicationEventTarget();
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        let mut handler = std::ptr::null_mut();
        if InstallEventHandler(
            target,
            on_event,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        ) != 0
        {
            return false;
        }
        let id = EventHotKeyID {
            signature: u32::from_be_bytes(*b"orby"),
            id: 1,
        };
        let mut hk = std::ptr::null_mut();
        RegisterEventHotKey(KEY_SPACE, CONTROL_KEY | OPTION_KEY, id, target, 0, &mut hk) == 0
    }
}
