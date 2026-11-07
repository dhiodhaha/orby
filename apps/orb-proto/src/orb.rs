//! The drop window: the liquid-metal shader, springs, docking to an edge, tucking, dragging.

use std::time::Instant;

use windows::core::{s, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use app_core::prayer::Times;
use app_downloads::Ring;

use crate::gfx::{self, Shared};
use crate::{
    dial, disk, downloads, habits, hook, log_line, notices, pill, prayer, px, start_anim, App,
    TIMER_TUCK,
};

const SHADER: &str = include_str!("metal.hlsl");

// Logical sizes (at 96 dpi).
pub const WIN: f32 = 68.0; // orb window
pub const ORB: f32 = 48.0; // the drop
pub const INSET: f32 = (WIN - ORB) / 2.0;
const RING_R: f32 = 30.0;
const EDGE_MARGIN: f32 = 8.0;
pub const NEAR: f32 = 20.0;
pub const SS: f32 = 2.0; // orb supersampling

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    orb_origin: [f32; 2],
    orb_size: f32,
    time: f32,
    angle: f32,
    distortion: f32,
    opacity: f32,
    brightness: f32,
    softness: f32,
    repetition: f32,
    shift_red: f32,
    shift_blue: f32,
    contour: f32,
    tint_a: f32,
    ring_start: f32,
    pad: f32,
    ring_mode: f32,
    ring_progress: f32,
    ring_radius: f32,
    px1: f32,
    ring_ink: [f32; 4],
    ring_track: [f32; 4],
    ring_halo: [f32; 4],
}

#[derive(Clone, Copy)]
pub struct Spring {
    pub pos: f32,
    vel: f32,
    pub target: f32,
    pub k: f32,
    pub c: f32,
}

