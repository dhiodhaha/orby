#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! Shared GPU objects and Direct2D surfaces on DirectComposition.
//!
//! One D3D11 device, one DirectComposition device, one D2D device context and one
//! DirectWrite factory serve every window (orb, pill, dial).

use windows::core::{w, Interface, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
pub use windows_numerics::Vector2;

use std::cell::RefCell;

/// Inter Variable 4.1 (SIL OFL 1.1, assets/fonts/OFL.txt), bundled so every machine draws the
/// same type. Loaded from memory by `load_inter`; nothing is installed on the system.
static INTER_TTF: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
/// The family names DirectWrite may give the bundled font. With its weight-width-slope model it
/// names the family after the optical size instances ("Inter Variable Text"); the font's own
/// name ID 1 is "Inter Variable".
const INTER_FAMILIES: [windows::core::PCWSTR; 2] =
    [w!("Inter Variable Text"), w!("Inter Variable")];
const FALLBACK_FAMILY: windows::core::PCWSTR = w!("Segoe UI Variable Text");

thread_local! {
    /// The factory and the shared typography (tnum + cv11) for `text`, which has no `Shared`.
    /// Every window draws on the one UI thread.
    static TEXT: RefCell<Option<(IDWriteFactory, Option<IDWriteTypography>)>> =
        const { RefCell::new(None) };
}

pub struct Shared {
    pub d3d: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub factory: IDXGIFactory2,
    pub dcomp: IDCompositionDevice,
    pub d2f: ID2D1Factory1,
    pub dc: ID2D1DeviceContext,
    pub dw: IDWriteFactory,
    /// The bundled Inter, or None when it could not be loaded (then Segoe UI Variable is used).
    inter: Option<(IDWriteFontCollection, windows::core::PCWSTR)>,
    /// Tabular figures and the single-storey a (cv11) on every layout.
    typography: Option<IDWriteTypography>,
    /// One line for the log: which face text uses and why.
    pub font_note: String,
}

/// Loads the embedded Inter Variable through an in-memory font file loader into its own font
/// collection (IDWriteFactory5, Windows 10 1703 and later).
fn load_inter(dw: &IDWriteFactory) -> Result<(IDWriteFontCollection, windows::core::PCWSTR)> {
    unsafe {
        let f5: IDWriteFactory5 = dw.cast()?;
        let loader = f5.CreateInMemoryFontFileLoader()?;
        f5.RegisterFontFileLoader(&loader)?;
        // The bytes are 'static, so the loader can point at them instead of copying 860 KB; the
        // factory stands in as the owner object that keeps the reference alive.
        let owner: windows::core::IUnknown = f5.cast()?;
        let file = loader.CreateInMemoryFontFileReference(
            &f5,
            INTER_TTF.as_ptr() as *const _,
            INTER_TTF.len() as u32,
            &owner,
        )?;
        let builder = f5.CreateFontSetBuilder()?;
        builder.AddFontFile(&file)?;
        let set = builder.CreateFontSet()?;
        let coll = f5.CreateFontCollectionFromFontSet(&set)?;
        let coll: IDWriteFontCollection = coll.cast()?;
        // The family must be there, or every format would silently fall back to the system font.
        for family in INTER_FAMILIES {
            let mut index = 0u32;
            let mut found = windows::core::BOOL::default();
            coll.FindFamilyName(family, &mut index, &mut found)?;
            if found.as_bool() {
                return Ok((coll, family));
            }
        }
        Err(windows::core::Error::from(E_FAIL))
    }
}

/// OpenType tag from four ASCII bytes (DWRITE_MAKE_OPENTYPE_TAG).
const fn ot_tag(t: &[u8; 4]) -> u32 {
    (t[0] as u32) | (t[1] as u32) << 8 | (t[2] as u32) << 16 | (t[3] as u32) << 24
}

fn make_typography(dw: &IDWriteFactory) -> Option<IDWriteTypography> {
    unsafe {
        let ty = dw.CreateTypography().ok()?;
        for tag in [DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES.0, ot_tag(b"cv11")] {
            let _ = ty.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG(tag),
                parameter: 1,
            });
        }
        Some(ty)
    }
}

