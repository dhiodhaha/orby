//! Low-level mouse hook: proximity and the dial, without polling.
//!
//! The hook runs on this thread's message loop, so plain statics are enough; it only posts
//! messages back to the orb window and never touches the App.

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::orb::{orb_center, NEAR, ORB};
use crate::{px, App, WM_APP_DIALMOVE, WM_APP_NEAR, WM_APP_OUTSIDE};

static mut HOOK_HWND: HWND = HWND(std::ptr::null_mut());
static mut HOOK_NEAR: bool = false;
static mut HOOK_DIAL: bool = false;
static mut HOOK_CENTER: (f32, f32, f32) = (0.0, 0.0, 0.0);
static mut HOOK_KEEP: [RECT; 5] = [RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
}; 5]; // orb, dial, pill, panel, menu

fn pack(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u32 & 0xffff) << 16) | (x as u32 & 0xffff)) as isize)
}

pub fn unpack(l: LPARAM) -> (f32, f32) {
    (
        (l.0 & 0xffff) as u16 as i16 as f32,
        ((l.0 >> 16) & 0xffff) as u16 as i16 as f32,
    )
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let m = wparam.0 as u32;
        if m == WM_MOUSEMOVE {
            let (cx, cy, r) = HOOK_CENTER;
            let dx = info.pt.x as f32 - cx;
            let dy = info.pt.y as f32 - cy;
            let near = (dx * dx + dy * dy).sqrt() < r;
            if near != HOOK_NEAR || near {
                HOOK_NEAR = near;
                let _ = PostMessageW(
                    Some(HOOK_HWND),
                    WM_APP_NEAR,
                    WPARAM(near as usize),
                    pack(info.pt.x, info.pt.y),
                );
            }
            if HOOK_DIAL {
                let _ = PostMessageW(
                    Some(HOOK_HWND),
                    WM_APP_DIALMOVE,
                    WPARAM(0),
                    pack(info.pt.x, info.pt.y),
                );
            }
        } else if HOOK_DIAL && (m == WM_LBUTTONDOWN || m == WM_RBUTTONDOWN) {
            let keep = HOOK_KEEP;
            let inside = keep.iter().any(|r| {
                info.pt.x >= r.left
                    && info.pt.x < r.right
                    && info.pt.y >= r.top
                    && info.pt.y < r.bottom
            });
            if !inside {
                let _ = PostMessageW(Some(HOOK_HWND), WM_APP_OUTSIDE, WPARAM(0), LPARAM(0));
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn window_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

/// A window rect cut to the window's region: mid-switch the panel window is larger than the shape
/// on screen, and a click beside that shape is outside.
fn shaped_rect(h: HWND) -> RECT {
    let r = window_rect(h);
    let mut b = RECT::default();
    // SIMPLEREGION or COMPLEXREGION; no region (or an error) keeps the whole window.
    if unsafe { windows::Win32::Graphics::Gdi::GetWindowRgnBox(h, &mut b) }.0 < 2 {
        return r;
    }
    RECT {
        left: r.left + b.left,
        top: r.top + b.top,
        right: (r.left + b.right).min(r.right),
        bottom: (r.top + b.bottom).min(r.bottom),
    }
}

/// Keeps the hook's copy of the drop centre and our window rects current.
pub fn update_hook(app: &App) {
    let (cx, cy) = orb_center(app);
    unsafe {
        HOOK_CENTER = (cx, cy, px(app, ORB / 2.0 + NEAR + 40.0));
        HOOK_KEEP = [
            window_rect(app.hwnd),
            window_rect(app.dial_hwnd),
            window_rect(app.pill_hwnd),
            shaped_rect(app.panel_hwnd),
            window_rect(app.dl.menu_hwnd()),
        ];
    }
}

/// While the dial, a panel or the format menu is open, the hook also reports pointer moves (for aiming) and
/// clicks outside our windows (to close).
pub fn set_watch_clicks(on: bool) {
    unsafe {
        HOOK_DIAL = on;
    }
}

pub fn install(hwnd: HWND, hinst: HINSTANCE) -> windows::core::Result<HHOOK> {
    unsafe {
        HOOK_HWND = hwnd;
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinst), 0)
    }
}