impl Spring {
    pub fn new(v: f32, k: f32, c: f32) -> Self {
        Self {
            pos: v,
            vel: 0.0,
            target: v,
            k,
            c,
        }
    }
    pub fn step(&mut self, dt: f32) {
        let n = 4;
        let h = dt / n as f32;
        for _ in 0..n {
            let a = -self.k * (self.pos - self.target) - self.c * self.vel;
            self.vel += a * h;
            self.pos += self.vel * h;
        }
    }
    pub fn settled(&self, eps: f32) -> bool {
        (self.pos - self.target).abs() < eps && self.vel.abs() < eps * 10.0
    }
    pub fn snap(&mut self, v: f32) {
        self.pos = v;
        self.target = v;
        self.vel = 0.0;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

pub struct OrbGpu {
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    /// The same back buffer as a Direct2D target, for the badge drawn over the drop.
    badge_target: Option<ID2D1Bitmap1>,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

pub fn work_area(hwnd: HWND) -> RECT {
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

pub fn dock_x(app: &App, side: Side, tucked: bool) -> f32 {
    let wa = work_area(app.hwnd);
    let half_win = px(app, WIN / 2.0);
    match (side, tucked) {
        (Side::Right, false) => wa.right as f32 - px(app, WIN - INSET) - px(app, EDGE_MARGIN),
        (Side::Right, true) => wa.right as f32 - half_win,
        (Side::Left, false) => wa.left as f32 - px(app, INSET) + px(app, EDGE_MARGIN),
        (Side::Left, true) => wa.left as f32 - half_win,
    }
}

pub fn clamp_y(app: &App, y: f32) -> f32 {
    let wa = work_area(app.hwnd);
    let top = wa.top as f32 - px(app, INSET) + px(app, EDGE_MARGIN);
    let bottom = wa.bottom as f32 - px(app, WIN - INSET) - px(app, EDGE_MARGIN);
    y.clamp(top, bottom)
}

/// Highlight angle from the time of day, keyed to today's prayer times:
/// Subuh 28°, Dzuhur 90°, Ashar 128°, Maghrib 160°. After Isya the metal dims.
pub fn sun_from_clock(t: &Times) -> (f32, f32) {
    let h = prayer::now_minutes() / 60.0;
    let hr = |m: u32| m as f32 / 60.0;
    let keys = [
        (hr(t.subuh), 28.0),
        (hr(t.dzuhur), 90.0),
        (hr(t.ashar), 128.0),
        (hr(t.maghrib), 160.0),
    ];
    let mut angle = if h <= keys[0].0 { keys[0].1 } else { keys[3].1 };
    for w in keys.windows(2) {
        if h >= w[0].0 && h <= w[1].0 {
            let f = (h - w[0].0) / (w[1].0 - w[0].0);
            angle = w[0].1 + (w[1].1 - w[0].1) * f;
        }
    }
    let night = h < hr(t.subuh) || h >= hr(t.isya);
    if night {
        (90.0, 0.7)
    } else {
        (angle, 1.0)
    }
}

fn compile(entry: PCSTR, target: PCSTR) -> windows::core::Result<ID3DBlob> {
    unsafe {
        let mut code = None;
        let mut errors = None;
        let r = D3DCompile(
            SHADER.as_ptr() as _,
            SHADER.len(),
            s!("metal.hlsl"),
            None,
            None,
            entry,
            target,
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        );
        if let Err(e) = r {
            if let Some(err) = errors {
                let msg = std::slice::from_raw_parts(
                    err.GetBufferPointer() as *const u8,
                    err.GetBufferSize(),
                );
                let _ = std::fs::write(
                    crate::exe_dir().join("orb-proto-shader-error.txt"),
                    String::from_utf8_lossy(msg).as_bytes(),
                );
            }
            return Err(e);
        }
        Ok(code.unwrap())
    }
}

fn blob_bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

pub fn create_orb_gpu(g: &Shared, hwnd: HWND, size: u32) -> windows::core::Result<OrbGpu> {
    unsafe {
        let swap = g.swapchain(size, size)?;
        let (target, visual) = g.bind(hwnd, &swap, 1.0 / SS)?;
        let vsb = compile(s!("vs_main"), s!("vs_5_0"))?;
        let psb = compile(s!("ps_main"), s!("ps_5_0"))?;
        let mut vs = None;
        g.d3d
            .CreateVertexShader(blob_bytes(&vsb), None, Some(&mut vs))?;
        let mut ps = None;
        g.d3d
            .CreatePixelShader(blob_bytes(&psb), None, Some(&mut ps))?;
        let cbd = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Params>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let mut cb = None;
        g.d3d.CreateBuffer(&cbd, None, Some(&mut cb))?;
        let back: ID3D11Texture2D = swap.GetBuffer(0)?;
        let mut rtv = None;
        g.d3d.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        let surf: IDXGISurface = swap.GetBuffer(0)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let badge_target = g.dc.CreateBitmapFromDxgiSurface(&surf, Some(&props)).ok();
        Ok(OrbGpu {
            swap,
            rtv,
            vs: vs.unwrap(),
            ps: ps.unwrap(),
            cb: cb.unwrap(),
            badge_target,
            _target: target,
            _visual: visual,
        })
    }
}

fn premul(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0] * a, c[1] * a, c[2] * a, a]
}

pub fn render_orb(app: &mut App) {
    let k = app.scale * SS;
    let size = (WIN * k).round();
    let (ink, track) = if app.theme.dark {
        ([0.98, 0.98, 0.98], [1.0, 1.0, 1.0])
    } else {
        ([0.09, 0.09, 0.09], [0.0, 0.0, 0.0])
    };
    // Focus wins the ring while a session runs; otherwise downloads show the whole queue.
    let hair_a = if app.theme.dark { 0.24 } else { 0.18 };
    // A scan you started shows its progress over the downloads queue while it runs.
    let (queue, spin) = match disk::ring(app) {
        Some(r) => (r, disk::spin_start(app)),
        None => (downloads::ring(app), downloads::spin_start(app)),
    };
    let (ring_mode, ring_progress, track_a, ring_start, ink) = match (habits::ring(app), queue) {
        (Some(p), _) => (2.0, p, if app.theme.dark { 0.6 } else { 0.45 }, 0.0, ink),
        (None, Ring::Reading) => (1.0, 0.25, hair_a, spin, ink),
        (None, Ring::Progress { fraction, tone }) => (
            1.0,
            fraction as f32,
            hair_a,
            0.0,
            downloads::ring_color(app, tone),
        ),
        (None, Ring::Hidden) => (0.0, 0.0, 0.0, 0.0, ink),
    };
    let params = Params {
        orb_origin: [INSET * k, INSET * k],
        orb_size: ORB * k,
        time: app.shader_time,
        angle: app.sun_angle + app.lean.pos,
        distortion: app.distortion,
        opacity: app.opacity.pos.clamp(0.0, 1.0),
        brightness: app.brightness,
        softness: 0.12,
        repetition: 1.5,
        shift_red: 0.15,
        shift_blue: 0.15,
        contour: 0.4,
        tint_a: 0.4,
        ring_start,
        pad: 0.0,
        ring_mode,
        ring_progress,
        ring_radius: RING_R * k,
        px1: k,
        ring_ink: premul(ink, 1.0),
        ring_track: premul(track, track_a),
        ring_halo: if app.theme.dark {
            premul([0.06, 0.06, 0.06], 0.55)
        } else {
            premul([1.0, 1.0, 1.0], 0.75)
        },
    };
    let o = &app.orb_gpu;
    let c = &app.g.ctx;
    unsafe {
        c.UpdateSubresource(&o.cb, 0, None, &params as *const _ as _, 0, 0);
        let rtv = o.rtv.clone();
        c.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0, 0.0, 0.0, 0.0]);
        c.OMSetRenderTargets(Some(&[rtv]), None);
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size,
            Height: size,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        c.RSSetViewports(Some(&[vp]));
        c.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        c.VSSetShader(&o.vs, None);
        c.PSSetShader(&o.ps, None);
        c.PSSetConstantBuffers(0, Some(&[Some(o.cb.clone())]));
        c.Draw(3, 0);
    }
    draw_badge(app);
    unsafe {
        let _ = app.orb_gpu.swap.Present(1, DXGI_PRESENT(0));
    }
}