/// A text layout with the shared typography applied to all of it.
fn new_layout(
    dw: &IDWriteFactory,
    ty: Option<&IDWriteTypography>,
    s: &str,
    fmt: &IDWriteTextFormat,
    w: f32,
    h: f32,
) -> Option<IDWriteTextLayout> {
    let t: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let l = dw.CreateTextLayout(&t, fmt, w.max(0.0), h.max(0.0)).ok()?;
        if let Some(ty) = ty {
            let _ = l.SetTypography(
                ty,
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: t.len() as u32,
                },
            );
        }
        Some(l)
    }
}

impl Shared {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut d3d = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[
                    D3D_FEATURE_LEVEL_11_0,
                    D3D_FEATURE_LEVEL_10_1,
                    D3D_FEATURE_LEVEL_10_0,
                ]),
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                Some(&mut ctx),
            )?;
            let d3d: ID3D11Device = d3d.unwrap();
            let ctx: ID3D11DeviceContext = ctx.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let d2f: ID2D1Factory1 = D2D1CreateFactory(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                Some(&D2D1_FACTORY_OPTIONS {
                    debugLevel: D2D1_DEBUG_LEVEL_NONE,
                }),
            )?;
            let d2dev = d2f.CreateDevice(&dxgi)?;
            let dc = d2dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let (inter, font_note) = match load_inter(&dw) {
                Ok(c) => (Some(c), "font: Inter Variable 4.1 (bundled)".to_string()),
                Err(e) => (
                    None,
                    format!("font: Inter failed to load ({e:?}), using Segoe UI Variable"),
                ),
            };
            let typography = make_typography(&dw);
            TEXT.with(|t| *t.borrow_mut() = Some((dw.clone(), typography.clone())));
            Ok(Self {
                d3d,
                ctx,
                factory,
                dcomp,
                d2f,
                dc,
                dw,
                inter,
                typography,
                font_note,
            })
        }
    }

    pub fn swapchain(&self, w: u32, h: u32) -> Result<IDXGISwapChain1> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w.max(1),
            Height: h.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        unsafe {
            self.factory
                .CreateSwapChainForComposition(&self.d3d, &desc, None)
        }
    }

    /// Bind a swap chain to a window. `downscale` < 1 renders at a higher resolution
    /// and lets DirectComposition filter it down (used for the orb's supersampling).
    pub fn bind(
        &self,
        hwnd: HWND,
        swap: &IDXGISwapChain1,
        downscale: f32,
    ) -> Result<(IDCompositionTarget, IDCompositionVisual)> {
        unsafe {
            let target = self.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = self.dcomp.CreateVisual()?;
            visual.SetContent(swap)?;
            if downscale != 1.0 {
                let st = self.dcomp.CreateScaleTransform()?;
                st.SetScaleX2(downscale)?;
                st.SetScaleY2(downscale)?;
                visual.SetTransform(&st)?;
                visual.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
            }
            target.SetRoot(&visual)?;
            self.dcomp.Commit()?;
            Ok((target, visual))
        }
    }

    /// A single-line, vertically centred text format in Inter (Segoe UI Variable if Inter did not
    /// load). Inter's weight and optical size axes are set exactly: `wght` = `weight`, `opsz` =
    /// the size clamped to the font's 14 to 32 range, as the browser does in the mockups.
    pub fn format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        unsafe {
            let (family, coll) = match &self.inter {
                Some((c, family)) => (*family, Some(c)),
                None => (FALLBACK_FAMILY, None),
            };
            let f = self.dw.CreateTextFormat(
                family,
                coll,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            if self.inter.is_some() {
                if let Ok(f3) = f.cast::<IDWriteTextFormat3>() {
                    let _ = f3.SetFontAxisValues(&[
                        DWRITE_FONT_AXIS_VALUE {
                            axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT,
                            value: weight.0 as f32,
                        },
                        DWRITE_FONT_AXIS_VALUE {
                            axisTag: DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE,
                            value: size.clamp(14.0, 32.0),
                        },
                    ]);
                }
            }
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(f)
        }
    }

    /// A layout with tabular figures and cv11 on, like everything `text` draws. Use it instead of
    /// `dw.CreateTextLayout` when measuring, so measured and drawn widths agree.
    pub fn layout(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        w: f32,
        h: f32,
    ) -> Option<IDWriteTextLayout> {
        new_layout(&self.dw, self.typography.as_ref(), s, fmt, w, h)
    }

    pub fn text_width(&self, text: &str, fmt: &IDWriteTextFormat) -> f32 {
        match self.layout(text, fmt, 2000.0, 100.0) {
            Some(l) => unsafe {
                let mut m = DWRITE_TEXT_METRICS::default();
                let _ = l.GetMetrics(&mut m);
                m.widthIncludingTrailingWhitespace
            },
            None => 0.0,
        }
    }
}

