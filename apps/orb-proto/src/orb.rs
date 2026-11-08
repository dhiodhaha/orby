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
    dial, disk, downloads, habits, hook, log_line, notices, panels, pill, prayer, px, start_anim,
    App, TIMER_TUCK,
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
    /// Horizontal stretch of the drop (0 = round); the height shrinks to keep its volume.
    squash: f32,
    ring_mode: f32,
    ring_progress: f32,
    ring_radius: f32,
    px1: f32,
    /// Where the metal noise field is sampled from: it wanders along a random path while active,
    /// so the flow never slides the same way twice.
    wander: [f32; 2],
    /// Strength of a second, slowly drifting warp of the stripes (0 at idle).
    warp: f32,
    _pad: f32,
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
    pub fn vel(&self) -> f32 {
        self.vel
    }
    /// Adds velocity, so a moving spring is nudged from where it is rather than restarted.
    pub fn kick(&mut self, dv: f32) {
        self.vel += dv;
    }
    fn retune(&mut self, target: f32, k: f32, c: f32) {
        self.target = target;
        self.k = k;
        self.c = c;
    }
}

// The drop's life while you use it. Two layers:
// - Activity: while you engage with Orby (pointer on or close to the drop, the dial open, a drag,
//   a panel or a pill showing) the metal flows in a slow loop, the drop breathes and the highlight
//   drifts. Activity climbs in 250 ms, holds 300 ms after you stop, then eases out over 800 ms and
//   lands on exactly 0: idle is a still frame again and no frames are drawn.
// - Reactions: press, release pulse, picks, notices, dock wobble. Springs and decays on top of
//   the loop that always come back to rest.

/// Scale while the button is held: felt more than seen.
const PRESS_SCALE: f32 = 0.94;
/// Scale while the pointer is on the drop.
const HOVER_SCALE: f32 = 1.03;
/// How fast the metal's flow dies away after a kick (seconds, exponential).
const FLOW_TAU: f32 = 0.55;
/// Below this the flow stops and the frame is still.
const FLOW_STILL: f32 = 0.04;
/// The most the pointer moving near the drop can stir the metal.
const STIR_MAX: f32 = 0.6;
/// The stretch at full speed (a dock across the screen); a tuck slide gets about a fifth of it.
const STRETCH_MAX: f32 = 0.05;
/// Horizontal speed (logical px/s) at which the stretch reaches about 76% of its maximum.
const STRETCH_SPEED: f32 = 1500.0;

/// Seconds for activity to climb 0 -> 1 (smoothstepped, so it reads as about 200 ms).
const ACT_RISE: f32 = 0.25;
/// Seconds for activity to fall 1 -> 0 once the grace has run out.
const ACT_FALL: f32 = 0.8;
/// Engagement may drop this long (moving between dial items, crossing the drop's edge) before
/// activity starts to fall.
const ACT_GRACE: f32 = 0.3;
/// Shader time per second at full activity: a calm flow, about half Paper's default speed.
const ACT_FLOW: f32 = 0.45;
/// Extra edge distortion at full activity (the resting value is 0.1).
const ACT_DISTORT: f32 = 0.035;
/// Breathing: scale up to +-0.8%, an irregular rhythm around 0.3 Hz, only while active.
const BREATH_AMP: f32 = 0.008;
const BREATH_HZ: f32 = 0.3;
/// The highlight sways up to +-9 degrees, wandering (no fixed period), only while active.
const DRIFT_DEG: f32 = 9.0;
const DRIFT_HZ: f32 = 0.08;
/// Flow speed wanders this far either side of ACT_FLOW (0.35 = 0.65x .. 1.35x).
const FLOW_VARY: f32 = 0.35;
/// How fast the noise field drifts (uv per second at full activity) and how often its heading
/// changes (noise cycles per second).
const WANDER_SPEED: f32 = 0.09;
const WANDER_TURN_HZ: f32 = 0.05;
/// The second stripe warp at full activity, and how fast its strength breathes.
const WARP_MAX: f32 = 1.0;
const WARP_HZ: f32 = 0.06;
/// A pick or click ripple: extra edge distortion that dies away (seconds, exponential).
const RIPPLE_DISTORT: f32 = 0.11;
const RIPPLE_TAU: f32 = 0.3;
const RIPPLE_STILL: f32 = 0.02;