/// Overdue tasks plus the notices waiting behind the front pill (none while focus runs), as a
/// count on the drop's upper shoulder, on the side away from the edge so it stays readable while
/// tucked. Ink disc, panel-coloured digits and rim, so it reads on metal. A download that needs
/// you takes the spot with an ember "!" until the panel is opened.
fn draw_badge(app: &App) {
    let n = app.tasks.overdue + notices::badge_count(app);
    let ember = app.dl.attention || app.disk.attention;
    let Some(target) = app.orb_gpu.badge_target.as_ref().filter(|_| n > 0 || ember) else {
        return;
    };
    let label = if ember {
        "!".to_string()
    } else if n > 9 {
        "9+".to_string()
    } else {
        n.to_string()
    };
    let n = if ember { 1 } else { n };
    let t = &app.theme;
    let (cx, cy) = (
        WIN / 2.0 + if app.side == Side::Right { -17.0 } else { 17.0 },
        WIN / 2.0 - 17.0,
    );
    let dpi = 96.0 * app.scale * SS;
    let dc = &app.g.dc;
    unsafe {
        dc.SetTarget(target);
        dc.SetDpi(dpi, dpi);
        dc.BeginDraw();
        let w = if n > 9 { 7.0 } else { 0.0 };
        let rim = gfx::brush(dc, t.panel);
        dc.FillRoundedRectangle(
            &gfx::rrect(
                cx - 9.5 - w / 2.0,
                cy - 9.5,
                cx + 9.5 + w / 2.0,
                cy + 9.5,
                9.5,
            ),
            &rim,
        );
        let (fill, digits) = if ember {
            (t.ember, gfx::brush(dc, gfx::rgba(0xFFFFFF, 1.0)))
        } else {
            (t.text, rim.clone())
        };
        dc.FillRoundedRectangle(
            &gfx::rrect(
                cx - 8.0 - w / 2.0,
                cy - 8.0,
                cx + 8.0 + w / 2.0,
                cy + 8.0,
                8.0,
            ),
            &gfx::brush(dc, fill),
        );
        gfx::center_text(
            &app.g,
            dc,
            &label,
            &app.fonts.badge,
            cx,
            cy - 8.0,
            16.0,
            &digits,
        );
        let _ = dc.EndDraw(None, None);
        dc.SetTarget(None);
    }
}