/// A Direct2D drawing surface for one window, in logical pixels (D2D DPI = 96 * scale).
pub struct Surface {
    pub swap: IDXGISwapChain1,
    bmp: Option<ID2D1Bitmap1>,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    effect: Option<IDCompositionEffectGroup>,
    opacity: f32,
    pub w: u32,
    pub h: u32,
    scale: f32,
}

impl Surface {
    pub fn new(g: &Shared, hwnd: HWND, w: u32, h: u32, scale: f32) -> Result<Self> {
        let swap = g.swapchain(w, h)?;
        let (t, v) = g.bind(hwnd, &swap, 1.0)?;
        let mut s = Self {
            swap,
            bmp: None,
            _target: t,
            _visual: v,
            effect: None,
            opacity: 1.0,
            w,
            h,
            scale,
        };
        s.make_bitmap(g)?;
        Ok(s)
    }

    fn make_bitmap(&mut self, g: &Shared) -> Result<()> {
        unsafe {
            let surf: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0 * self.scale,
                dpiY: 96.0 * self.scale,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            self.bmp = Some(g.dc.CreateBitmapFromDxgiSurface(&surf, Some(&props))?);
            Ok(())
        }
    }

    pub fn resize(&mut self, g: &Shared, w: u32, h: u32) -> Result<()> {
        if w == self.w && h == self.h {
            return Ok(());
        }
        unsafe {
            g.dc.SetTarget(None);
            self.bmp = None;
            self.swap.ResizeBuffers(
                0,
                w.max(1),
                h.max(1),
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.w = w;
        self.h = h;
        self.make_bitmap(g)
    }

    pub fn begin<'a>(&self, g: &'a Shared) -> &'a ID2D1DeviceContext {
        unsafe {
            let img: Option<ID2D1Image> = self.bmp.as_ref().and_then(|b| b.cast().ok());
            g.dc.SetTarget(img.as_ref());
            g.dc.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            g.dc.BeginDraw();
            g.dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
        }
        &g.dc
    }

    pub fn end(&self, g: &Shared) {
        unsafe {
            let _ = g.dc.EndDraw(None, None);
            g.dc.SetTarget(None);
            let _ = self.swap.Present(1, DXGI_PRESENT(0));
        }
    }

    /// Fade the whole window in the compositor, without redrawing it. 1.0 drops the effect so a
    /// resting window composes as cheaply as before.
    pub fn set_opacity(&mut self, g: &Shared, a: f32) {
        let a = a.clamp(0.0, 1.0);
        if (a - self.opacity).abs() < 0.001 {
            return;
        }
        self.opacity = a;
        unsafe {
            if a >= 0.999 {
                let _ = self._visual.SetEffect(None);
            } else {
                if self.effect.is_none() {
                    self.effect = g.dcomp.CreateEffectGroup().ok();
                }
                if let Some(fx) = &self.effect {
                    let _ = fx.SetOpacity2(a);
                    let _ = self._visual.SetEffect(fx);
                }
            }
            let _ = g.dcomp.Commit();
        }
    }
}