/// How engaged you are with Orby right now, as a target for the activity level.
#[derive(Clone, Copy, Default)]
pub struct Engaged {
    /// The button is down on the drop, or it is being dragged.
    pub pressed: bool,
    pub dial_open: bool,
    /// A dial item is under the pointer or the keyboard.
    pub dial_aim: bool,
    /// The pointer is on the drop.
    pub over: bool,
    /// The pointer is in the near zone (20 px around the drop).
    pub close: bool,
    pub panel_open: bool,
    pub pill: bool,
}

impl Engaged {
    pub fn level(&self) -> f32 {
        let mut l: f32 = 0.0;
        if self.pill {
            l = l.max(0.3);
        }
        if self.panel_open {
            l = l.max(0.4);
        }
        if self.close {
            l = l.max(0.7);
        }
        if self.dial_open {
            l = l.max(if self.dial_aim { 1.0 } else { 0.85 });
        }
        if self.over || self.pressed {
            l = l.max(1.0);
        }
        l
    }
}

/// The activity level: follows engagement with a grace hold and linear ramps, read smoothstepped.
#[derive(Clone, Copy, Default)]
pub struct Activity {
    level: f32,
    hold: f32,
    grace: f32,
}

impl Activity {
    pub fn step(&mut self, target: f32, dt: f32) {
        let target = target.clamp(0.0, 1.0);
        if target >= self.hold {
            self.hold = target;
            self.grace = ACT_GRACE;
        } else {
            self.grace -= dt;
            if self.grace <= 0.0 {
                self.hold = target;
                self.grace = ACT_GRACE;
            }
        }
        if self.hold > self.level {
            self.level = (self.level + dt / ACT_RISE).min(self.hold);
        } else {
            self.level = (self.level - dt / ACT_FALL).max(self.hold);
        }
    }

    /// 0 idle .. 1 fully active; both ramps start and land softly.
    pub fn value(&self) -> f32 {
        let x = self.level;
        x * x * (3.0 - 2.0 * x)
    }

    /// Nothing to animate: level and hold are exactly 0.
    pub fn still(&self) -> bool {
        self.level == 0.0 && self.hold == 0.0
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }
}

pub struct Life {
    /// Uniform scale of the drop: press, hover, the click pulse and a notice's bulge.
    pub squish: Spring,
    /// Stretch along the horizontal motion; it lags the slide, so the drop wobbles once it lands.
    pub wobble: Spring,
    /// Extra shader time per second. Kicked by interactions, decays to 0 (a still frame).
    pub flow: f32,
    /// Engagement, 0 idle .. 1 active: drives the looping flow, breathing and drift.
    pub activity: Activity,
    /// Seconds of active time since this activation began: the clock for the random loop.
    phase: f32,
    /// Fresh for each activation, so every session of the loop looks different.
    seed: u32,
    /// Where the noise field is sampled from (see Params::wander).
    wander: [f32; 2],
    /// A pick or click ripple through the drop's edge, decaying to 0.
    ripple: f32,
    /// The pill, dial and panel should tick this frame. Set by start_anim, so the drop's own
    /// loop never makes them redraw.
    pub repaint: bool,
    /// Where tick last placed the window.
    placed: Option<(i32, i32)>,
    /// The pointer is on the drop.
    over: bool,
    /// The pointer is in the near zone around the drop.
    close: bool,
    /// The last pointer position near the drop, for stirring.
    last_pt: Option<(f32, f32)>,
    dial_was_open: bool,
    /// Windows "Animation effects" is off: no squish, wobble or flow.
    calm: bool,
}

impl Life {
    pub fn new() -> Self {
        Self {
            squish: Spring::new(1.0, 420.0, 30.0),
            wobble: Spring::new(0.0, 500.0, 14.0),
            flow: 0.0,
            activity: Activity::default(),
            phase: 0.0,
            seed: fresh_seed(),
            wander: [0.0, 0.0],
            ripple: 0.0,
            repaint: false,
            placed: None,
            over: false,
            close: false,
            last_pt: None,
            dial_was_open: false,
            calm: downloads::reduced_motion(),
        }
    }

    fn energize(&mut self, v: f32) {
        if !self.calm {
            self.flow = self.flow.max(v);
        }
    }

    /// The resting scale for where the pointer is.
    fn rest_scale(&self) -> f32 {
        if self.over {
            HOVER_SCALE
        } else {
            1.0
        }
    }

    fn settle_scale(&mut self, k: f32, c: f32) {
        if self.calm {
            self.squish.snap(1.0);
        } else {
            let t = self.rest_scale();
            self.squish.retune(t, k, c);
        }
    }