pub fn place_orb(app: &App) {
    unsafe {
        let _ = SetWindowPos(
            app.hwnd,
            Some(HWND_TOPMOST),
            app.x.pos.round() as i32,
            app.y.round() as i32,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

pub fn set_orb_region(app: &App) {
    unsafe {
        // The drop plus the ring; clicks outside go to the app underneath.
        let a = px(app, WIN / 2.0 - RING_R - 3.0).round() as i32;
        let b = px(app, WIN / 2.0 + RING_R + 3.0).round() as i32;
        let rgn = CreateEllipticRgn(a, a, b, b);
        let _ = SetWindowRgn(app.hwnd, Some(rgn), true);
    }
}

pub fn orb_center(app: &App) -> (f32, f32) {
    (app.x.pos + px(app, WIN / 2.0), app.y + px(app, WIN / 2.0))
}

pub fn set_tucked(app: &mut App, tucked: bool) {
    if app.tucked == tucked {
        return;
    }
    app.tucked = tucked;
    app.x.target = dock_x(app, app.side, tucked);
    if tucked {
        app.x.k = 190.0;
        app.x.c = 26.0;
    } else {
        app.x.k = 480.0;
        app.x.c = 34.0;
    }
    app.opacity.target = if tucked { 0.4 } else { 1.0 };
    start_anim(app);
}

/// A window moved every frame is raised only as it appears: raising it again would lift it over
/// the format menu shown after it.
pub fn raise_once(hwnd: HWND) -> SET_WINDOW_POS_FLAGS {
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        SWP_NOZORDER
    } else {
        SET_WINDOW_POS_FLAGS(0)
    }
}

pub fn arm_tuck_timer(app: &App) {
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_TUCK, 3000, None);
    }
}