/// Draws into an offscreen bitmap (`w` x `h` logical pixels at `scale`) and returns its pixels as
/// premultiplied RGBA rows: the snapshot mode's renderer, on the same device context as the
/// windows, so a snapshot is exactly what a window would show.
pub fn render_offscreen(
    g: &Shared,
    w: f32,
    h: f32,
    scale: f32,
    draw: impl FnOnce(&ID2D1DeviceContext),
) -> Result<(u32, u32, Vec<u8>)> {
    let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
    unsafe {
        let props = |opts| D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            bitmapOptions: opts,
            ..Default::default()
        };
        let size = D2D_SIZE_U {
            width: pw,
            height: ph,
        };
        let target = g
            .dc
            .CreateBitmap(size, None, 0, &props(D2D1_BITMAP_OPTIONS_TARGET))?;
        let cpu = g.dc.CreateBitmap(
            size,
            None,
            0,
            &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
        )?;
        g.dc.SetTarget(&target);
        g.dc.SetDpi(96.0 * scale, 96.0 * scale);
        g.dc.BeginDraw();
        g.dc.Clear(Some(&D2D1_COLOR_F::default()));
        draw(&g.dc);
        g.dc.EndDraw(None, None)?;
        g.dc.SetTarget(None);
        cpu.CopyFromBitmap(None, &target, None)?;
        let m = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
        let mut out = Vec::with_capacity((pw * ph * 4) as usize);
        for y in 0..ph {
            let row = std::slice::from_raw_parts(m.bits.add((y * m.pitch) as usize), pw as usize * 4);
            for px in row.chunks(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        cpu.Unmap()?;
        Ok((pw, ph, out))
    }
}

// ---- small drawing helpers ----

pub fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

pub fn brush(dc: &ID2D1DeviceContext, c: D2D1_COLOR_F) -> ID2D1SolidColorBrush {
    unsafe { dc.CreateSolidColorBrush(&c, None).unwrap() }
}

pub fn rrect(l: f32, t: f32, r: f32, b: f32, rad: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: l,
            top: t,
            right: r,
            bottom: b,
        },
        radiusX: rad,
        radiusY: rad,
    }
}

pub fn ellipse(cx: f32, cy: f32, r: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: Vector2 { X: cx, Y: cy },
        radiusX: r,
        radiusY: r,
    }
}

pub fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub fn text(
    dc: &ID2D1DeviceContext,
    s: &str,
    fmt: &IDWriteTextFormat,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    b: &ID2D1SolidColorBrush,
) {
    // Through a layout, so the shared typography (tabular figures, cv11) applies everywhere.
    let layout = TEXT.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|(dw, ty)| new_layout(dw, ty.as_ref(), s, fmt, w, h))
    });
    unsafe {
        match layout {
            Some(l) => dc.DrawTextLayout(pt(x, y), &l, b, D2D1_DRAW_TEXT_OPTIONS_NONE),
            None => {
                let t: Vec<u16> = s.encode_utf16().collect();
                let rect = D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                };
                dc.DrawText(
                    &t,
                    fmt,
                    &rect,
                    b,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
    }
}

pub fn center_text(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    s: &str,
    fmt: &IDWriteTextFormat,
    cx: f32,
    y: f32,
    h: f32,
    b: &ID2D1SolidColorBrush,
) {
    let w = g.text_width(s, fmt);
    text(dc, s, fmt, cx - w / 2.0, y, w + 2.0, h, b);
}

pub fn right_text(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    s: &str,
    fmt: &IDWriteTextFormat,
    right: f32,
    y: f32,
    h: f32,
    b: &ID2D1SolidColorBrush,
) {
    let w = g.text_width(s, fmt);
    text(dc, s, fmt, right - w, y, w + 2.0, h, b);
}

pub fn round_stroke(g: &Shared) -> ID2D1StrokeStyle {
    unsafe {
        let p = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
            transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        };
        g.d2f.CreateStrokeStyle(&p, None).unwrap().into()
    }
}

fn stroke_with(g: &Shared, dashes: &[f32], cap: D2D1_CAP_STYLE) -> ID2D1StrokeStyle {
    unsafe {
        let p = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: cap,
            endCap: cap,
            dashCap: cap,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_CUSTOM,
            dashOffset: 0.0,
            transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        };
        g.d2f.CreateStrokeStyle(&p, Some(dashes)).unwrap().into()
    }
}

/// The dotted time track: at 1.5 px, round dots every 6 px (dash lengths are in stroke widths).
pub fn dotted_stroke(g: &Shared) -> ID2D1StrokeStyle {
    stroke_with(g, &[0.0, 4.0], D2D1_CAP_STYLE_ROUND)
}