    fn is_moving(&self) -> bool {
        self.flow > 0.0
            || self.ripple > 0.0
            || !self.squish.settled(0.001)
            || !self.wobble.settled(0.001)
    }

    /// Breathing scale factor and highlight sway (degrees) for the current activity.
    fn breath_and_drift(&self) -> (f32, f32) {
        let a = self.activity.value();
        if a == 0.0 {
            return (1.0, 0.0);
        }
        (
            1.0 + BREATH_AMP * a * wiggle(self.seed, 1, BREATH_HZ * self.phase),
            DRIFT_DEG * a * wiggle(self.seed, 2, DRIFT_HZ * self.phase),
        )
    }

    /// Strength of the stripe warp right now (0 at idle).
    fn warp(&self) -> f32 {
        let a = self.activity.value();
        WARP_MAX * a * (0.55 + 0.45 * wiggle(self.seed, 5, WARP_HZ * self.phase))
    }
}

/// A pick or click landed: a pulse (the drop swells about 3% at strength 1), a ripple through
/// the metal's edge and a burst of flow, on top of the active loop. Nothing under reduced motion.
pub fn kick(app: &mut App, strength: f32) {
    let l = &mut app.life;
    l.calm = downloads::reduced_motion();
    if l.calm {
        return;
    }
    let s = strength.clamp(0.0, 1.5);
    let t = l.rest_scale();
    l.squish.retune(t, 420.0, 16.0);
    l.squish.kick(0.7 * s);
    l.ripple = l.ripple.max(s);
    l.energize(0.5 + 0.6 * s);
    start_anim(app);
}

/// Whether the pill, dial and panel tick this frame: only when one of them (or the drop's own
/// springs) moves, or a frame was asked for. The active loop alone redraws only the drop.
pub fn full_frame(app: &mut App) -> bool {
    std::mem::take(&mut app.life.repaint)
        || moving_besides_loop(app)
        || pill::is_animating(app)
        || dial::is_animating(app)
        || panels::is_animating(app)
}

