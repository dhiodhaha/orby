#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! Edges and depth (frame "v6.3 · 01 Foundation"): depth comes from the edge, not a bigger shadow.
//!
//! - **Lip:** one pixel on the side that faces away from the light. White: a bottom line, black
//!   at 4%. Dark: a top line, white at 6%. Drawn inside the shape, following its corners, on
//!   panels, cards and controls (`lip`).
//! - **Tucked shadow:** dark floating surfaces get `0 16px 40px -18px rgba(0,0,0,.55)`, so the
//!   shadow sits under the surface instead of spreading round it. White keeps the soft
//!   `0 4px 42px rgba(0,0,0,.07), 0 2px 6px rgba(0,0,0,.05)` (the frame's white stage).
//!
//! Every floating window clips itself to its rounded shape with a window region, so a shadow
//! cannot be drawn inside it. `follow` keeps one click-through window per surface right under
//! it in z-order and draws the shadow there. The shadow is drawn once per size or theme and only
//! moved after that, so an idle surface costs nothing.

use std::cell::RefCell;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{brush, rgba, rrect, Shared, Surface};
use crate::ui::Theme;

/// One CSS box shadow, in logical px: `0 y blur spread color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub y: f32,
    /// The CSS blur radius; the Gaussian's sigma is half of it.
    pub blur: f32,
    /// Negative pulls the shadow under the surface. D2D has no spread, so the source shape is
    /// grown (or shrunk) by this much before it is blurred.
    pub spread: f32,
    pub alpha: f32,
}

/// White floating surfaces: the soft pair from the frame's white stage.
pub const SHADOW_LIGHT: [BoxShadow; 2] = [
    BoxShadow {
        y: 4.0,
        blur: 42.0,
        spread: 0.0,
        alpha: 0.07,
    },
    BoxShadow {
        y: 2.0,
        blur: 6.0,
        spread: 0.0,
        alpha: 0.05,
    },
];

/// Dark floating surfaces: tucked, `0 16px 40px -18px rgba(0,0,0,.55)`.
pub const SHADOW_DARK: [BoxShadow; 1] = [BoxShadow {
    y: 16.0,
    blur: 40.0,
    spread: -18.0,
    alpha: 0.55,
}];

pub fn shadows(dark: bool) -> &'static [BoxShadow] {
    if dark {
        &SHADOW_DARK
    } else {
        &SHADOW_LIGHT
    }
}

/// How far (logical px) the shadows reach past the shape on any side.
pub fn reach(dark: bool) -> f32 {
    shadows(dark)
        .iter()
        .map(|s| s.blur + s.y.abs() + s.spread)
        .fold(0.0, f32::max)
        .ceil()
}

/// The lip's colour: white, black 4% at the bottom; dark, white 6% at the top.
pub fn lip_color(t: &Theme) -> D2D1_COLOR_F {
    if t.dark {
        rgba(0xFFFFFF, 0.06)
    } else {
        rgba(0x000000, 0.04)
    }
}

/// The T3 lip on a rounded rect: a one-pixel crescent along the inside of the bottom edge
/// (white) or the top edge (dark), following the corners, like CSS `inset 0 -1px 0` /
/// `inset 0 1px 0`. `a` scales its alpha (fades, pressed states pass 0).
pub fn lip(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    l: f32,
    top: f32,
    r: f32,
    b: f32,
    radius: f32,
    a: f32,
) {
    if a <= 0.01 || r - l < 2.0 || b - top < 2.0 {
        return;
    }
    let dy = if t.dark { 1.0 } else { -1.0 };
    unsafe {
        let shape = rrect(l, top, r, b, radius);
        let moved = rrect(l, top + dy, r, b + dy, radius);
        let (Ok(g0), Ok(g1)) = (
            g.d2f.CreateRoundedRectangleGeometry(&shape),
            g.d2f.CreateRoundedRectangleGeometry(&moved),
        ) else {
            return;
        };
        let Ok(path) = g.d2f.CreatePathGeometry() else {
            return;
        };
        let Ok(sink) = path.Open() else { return };
        let ok = g0
            .CombineWithGeometry(&g1, D2D1_COMBINE_MODE_EXCLUDE, None, 0.05, &sink)
            .is_ok();
        let _ = sink.Close();
        if ok {
            let c = lip_color(t);
            dc.FillGeometry(&path, &brush(dc, D2D1_COLOR_F { a: c.a * a, ..c }), None);
        }
    }
}