/// A flat dashed rule (the drink goal line): 3 px on, 3 px off at 1 px.
pub fn dashed_stroke(g: &Shared) -> ID2D1StrokeStyle {
    stroke_with(g, &[3.0, 3.0], D2D1_CAP_STYLE_FLAT)
}

/// A point on a circle, `deg` clockwise from 12 o'clock.
pub fn on_circle(cx: f32, cy: f32, r: f32, deg: f32) -> Vector2 {
    let a = deg.to_radians();
    pt(cx + r * a.sin(), cy - r * a.cos())
}

/// A circular arc clockwise from `from` to `to` degrees (from 12 o'clock); a full turn is a circle.
pub fn draw_arc(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    cx: f32,
    cy: f32,
    r: f32,
    from: f32,
    to: f32,
    b: &ID2D1SolidColorBrush,
    w: f32,
    st: &ID2D1StrokeStyle,
) {
    let sweep = to - from;
    if sweep <= 0.01 {
        return;
    }
    unsafe {
        if sweep >= 359.9 {
            dc.DrawEllipse(&ellipse(cx, cy, r), b, w, st);
            return;
        }
        let geo = path(g, on_circle(cx, cy, r, from), false, |k| {
            arc_to(k, on_circle(cx, cy, r, to), r, sweep > 180.0, true)
        });
        dc.DrawGeometry(&geo, b, w, st);
    }
}

/// A path made of line segments and circular arcs, built with a small closure.
pub fn path(
    g: &Shared,
    start: Vector2,
    filled: bool,
    build: impl FnOnce(&ID2D1GeometrySink),
) -> ID2D1PathGeometry {
    unsafe {
        let geo = g.d2f.CreatePathGeometry().unwrap();
        let sink = geo.Open().unwrap();
        sink.BeginFigure(
            start,
            if filled {
                D2D1_FIGURE_BEGIN_FILLED
            } else {
                D2D1_FIGURE_BEGIN_HOLLOW
            },
        );
        build(&sink);
        sink.EndFigure(if filled {
            D2D1_FIGURE_END_CLOSED
        } else {
            D2D1_FIGURE_END_OPEN
        });
        let _ = sink.Close();
        geo.into()
    }
}

pub fn arc_to(sink: &ID2D1GeometrySink, to: Vector2, r: f32, large: bool, cw: bool) {
    unsafe {
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: to,
            size: D2D_SIZE_F {
                width: r,
                height: r,
            },
            rotationAngle: 0.0,
            sweepDirection: if cw {
                D2D1_SWEEP_DIRECTION_CLOCKWISE
            } else {
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
            },
            arcSize: if large {
                D2D1_ARC_SIZE_LARGE
            } else {
                D2D1_ARC_SIZE_SMALL
            },
        });
    }
}

pub fn line_to(sink: &ID2D1GeometrySink, to: Vector2) {
    unsafe { sink.AddLine(to) }
}

pub fn quad_to(sink: &ID2D1GeometrySink, ctrl: Vector2, to: Vector2) {
    unsafe {
        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
            point1: ctrl,
            point2: to,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_inter_loads_from_memory_with_tabular_figures() {
        // The embedded file is the variable TrueType (version 1.0 sfnt header).
        assert_eq!(&INTER_TTF[..4], &[0, 1, 0, 0]);
        unsafe {
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let (coll, family) = load_inter(&dw).expect("Inter loads through the in-memory loader");
            let f = dw
                .CreateTextFormat(
                    family,
                    &coll,
                    DWRITE_FONT_WEIGHT_MEDIUM,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    13.0,
                    w!("en-us"),
                )
                .unwrap();
            let ty = make_typography(&dw);
            assert!(ty.is_some());
            // With tnum on, "1111" and "0000" are exactly as wide.
            let width = |s: &str| {
                let l = new_layout(&dw, ty.as_ref(), s, &f, 500.0, 50.0).unwrap();
                let mut m = DWRITE_TEXT_METRICS::default();
                l.GetMetrics(&mut m).unwrap();
                m.width
            };
            assert!((width("1111") - width("0000")).abs() < 0.01);
            assert_eq!(ot_tag(b"tnum"), DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES.0);
        }
    }
}