/// A notice arrived: the drop swells once (about 4%) and the metal runs, then settles.
pub fn kick_notice(app: &mut App) {
    let l = &mut app.life;
    l.calm = downloads::reduced_motion();
    if l.calm {
        return;
    }
    let t = l.rest_scale();
    l.squish.retune(t, 420.0, 16.0);
    l.squish.kick(0.9);
    l.ripple = l.ripple.max(0.7);
    l.energize(0.9);
    start_anim(app);
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
    let (breath, drift) = app.life.breath_and_drift();
    let drop = ORB * k * (app.life.squish.pos * breath).clamp(0.85, 1.1);
    let distortion =
        app.distortion + ACT_DISTORT * app.life.activity.value() + RIPPLE_DISTORT * app.life.ripple;
    let params = Params {
        orb_origin: [(size - drop) / 2.0, (size - drop) / 2.0],
        orb_size: drop,
        time: app.shader_time,
        angle: app.sun_angle + app.lean.pos + drift,
        distortion,
        opacity: app.opacity.pos.clamp(0.0, 1.0),
        brightness: app.brightness,
        softness: 0.12,
        repetition: 1.5,
        shift_red: 0.15,
        shift_blue: 0.15,
        contour: 0.4,
        tint_a: 0.4,
        ring_start,
        squash: app.life.wobble.pos.clamp(-0.08, 0.08),
        ring_mode,
        ring_progress,
        ring_radius: RING_R * k,
        px1: k,
        wander: app.life.wander,
        warp: app.life.warp(),
        _pad: 0.0,
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
    if !tucked {
        app.life.energize(0.35);
    }
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
    step_life(app, dt);
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
    let at = (app.x.pos.round() as i32, app.y.round() as i32);
    if app.life.placed != Some(at) {
        app.life.placed = Some(at);
        place_orb(app);
    }
    render_orb(app);
}

/// What you are doing with Orby right now.
fn engaged(app: &App) -> Engaged {
    if !app.visible {
        // A pill can show while the drop is hidden: nothing to loop.
        return Engaged::default();
    }
    Engaged {
        pressed: app.press.is_some(),
        dial_open: app.dial_open,
        dial_aim: app.dial_open && (app.dial_aim.is_some() || app.dial_child.is_some()),
        over: app.near && app.life.over,
        close: app.near && app.life.close,
        panel_open: app.panel_open,
        pill: app.pill.is_some() && !app.pill_closing,
    }
}

fn step_life(app: &mut App, dt: f32) {
    if app.anim_frames == 0 {
        app.life.calm = downloads::reduced_motion();
    }
    if app.dial_open != app.life.dial_was_open {
        app.life.dial_was_open = app.dial_open;
        app.life.energize(if app.dial_open { 0.9 } else { 0.4 });
    }
    let target = engaged(app).level();
    let l = &mut app.life;
    if l.calm {
        l.activity.stop();
    } else {
        l.activity.step(target, dt);
    }
    let a = l.activity.value();
    if a > 0.0 {
        // A new activation (from a still frame) gets a new seed, so no two loops look alike.
        if l.phase == 0.0 {
            l.seed = fresh_seed();
        }
        l.phase += dt;
        let speed = 1.0 + FLOW_VARY * wiggle(l.seed, 3, 0.11 * l.phase);
        app.shader_time += dt * ACT_FLOW * a * speed;
        let heading = std::f32::consts::PI * wiggle(l.seed, 4, WANDER_TURN_HZ * l.phase);
        l.wander[0] += dt * WANDER_SPEED * a * heading.cos();
        l.wander[1] += dt * WANDER_SPEED * a * heading.sin();
    } else {
        l.phase = 0.0;
    }
    if l.ripple > 0.0 {
        l.ripple *= (-dt / RIPPLE_TAU).exp();
        if l.ripple < RIPPLE_STILL {
            l.ripple = 0.0;
        }
    }
    if l.flow > 0.0 {
        app.shader_time += dt * l.flow;
        l.flow *= (-dt / FLOW_TAU).exp();
        if l.flow < FLOW_STILL {
            l.flow = 0.0;
        }
    }
    l.squish.step(dt);
    let speed = if l.calm || app.dragging {
        0.0
    } else {
        app.x.vel().abs() / app.scale
    };
    l.wobble.target = STRETCH_MAX * (speed / STRETCH_SPEED).tanh();
    l.wobble.step(dt);
    if l.calm {
        l.squish.snap(1.0);
        l.wobble.snap(0.0);
        l.flow = 0.0;
        l.ripple = 0.0;
    }
}

/// The drop draws while active (the loop) or while anything else on it moves. Idle means
/// activity exactly 0, springs settled, flow and ripple 0: this is false and no frames run.
pub fn is_animating(app: &App) -> bool {
    !app.life.activity.still() || moving_besides_loop(app)
}

/// Everything that moves the drop except the active loop.
fn moving_besides_loop(app: &App) -> bool {
    app.flow.is_some()
        || app.life.is_moving()
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
    // Press: the drop sinks fast (about 120 ms, no bounce) and the metal starts to run.
    let l = &mut app.life;
    l.calm = downloads::reduced_motion();
    if !l.calm {
        l.squish.retune(PRESS_SCALE, 900.0, 48.0);
        l.energize(1.2);
    }
    start_anim(app);
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
            // Picked up: the drop comes back to full size and follows the pointer.
            app.life.settle_scale(420.0, 26.0);
            start_anim(app);
            dial::close_dial(app);
            if app.pill.is_some() {
                pill::dismiss_pill(app);
            }
        }
        if app.dragging {
            // Dragging stirs the metal by how far the drop travels.
            let step = ((wx + dx) as f32 - app.x.pos).hypot((wy + dy) as f32 - app.y) / app.scale;
            let l = &mut app.life;
            if !l.calm && step > 0.0 && l.flow < 0.8 {
                l.flow = (l.flow + step * 0.004).min(0.8);
                start_anim(app);
            }
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
    // Release: back past rest with one small overshoot (about 2%), the click pulse.
    app.life.settle_scale(420.0, 16.0);
    app.life.energize(if was_drag { 0.6 } else { 1.0 });
    if !was_drag && !app.life.calm {
        app.life.ripple = app.life.ripple.max(0.6);
    }
    start_anim(app);
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
        let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        let close = dist < px(app, ORB / 2.0 + NEAR);
        on_pointer_life(app, x, y, dist, close);
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
        app.life.last_pt = None;
        app.life.close = false;
        if app.life.over {
            app.life.over = false;
            if app.press.is_none() {
                app.life.settle_scale(420.0, 30.0);
            }
        }
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

/// Hover scale on the drop, and the metal stirring as the pointer moves close to it.
fn on_pointer_life(app: &mut App, x: f32, y: f32, dist: f32, close: bool) {
    if !app.near {
        app.life.calm = downloads::reduced_motion();
    }
    let over = dist < px(app, ORB / 2.0 + 2.0);
    let mut moved = false;
    if close != app.life.close {
        // Coming close wakes the loop; leaving lets it ease out after the grace.
        app.life.close = close;
        moved = true;
    }
    if over != app.life.over {
        app.life.over = over;
        if app.press.is_none() {
            app.life.settle_scale(420.0, 30.0);
            moved = true;
        }
    }
    let last = app.life.last_pt.replace((x, y));
    if close && !app.life.calm && !app.dragging {
        // Strongest right at the drop, weakest at the edge of the near zone.
        let falloff = 1.0 - (dist / px(app, ORB / 2.0 + NEAR)).clamp(0.0, 1.0);
        let step = last.map_or(0.0, |(lx, ly)| (x - lx).hypot(y - ly) / app.scale);
        let add = step * 0.006 * (0.4 + 0.6 * falloff);
        let l = &mut app.life;
        if add > 0.0 && l.flow < STIR_MAX {
            l.flow = (l.flow + add).min(STIR_MAX);
            moved = true;
        }
    }
    if moved {
        start_anim(app);
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

/// A seed from the clock: different every activation, no RNG crate needed.
fn fresh_seed() -> u32 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9);
    hash32((n ^ (n >> 29)) as u32)
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// Smooth 1D value noise in -1..1 for one channel of a seed: two octaves, so it never settles
/// into a visible period the way a sine does.
fn wiggle(seed: u32, channel: u32, x: f32) -> f32 {
    fn octave(seed: u32, x: f32) -> f32 {
        let i = x.floor();
        let f = x - i;
        let at = |n: i64| {
            hash32(seed ^ (n as u32).wrapping_mul(0x9e37_79b9)) as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        let (a, b) = (at(i as i64), at(i as i64 + 1));
        let t = f * f * (3.0 - 2.0 * f);
        a + (b - a) * t
    }
    let s = hash32(seed ^ channel.wrapping_mul(0x85eb_ca6b));
    (0.7 * octave(s, x) + 0.3 * octave(s ^ 0x5bd1_e995, x * 2.13 + 7.7)).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::Spring;

    #[test]
    fn the_loop_noise_is_smooth_bounded_and_differs_per_seed() {
        let (a, b) = (super::wiggle(11, 3, 0.0), super::wiggle(12, 3, 0.0));
        assert!(
            (a - b).abs() > 1e-4,
            "two activations start from different shapes"
        );
        let mut prev = super::wiggle(7, 1, 0.0);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 1..20_000 {
            let v = super::wiggle(7, 1, i as f32 * 0.001);
            assert!((-1.0..=1.0).contains(&v));
            // At 1 ms steps the value never jumps: no visible pop in the metal.
            assert!((v - prev).abs() < 0.01, "jump at {i}: {prev} -> {v}");
            prev = v;
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(hi - lo > 0.5, "it actually wanders");
        // No short period: the curve one cycle later is not the same curve.
        let diff: f32 = (0..100)
            .map(|i| {
                (super::wiggle(7, 1, i as f32 * 0.1) - super::wiggle(7, 1, 1.0 + i as f32 * 0.1))
                    .abs()
            })
            .sum();
        assert!(diff > 5.0);
    }

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
    fn click_pulse_overshoots_a_little_then_rests() {
        // Press to 0.94, then release on the "pulse" spring (420 / 16) at 60 Hz.
        let mut s = Spring::new(1.0, 900.0, 48.0);
        s.target = super::PRESS_SCALE;
        for _ in 0..9 {
            s.step(1.0 / 60.0); // a 150 ms click
        }
        assert!(s.pos < 0.96);
        s.retune(1.0, 420.0, 16.0);
        let mut peak = 0.0f32;
        let mut frames = 0;
        while !s.settled(0.001) {
            s.step(1.0 / 60.0);
            peak = peak.max(s.pos);
            frames += 1;
            assert!(frames < 120, "pulse still moving after 2 s");
        }
        assert!(peak > 1.005 && peak < 1.03, "overshoot {peak}");
    }

    #[test]
    fn notice_bulge_and_dock_wobble_come_to_rest() {
        let mut bulge = Spring::new(1.0, 420.0, 16.0);
        bulge.kick(0.9);
        let mut wobble = Spring::new(0.0, 500.0, 14.0);
        wobble.target = super::STRETCH_MAX;
        let mut peak = 0.0f32;
        for _ in 0..12 {
            wobble.step(1.0 / 60.0);
            bulge.step(1.0 / 60.0);
            peak = peak.max(bulge.pos);
        }
        assert!(peak > 1.02 && peak < 1.06, "bulge {peak}");
        wobble.target = 0.0; // landed
        let mut frames = 0;
        while !(bulge.settled(0.001) && wobble.settled(0.001)) {
            wobble.step(1.0 / 60.0);
            bulge.step(1.0 / 60.0);
            frames += 1;
            assert!(frames < 120, "still moving after 2 s");
        }
    }

    #[test]
    fn metal_flow_decays_to_a_still_frame() {
        let mut flow = 1.2f32;
        let mut frames = 0;
        while flow > 0.0 {
            flow *= (-(1.0f32 / 60.0) / super::FLOW_TAU).exp();
            if flow < super::FLOW_STILL {
                flow = 0.0;
            }
            frames += 1;
        }
        // About 1.9 s from the strongest kick, most of the movement in the first half second.
        assert!((100..125).contains(&frames), "{frames}");
    }

    const DT: f32 = 1.0 / 60.0;

    fn run(a: &mut super::Activity, target: f32, secs: f32) {
        for _ in 0..(secs / DT).round() as usize {
            a.step(target, DT);
        }
    }

    #[test]
    fn activity_ramps_up_in_about_a_quarter_second() {
        let mut a = super::Activity::default();
        run(&mut a, 1.0, 0.1);
        assert!(a.value() > 0.1 && a.value() < 0.6, "{}", a.value());
        run(&mut a, 1.0, 0.17);
        assert_eq!(a.value(), 1.0);
    }

    #[test]
    fn activity_holds_through_the_grace_then_eases_out_to_exact_zero() {
        let mut a = super::Activity::default();
        run(&mut a, 1.0, 0.5);
        // Moving between dial items: engagement drops for 200 ms, nothing changes.
        run(&mut a, 0.0, 0.2);
        assert_eq!(a.value(), 1.0);
        run(&mut a, 1.0, 0.05);
        // Engagement ends: still full through the grace, then falls over 0.8 s.
        run(&mut a, 0.0, 0.28);
        assert_eq!(a.value(), 1.0);
        run(&mut a, 0.0, 0.4);
        assert!(a.value() > 0.2 && a.value() < 0.8, "{}", a.value());
        assert!(!a.still());
        run(&mut a, 0.0, 0.5);
        assert_eq!(a.value(), 0.0);
        assert!(a.still(), "idle must be exactly still");
    }

    #[test]
    fn activity_settles_on_a_lower_engagement() {
        // The dial closes into a panel: down to the panel's 0.4 and stays there, never 0.
        let mut a = super::Activity::default();
        let dial = super::Engaged {
            dial_open: true,
            dial_aim: true,
            ..Default::default()
        };
        let panel = super::Engaged {
            panel_open: true,
            ..Default::default()
        };
        run(&mut a, dial.level(), 0.5);
        run(&mut a, panel.level(), 3.0);
        assert!((a.level - 0.4).abs() < 1e-6);
        assert!(!a.still());
        run(&mut a, 0.0, 1.0);
        assert!(a.still());
    }

    #[test]
    fn engagement_levels_rank_as_designed() {
        use super::Engaged;
        let none = Engaged::default();
        assert_eq!(none.level(), 0.0);
        let pill = Engaged { pill: true, ..none };
        let panel = Engaged {
            panel_open: true,
            ..pill
        };
        let close = Engaged {
            close: true,
            ..panel
        };
        let dial = Engaged {
            dial_open: true,
            ..close
        };
        let aim = Engaged {
            dial_aim: true,
            ..dial
        };
        let levels = [pill, panel, close, dial, aim].map(|e| e.level());
        assert!(levels.windows(2).all(|w| w[0] < w[1]), "{levels:?}");
        assert_eq!(
            Engaged {
                pressed: true,
                ..none
            }
            .level(),
            1.0
        );
    }

    #[test]
    fn ripple_dies_away_in_about_a_second() {
        let mut r = 1.0f32;
        let mut frames = 0;
        while r > 0.0 {
            r *= (-DT / super::RIPPLE_TAU).exp();
            if r < super::RIPPLE_STILL {
                r = 0.0;
            }
            frames += 1;
        }
        assert!((60..75).contains(&frames), "{frames}");
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