pub fn set_visible(app: &mut App, v: bool) {
    app.visible = v;
    unsafe {
        let _ = ShowWindow(app.hwnd, if v { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
    log_line(app, if v { "shown" } else { "hidden" });
}

/// One animation frame of the drop: springs, the prayer flow, then draw.
pub fn tick(app: &mut App, now: Instant, dt: f32) {
    if !app.dragging {
        app.x.step(dt);
    }
    app.opacity.step(dt);
    app.lean.step(dt);
    if let Some(start) = app.flow {
        let e = (now - start).as_secs_f32();
        let d = 5.0;
        if e >= d {
            app.flow = None;
            app.distortion = 0.1;
        } else {
            let env = (std::f32::consts::PI * e / d).sin();
            app.shader_time += dt * (0.4 + 1.6 * env);
            app.distortion = 0.1 + 0.18 * env;
        }
    }
    place_orb(app);
    render_orb(app);
}

pub fn is_animating(app: &App) -> bool {
    app.flow.is_some()
        || downloads::spinning(app)
        || disk::spinning(app)
        || !app.x.settled(0.3)
        || !app.opacity.settled(0.002)
        || !app.lean.settled(0.05)
}

// ---- input on the drop window ----

pub fn on_press(app: &mut App, hwnd: HWND) {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        app.press = Some((pt, app.x.pos as i32, app.y as i32));
        app.dragging = false;
        let _ = SetCapture(hwnd);
        let _ = KillTimer(Some(hwnd), TIMER_TUCK);
    }
}

pub fn on_move(app: &mut App) {
    if let Some((p0, wx, wy)) = app.press {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let (dx, dy) = (pt.x - p0.x, pt.y - p0.y);
        if !app.dragging && (dx.abs() > 3 || dy.abs() > 3) {
            app.dragging = true;
            dial::close_dial(app);
            if app.pill.is_some() {
                pill::dismiss_pill(app);
            }
        }
        if app.dragging {
            app.x.snap((wx + dx) as f32);
            app.y = (wy + dy) as f32;
            place_orb(app);
            hook::update_hook(app);
        }
    }
}

/// Mouse up: ends a drag by docking to the nearer edge, otherwise a click toggles the dial.
pub fn on_release(app: &mut App, hwnd: HWND) {
    let was_drag = app.dragging;
    app.press = None;
    app.dragging = false;
    if was_drag {
        let wa = work_area(hwnd);
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        app.side = if pt.x < (wa.left + wa.right) / 2 {
            Side::Left
        } else {
            Side::Right
        };
        app.tucked = false;
        app.x.k = 320.0;
        app.x.c = 17.0;
        app.x.target = dock_x(app, app.side, false);
        app.y = clamp_y(app, app.y);
        start_anim(app);
        log_line(app, "docked");
        arm_tuck_timer(app);
    } else if app.dial_open {
        dial::close_dial(app);
    } else {
        dial::open_dial(app, false);
    }
}

/// The pointer came near (or left): slide out when close, lean the highlight toward it.
pub fn on_near(app: &mut App, hwnd: HWND, near: bool, x: f32, y: f32) {
    let (cx, cy) = orb_center(app);
    if near {
        let close = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() < px(app, ORB / 2.0 + NEAR);
        disk::on_orb_near(app, close);
        downloads::on_orb_near(app, close);
        notices::on_orb_near(app, close);
        if close {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_TUCK);
            }
            set_tucked(app, false);
        }
        let lean = ((x - cx) / px(app, 60.0)).clamp(-1.0, 1.0) * 28.0;
        if (app.lean.target - lean).abs() > 0.5 {
            app.lean.target = lean;
            start_anim(app);
        }
        app.near = true;
    } else if app.near {
        app.near = false;
        disk::on_orb_gone(app);
        downloads::on_orb_gone(app);
        notices::on_orb_gone(app);
        app.lean.target = 0.0;
        start_anim(app);
        if !app.dial_open && app.pill.is_none() && !habits::focus_running(app) {
            arm_tuck_timer(app);
        }
    }
}

pub fn on_tuck_timer(app: &mut App, hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_TUCK);
    }
    // A running focus keeps the orb out so its ring stays readable.
    if !app.near
        && app.press.is_none()
        && app.flow.is_none()
        && !app.dial_open
        && app.pill.is_none()
    {
        set_tucked(app, true);
    }
}

/// Re-dock after a display change.
pub fn on_display_change(app: &mut App) {
    app.x.snap(dock_x(app, app.side, app.tucked));
    app.y = clamp_y(app, app.y);
    place_orb(app);
    hook::update_hook(app);
}

#[cfg(test)]
mod tests {
    use super::Spring;

    #[test]
    fn bouncy_spring_overshoots_then_settles_on_its_target() {
        // The pill's spring, stepped at 75 Hz for 2 s.
        let mut s = Spring::new(0.0, 320.0, 17.0);
        s.target = 1.0;
        let mut overshoot = 0.0f32;
        for _ in 0..150 {
            s.step(1.0 / 75.0);
            overshoot = overshoot.max(s.pos - 1.0);
        }
        assert!(overshoot > 0.0);
        assert!(s.settled(0.002));
    }

    #[test]
    fn snap_stops_the_spring_in_place() {
        let mut s = Spring::new(0.0, 480.0, 34.0);
        s.target = 10.0;
        s.step(0.01);
        s.snap(4.0);
        assert!(s.settled(1e-6));
        assert_eq!(s.pos, 4.0);
    }
}
