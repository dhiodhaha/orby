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

pub struct Shared {
    pub d3d: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub factory: IDXGIFactory2,
    pub dcomp: IDCompositionDevice,
    pub d2f: ID2D1Factory1,
    pub dc: ID2D1DeviceContext,
    pub dw: IDWriteFactory,
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
            Ok(Self {
                d3d,
                ctx,
                factory,
                dcomp,
                d2f,
                dc,
                dw,
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

    pub fn format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        unsafe {
            let f = self.dw.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(f)
        }
    }

    pub fn text_width(&self, text: &str, fmt: &IDWriteTextFormat) -> f32 {
        let t: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            match self.dw.CreateTextLayout(&t, fmt, 2000.0, 100.0) {
                Ok(l) => {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = l.GetMetrics(&mut m);
                    m.widthIncludingTrailingWhitespace
                }
                Err(_) => 0.0,
            }
        }
    }
}

/// A Direct2D drawing surface for one window, in logical pixels (D2D DPI = 96 * scale).
pub struct Surface {
    pub swap: IDXGISwapChain1,
    bmp: Option<ID2D1Bitmap1>,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
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
    let t: Vec<u16> = s.encode_utf16().collect();
    let rect = D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    unsafe {
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