/// The shape of one shadow, grown by its spread, recorded into a command list and run through
/// D2D's Shadow effect (sigma = blur / 2). Records on its own, so call it before `BeginDraw` of
/// the target it will be drawn into. Draw the result `s.y` lower.
unsafe fn shadow_image(
    g: &Shared,
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    radius: f32,
    s: &BoxShadow,
) -> windows::core::Result<Option<ID2D1Image>> {
    let dc = &g.dc;
    let sp = s.spread;
    let (l, t, r, b) = (l - sp, t - sp, r + sp, b + sp);
    if r - l <= 0.5 || b - t <= 0.5 {
        return Ok(None);
    }
    let rad = (radius + sp).max(0.0).min((r - l) / 2.0).min((b - t) / 2.0);
    let list = dc.CreateCommandList()?;
    dc.SetTarget(&list);
    dc.BeginDraw();
    dc.FillRoundedRectangle(&rrect(l, t, r, b, rad), &brush(dc, rgba(0, 1.0)));
    let drawn = dc.EndDraw(None, None);
    dc.SetTarget(None);
    drawn?;
    list.Close()?;
    let fx = dc.CreateEffect(&CLSID_D2D1Shadow)?;
    fx.SetInput(0, &list, true);
    let sigma = (s.blur / 2.0).max(0.0);
    fx.SetValue(
        D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
        D2D1_PROPERTY_TYPE_FLOAT,
        &sigma.to_le_bytes(),
    )?;
    let mut col = [0u8; 16];
    for (i, v) in [0.0f32, 0.0, 0.0, s.alpha].iter().enumerate() {
        col[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    fx.SetValue(
        D2D1_SHADOW_PROP_COLOR.0 as u32,
        D2D1_PROPERTY_TYPE_VECTOR4,
        &col,
    )?;
    Ok(Some(fx.GetOutput()?))
}

// ---- shadow windows for floating surfaces ----

struct ShadowWin {
    owner: HWND,
    hwnd: HWND,
    surface: Option<Surface>,
    /// What the surface holds: size in px, radius, theme, scale.
    drawn: Option<(u32, u32, i32, bool, i32)>,
    shown: bool,
}

thread_local! {
    static WINS: RefCell<Vec<ShadowWin>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn shadow_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // Never takes the pointer or the focus.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn make_window() -> Option<HWND> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let class = w!("OrbShadow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(shadow_proc),
            hInstance: hinst,
            lpszClassName: class,
            ..Default::default()
        };
        // A second registration fails harmlessly.
        RegisterClassW(&wc);
        // Layered + transparent: the pointer goes through to whatever is under the shadow, and
        // the click-outside test (hook.rs) does not know this window, so a click on the shadow
        // still counts as outside.
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT,
            class,
            w!("Orby shadow"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst),
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        Some(hwnd)
    }
}

/// Keeps the shadow of the floating window `owner` under its visible shape. `shape` is the
/// shape in screen pixels (left, top, right, bottom), `radius` its corner radius in logical px,
/// `opacity` 0..1 fades it with the surface. Call it wherever the surface is placed; it redraws
/// only when the size, radius or theme changes.
pub fn follow(
    g: &Shared,
    owner: HWND,
    shape: (f32, f32, f32, f32),
    radius: f32,
    scale: f32,
    dark: bool,
    opacity: f32,
) {
    let reach_px = (reach(dark) * scale).ceil();
    let (l, t, r, b) = shape;
    let (w, h) = (
        ((r - l) + 2.0 * reach_px).round().max(1.0) as u32,
        ((b - t) + 2.0 * reach_px).round().max(1.0) as u32,
    );
    WINS.with(|ws| {
        let mut ws = ws.borrow_mut();
        let i = match ws.iter().position(|s| s.owner == owner) {
            Some(i) => i,
            None => {
                let Some(hwnd) = make_window() else { return };
                ws.push(ShadowWin {
                    owner,
                    hwnd,
                    surface: None,
                    drawn: None,
                    shown: false,
                });
                ws.len() - 1
            }
        };
        let s = &mut ws[i];
        let key = (w, h, (radius * 10.0) as i32, dark, (scale * 100.0) as i32);
        if s.drawn != Some(key) {
            match s.surface.as_mut() {
                None => s.surface = Surface::new(g, s.hwnd, w, h, scale).ok(),
                Some(sf) => {
                    let _ = sf.resize(g, w, h);
                }
            }
            if let Some(sf) = s.surface.as_ref() {
                let m = reach_px / scale;
                let (lw, lh) = ((r - l) / scale, (b - t) / scale);
                // Recorded before the window's BeginDraw: a device context draws one target at a time.
                let images: Vec<(ID2D1Image, f32)> = shadows(dark)
                    .iter()
                    .filter_map(|sh| unsafe {
                        shadow_image(g, m, m, m + lw, m + lh, radius, sh)
                            .ok()
                            .flatten()
                            .map(|i| (i, sh.y))
                    })
                    .collect();
                let dc = sf.begin(g);
                for (img, y) in &images {
                    unsafe {
                        dc.DrawImage(
                            img,
                            Some(&windows_numerics::Vector2 { X: 0.0, Y: *y }),
                            None,
                            D2D1_INTERPOLATION_MODE_LINEAR,
                            D2D1_COMPOSITE_MODE_SOURCE_OVER,
                        );
                    }
                }
                sf.end(g);
                s.drawn = Some(key);
            }
        }
        if let Some(sf) = s.surface.as_mut() {
            sf.set_opacity(g, opacity);
        }
        unsafe {
            let _ = SetWindowPos(
                s.hwnd,
                Some(owner),
                (l - reach_px).round() as i32,
                (t - reach_px).round() as i32,
                w as i32,
                h as i32,
                SWP_NOACTIVATE
                    | SWP_NOOWNERZORDER
                    | if s.shown {
                        SET_WINDOW_POS_FLAGS(0)
                    } else {
                        SWP_SHOWWINDOW
                    },
            );
        }
        s.shown = true;
    });
}

/// Hides the shadow of `owner` (its surface hid or closed).
pub fn hide(owner: HWND) {
    WINS.with(|ws| {
        for s in ws.borrow_mut().iter_mut().filter(|s| s.owner == owner) {
            if s.shown {
                unsafe {
                    let _ = ShowWindow(s.hwnd, SW_HIDE);
                }
                s.shown = false;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_shadow_is_tucked_and_white_is_soft() {
        // The frame's tokens.
        assert_eq!(SHADOW_DARK[0].spread, -18.0);
        assert_eq!(SHADOW_DARK[0].alpha, 0.55);
        assert_eq!(SHADOW_LIGHT[0].blur, 42.0);
        // Tucked: it reaches only 38 px past the edge (40 + 16 - 18).
        assert_eq!(reach(true), 38.0);
        assert_eq!(reach(false), 46.0);
    }
}
