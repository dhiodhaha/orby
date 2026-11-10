//! The frameless, resizable top-level windows (Disk and Tasks): the window style, the resize
//! edges, maximize and restore, and the rounded region. Windows does the moving, sizing, snapping
//! and maximizing itself (`WS_THICKFRAME | WS_MAXIMIZEBOX`); this file only keeps the system frame
//! from ever showing (`WM_NCCALCSIZE` gives the whole window to the client) and keeps a maximized
//! window inside the monitor's work area, so it never covers the taskbar.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Taskbar, Alt+Tab, snapping and Win+Up / Win+Down, with no caption or border drawn.
pub const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_POPUP.0 | WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0);

/// The band along each edge that resizes the window (logical px).
const EDGE: f32 = 6.0;
/// How far a corner's diagonal resize reaches along each edge (logical px): the window's 16 px
/// corner radius, so the whole rounded corner takes the diagonal.
const CORNER: f32 = 16.0;

pub fn is_max(hwnd: HWND) -> bool {
    unsafe { IsZoomed(hwnd).as_bool() }
}

/// Maximize, or restore a maximized window. Posted, so the WM_SIZE it causes arrives after the
/// click that asked for it has let go of the App.
pub fn toggle(hwnd: HWND) {
    let cmd = if is_max(hwnd) {
        SC_RESTORE
    } else {
        SC_MAXIMIZE
    };
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
    }
}

/// The work area of the monitor the window is on (or nearest to).
fn work_area_of(hwnd: HWND) -> RECT {
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

/// `WM_NCCALCSIZE`: no non-client area at all. A maximized window keeps to the work area even if
/// Windows placed it a frame's width past the monitor's edges.
fn on_nccalcsize(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if wparam.0 != 0 && is_max(hwnd) {
        let p = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
        let wa = work_area_of(hwnd);
        let r = &mut p.rgrc[0];
        r.left = r.left.max(wa.left);
        r.top = r.top.max(wa.top);
        r.right = r.right.min(wa.right);
        r.bottom = r.bottom.min(wa.bottom);
    }
    LRESULT(0)
}

/// `WM_GETMINMAXINFO`: the least size (logical px, at the window's DPI), and a maximized window
/// exactly on the work area of its monitor. It needs no App: it also arrives while the App is
/// busy moving the window.
fn on_minmax(hwnd: HWND, lparam: LPARAM, min_w: f32, min_h: f32) -> LRESULT {
    let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
    let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let (wa, m) = (mi.rcWork, mi.rcMonitor);
            mmi.ptMaxPosition = POINT {
                x: wa.left - m.left,
                y: wa.top - m.top,
            };
            mmi.ptMaxSize = POINT {
                x: wa.right - wa.left,
                y: wa.bottom - wa.top,
            };
        }
    }
    mmi.ptMinTrackSize = POINT {
        x: (min_w * scale).ceil() as i32,
        y: (min_h * scale).ceil() as i32,
    };
    LRESULT(0)
}

/// The frame's own messages: no non-client area, the size limits, and no caption painting on
/// activation. None for every other message.
pub fn nc_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    min_w: f32,
    min_h: f32,
) -> Option<LRESULT> {
    match msg {
        WM_NCCALCSIZE => Some(on_nccalcsize(hwnd, wparam, lparam)),
        WM_GETMINMAXINFO => Some(on_minmax(hwnd, lparam, min_w, min_h)),
        // -1: the default handling keeps the activation state without repainting a frame.
        WM_NCACTIVATE => Some(unsafe { DefWindowProcW(hwnd, msg, wparam, LPARAM(-1)) }),
        _ => None,
    }
}

/// The resize edge under a point in client pixels, or None (never while maximized).
pub fn edge_hit(hwnd: HWND, p: POINT, scale: f32) -> Option<u32> {
    if is_max(hwnd) {
        return None;
    }
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    edge_at(p.x, p.y, rc.right, rc.bottom, scale)
}

/// Which edge or corner (x, y) is on in a w × h window (device px).
fn edge_at(x: i32, y: i32, w: i32, h: i32, scale: f32) -> Option<u32> {
    let e = (EDGE * scale).round() as i32;
    let c = (CORNER * scale).round() as i32;
    let (left, right, top, bottom) = (x < e, x >= w - e, y < e, y >= h - e);
    let (near_l, near_r, near_t, near_b) = (x < c, x >= w - c, y < c, y >= h - c);
    Some(if (top && near_l) || (left && near_t) {
        HTTOPLEFT
    } else if (top && near_r) || (right && near_t) {
        HTTOPRIGHT
    } else if (bottom && near_l) || (left && near_b) {
        HTBOTTOMLEFT
    } else if (bottom && near_r) || (right && near_b) {
        HTBOTTOMRIGHT
    } else if left {
        HTLEFT
    } else if right {
        HTRIGHT
    } else if top {
        HTTOP
    } else if bottom {
        HTBOTTOM
    } else {
        return None;
    })
}

/// The window's shape after a size change: square corners while maximized, the 16 px rounded
/// corners otherwise.
pub fn shape(hwnd: HWND, scale: f32) {
    unsafe {
        if is_max(hwnd) {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let d = (32.0 * scale).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d);
        let _ = SetWindowRgn(hwnd, Some(rgn), true);
    }
}

/// The client size in device pixels.
pub fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners_resize_and_the_middle_does_not() {
        let (w, h) = (756, 704);
        assert_eq!(edge_at(0, 300, w, h, 1.0), Some(HTLEFT));
        assert_eq!(edge_at(755, 300, w, h, 1.0), Some(HTRIGHT));
        assert_eq!(edge_at(300, 2, w, h, 1.0), Some(HTTOP));
        assert_eq!(edge_at(300, 703, w, h, 1.0), Some(HTBOTTOM));
        assert_eq!(edge_at(2, 2, w, h, 1.0), Some(HTTOPLEFT));
        assert_eq!(
            edge_at(10, 1, w, h, 1.0),
            Some(HTTOPLEFT),
            "along the corner"
        );
        assert_eq!(edge_at(754, 700, w, h, 1.0), Some(HTBOTTOMRIGHT));
        assert_eq!(edge_at(1, 690, w, h, 1.0), Some(HTBOTTOMLEFT));
        assert_eq!(edge_at(300, 300, w, h, 1.0), None);
        assert_eq!(edge_at(6, 300, w, h, 1.0), None, "6 px band");
        assert_eq!(edge_at(8, 300, w, h, 1.5), Some(HTLEFT), "scaled by DPI");
    }
}
