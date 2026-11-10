#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! Pill and dial drawing on the v6.3 tokens: true neutral, Inter Variable (bundled, gfx.rs) with
//! tabular figures everywhere, and the shared number column helper `text_num`.

#[cfg(windows)]
use windows::Win32::Graphics::Direct2D::*;
#[cfg(windows)]
use windows::Win32::Graphics::DirectWrite::*;

use crate::canvas::*;
#[cfg(windows)]
use crate::gfx::{self, Shared};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub surface: Color,
    pub band: Color,
    pub hair: Color,
    pub text: Color,
    pub muted: Color,
    pub chip: Color,
    pub button: Color,
    pub button_hover: Color,
    pub panel: Color,
    pub raised: Color,
    pub track: Color,
    pub passed: Color,
    /// Drink water: moonlight white in dark, a soft grey in white. Never blue.
    pub water: Color,
    pub meniscus: Color,
    /// Needs you: failures only, never offline. The same in both themes.
    pub ember: Color,
    /// The History grid's five levels, nothing logged to goal met (or 2 h of focus).
    pub heat: [Color; 5],
}

/// Whether Windows draws apps dark (Settings, Personalisation, Colours, app mode).
#[cfg(windows)]
pub fn windows_dark() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::*;
    let mut v: u32 = 0;
    let mut size: u32 = 4;
    let light = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut size),
        )
        .is_ok()
            && v == 1
    };
    !light
}

pub fn theme(dark: bool) -> Theme {
    if !dark {
        Theme {
            dark: false,
            surface: rgba(0xFCFCFC, 1.0),
            band: rgba(0xFCFCFC, 1.0),
            hair: rgba(0x000000, 0.09),
            text: rgba(0x171717, 1.0),
            muted: rgba(0x6E6E6E, 1.0),
            chip: rgba(0x000000, 0.05),
            button: rgba(0x000000, 0.05),
            button_hover: rgba(0x000000, 0.09),
            panel: rgba(0xFCFCFC, 1.0),
            raised: rgba(0x000000, 0.035),
            track: rgba(0xD4D4D4, 1.0),
            passed: rgba(0xA3A3A3, 1.0),
            water: rgba(0xE5E5E5, 1.0),
            meniscus: rgba(0xA3A3A3, 1.0),
            ember: rgba(0xE2614F, 1.0),
            heat: [
                rgba(0xEEEEEE, 1.0),
                rgba(0xD4D4D4, 1.0),
                rgba(0xA3A3A3, 1.0),
                rgba(0x6E6E6E, 1.0),
                rgba(0x171717, 1.0),
            ],
        }
    } else {
        Theme {
            dark: true,
            surface: rgba(0x1F1F1F, 1.0),
            band: rgba(0x171717, 1.0),
            hair: rgba(0xFFFFFF, 0.09),
            text: rgba(0xFAFAFA, 1.0),
            muted: rgba(0x9E9E9E, 1.0),
            chip: rgba(0xFFFFFF, 0.07),
            button: rgba(0xFFFFFF, 0.07),
            button_hover: rgba(0xFFFFFF, 0.13),
            panel: rgba(0x171717, 1.0),
            raised: rgba(0xFFFFFF, 0.05),
            track: rgba(0x3A3A3A, 1.0),
            passed: rgba(0x6B6B6B, 1.0),
            water: rgba(0xEDEDED, 1.0),
            meniscus: rgba(0xFFFFFF, 1.0),
            ember: rgba(0xE2614F, 1.0),
            heat: [
                rgba(0x242424, 1.0),
                rgba(0x3A3A3A, 1.0),
                rgba(0x6B6B6B, 1.0),
                rgba(0x9E9E9E, 1.0),
                rgba(0xFAFAFA, 1.0),
            ],
        }
    }
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

pub struct Fonts {
    pub title: Font,
    pub body: Font,
    pub small: Font,
    pub small_medium: Font,
    pub panel_title: Font,
    pub big: Font,
    pub timer: Font,
    pub heading: Font,
    /// The orb badge's digits.
    pub badge: Font,
    /// Wrapping body text, top aligned (the Disk cards and the confirm sheet).
    pub small_wrap: Font,
    pub body_wrap: Font,
    /// 11/500: the Disk tags and the keep column.
    pub tag: Font,
}

#[cfg(windows)]
fn wrapping(f: Font) -> Font {
    unsafe {
        let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
    }
    f
}

impl Fonts {
    #[cfg(windows)]
    pub fn new(g: &Shared) -> Self {
        Self {
            title: g.format(13.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            body: g.format(13.0, DWRITE_FONT_WEIGHT_NORMAL).unwrap(),
            small: g.format(12.0, DWRITE_FONT_WEIGHT_NORMAL).unwrap(),
            small_medium: g.format(12.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            panel_title: g.format(14.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            big: g.format(26.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            timer: g.format(40.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            heading: g.format(20.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            badge: g.format(10.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
            small_wrap: wrapping(g.format(12.0, DWRITE_FONT_WEIGHT_NORMAL).unwrap()),
            body_wrap: wrapping(g.format(13.0, DWRITE_FONT_WEIGHT_NORMAL).unwrap()),
            tag: g.format(11.0, DWRITE_FONT_WEIGHT_MEDIUM).unwrap(),
        }
    }

    /// The same sizes and weights in the bundled Inter, through Core Text.
    #[cfg(target_os = "macos")]
    pub fn new() -> Self {
        use crate::mac::text::font;
        Self {
            title: font(13.0, 500.0, false),
            body: font(13.0, 400.0, false),
            small: font(12.0, 400.0, false),
            small_medium: font(12.0, 500.0, false),
            panel_title: font(14.0, 500.0, false),
            big: font(26.0, 500.0, false),
            timer: font(40.0, 500.0, false),
            heading: font(20.0, 500.0, false),
            badge: font(10.0, 500.0, false),
            small_wrap: font(12.0, 400.0, true),
            body_wrap: font(13.0, 400.0, true),
            tag: font(11.0, 500.0, false),
        }
    }
}

// ---- numbers ----

/// The frame's percent column: 44 px, 12/16 muted, right aligned.
#[allow(dead_code)] // for the Disk lists (another module owns them)
pub const NUM_COL_PCT: f32 = 44.0;
/// The frame's size column: 64 px, 13/16 500, right aligned.
#[allow(dead_code)] // for the Disk lists (another module owns them)
pub const NUM_COL_SIZE: f32 = 64.0;
/// The gap between number columns.
pub const NUM_COL_GAP: f32 = 10.0;

/// Draws a number (a size, time, count or percent) with its right edge at `right`, so digits line
/// up row to row. All text has tabular figures on (gfx.rs applies `tnum` to every layout), so a
/// right-aligned column is all a number needs. Use one of the `NUM_COL_*` widths to lay out the
/// column; for a value that should roll when it changes, use `motion::rolling_text_right`.
///
/// `text_num(g, dc, "182 GB", &f.title, right, y, h, &ink)`
#[cfg(windows)]
pub fn text_num(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    s: &str,
    fmt: &IDWriteTextFormat,
    right: f32,
    y: f32,
    h: f32,
    b: &ID2D1SolidColorBrush,
) {
    gfx::right_text(g, dc, s, fmt, right, y, h, b);
}

/// `text_num` on the portable `Canvas`: a number with its right edge at `right`.
pub fn num_text(cv: &mut dyn Canvas, s: &str, f: &Font, right: f32, y: f32, h: f32, c: Color) {
    cv.right_text(s, f, right, y, h, c);
}

// ---- icons (Lucide-style, drawn on a 24 grid, stroke 1.6) ----

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Tasks,
    Today,
    Prayer,
    Focus,
    Drink,
    More,
    Bell,
    Info,
    Close,
    /// The title band's maximize button: one rounded square.
    Maximize,
    /// The same button on a maximized window: two squares, one behind the other.
    Restore,
    Pause,
    Play,
    Skip,
    Stop,
    Clock,
    /// A rounded square: the "due now" checkbox chip.
    Check,
    Inbox,
    Sun,
    Person,
    People,
    Lock,
    Folder,
    List,
    Plus,
    ChevronRight,
    ChevronDown,
    Repeat,
    Link,
    CloudOff,
    /// A camera: an event with a call link.
    Video,
    /// A film frame with a play mark: a video download.
    Film,
    Music,
    Download,
    /// A tick on its own (a done download).
    Tick,
    /// Ember icons: a failed download. The pill draws them on an ember chip.
    Alert,
    Private,
    Drive,
    /// The same drive in ink: a drive that is fine (low space, the Disk panel).
    HardDrive,
    Trash,
    Refresh,
    Shield,
    /// Three sliders: System files (Storage settings).
    Sliders,
    /// Stacked sheets: temp files.
    Layers,
    Globe,
    /// A box with a lid: archives in the Files list.
    Archive,
    File,
    /// Open in Explorer: a box with an arrow out of its corner.
    External,
    ArrowLeft,
    Search,
    ChevronLeft,
    /// Four rounded squares: History.
    Grid,
    /// A map pin: the place for prayer times.
    Pin,
    Keyboard,
    /// A cog: Settings.
    Gear,
}

impl Icon {
    pub fn is_ember(self) -> bool {
        matches!(self, Icon::Alert | Icon::Private | Icon::Drive)
    }
}

/// A 28 px title-band button (close, maximize): no fill until hovered, a 14 px muted icon.
pub fn paint_title_button(
    cv: &mut dyn Canvas,
    t: &Theme,
    r: (f32, f32, f32, f32),
    icon: Icon,
    hover: bool,
) {
    if hover {
        cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), t.button_hover);
    }
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    paint_icon(cv, icon, cx, cy, 14.0, t.muted);
}

/// `paint_icon` on a Direct2D context, for the panels that still draw with Direct2D.
#[cfg(windows)]
pub fn draw_icon(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    icon: Icon,
    cx: f32,
    cy: f32,
    size: f32,
    b: &ID2D1SolidColorBrush,
) {
    let c = unsafe { b.GetColor() };
    paint_icon(&mut gfx::D2dCanvas::new(g, dc), icon, cx, cy, size, c);
}

/// The lip's colour: white, black 4% at the bottom; dark, white 6% at the top.
pub fn lip_color(t: &Theme) -> Color {
    if t.dark {
        rgba(0xFFFFFF, 0.06)
    } else {
        rgba(0x000000, 0.04)
    }
}

/// The T3 lip on a rounded rect: a one-pixel crescent along the inside of the bottom edge
/// (white) or the top edge (dark), following the corners. `a` scales its alpha.
pub fn lip(cv: &mut dyn Canvas, t: &Theme, l: f32, top: f32, r: f32, b: f32, radius: f32, a: f32) {
    if a <= 0.01 || r - l < 2.0 || b - top < 2.0 {
        return;
    }
    let c = lip_color(t);
    let dy = if t.dark { 1.0 } else { -1.0 };
    cv.lip(Color { a: c.a * a, ..c }, dy, l, top, r, b, radius);
}

/// A Lucide-style icon on the 24 grid, `size` px, stroke 1.6 at 16 px.
pub fn paint_icon(cv: &mut dyn Canvas, icon: Icon, cx: f32, cy: f32, size: f32, b: Color) {
    let s = size / 24.0;
    let p = |x: f32, y: f32| pt(cx + (x - 12.0) * s, cy + (y - 12.0) * s);
    let sw = 1.6 * (size / 16.0);
    let st = Stroke::Round;
    {
        let line = |cv: &mut dyn Canvas, a: (f32, f32), c: (f32, f32)| {
            cv.line(p(a.0, a.1), p(c.0, c.1), b, sw, st)
        };
        match icon {
            Icon::Tasks => {
                let r = rrect(
                    cx - 8.0 * s,
                    cy - 8.0 * s,
                    cx + 8.0 * s,
                    cy + 8.0 * s,
                    5.0 * s,
                );
                cv.stroke_rrect(&r, b, sw, st);
                let geo = path(p(8.5, 12.0), false, |k| {
                    line_to(k, p(11.0, 14.5));
                    line_to(k, p(15.5, 9.5));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Today => {
                let r = rrect(
                    cx - 9.0 * s,
                    cy - 7.0 * s,
                    cx + 9.0 * s,
                    cy + 9.0 * s,
                    3.0 * s,
                );
                cv.stroke_rrect(&r, b, sw, st);
                line(cv, (3.0, 10.0), (21.0, 10.0));
                line(cv, (8.0, 3.0), (8.0, 7.0));
                line(cv, (16.0, 3.0), (16.0, 7.0));
            }
            Icon::Prayer => {
                // A small day arc with the sun on it, echoing the prayer arc.
                let geo = path(p(4.0, 17.0), false, |k| {
                    arc_to(k, p(20.0, 17.0), 8.0 * s, false, true)
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (2.5, 17.0), (21.5, 17.0));
                cv.fill_ellipse(&ellipse(p(15.4, 9.8).X, p(15.4, 9.8).Y, 2.2 * s), b);
            }
            Icon::Focus => {
                cv.stroke_ellipse(&ellipse(cx, cy + 1.0 * s, 8.0 * s), b, sw, st);
                let geo = path(p(12.0, 9.0), false, |k| {
                    line_to(k, p(12.0, 13.0));
                    line_to(k, p(14.5, 15.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (10.0, 2.5), (14.0, 2.5));
            }
            Icon::Drink => {
                let geo = path(p(5.0, 4.0), false, |k| {
                    line_to(k, p(19.0, 4.0));
                    line_to(k, p(17.4, 19.2));
                    line_to(k, p(6.6, 19.2));
                    line_to(k, p(5.0, 4.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (5.7, 10.0), (18.3, 10.0));
            }
            Icon::More => {
                for x in [6.0, 12.0, 18.0] {
                    let q = p(x, 12.0);
                    cv.fill_ellipse(&ellipse(q.X, q.Y, 1.7 * s), b);
                }
            }
            Icon::Bell => {
                let geo = path(p(6.0, 13.0), false, |k| {
                    line_to(k, p(6.0, 9.0));
                    arc_to(k, p(18.0, 9.0), 6.0 * s, false, true);
                    line_to(k, p(18.0, 13.0));
                    line_to(k, p(20.5, 17.0));
                    line_to(k, p(3.5, 17.0));
                    line_to(k, p(6.0, 13.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (10.3, 20.5), (13.7, 20.5));
            }
            Icon::Info => {
                cv.stroke_ellipse(&ellipse(cx, cy, 9.0 * s), b, sw, st);
                line(cv, (12.0, 11.0), (12.0, 16.0));
                let q = p(12.0, 7.8);
                cv.fill_ellipse(&ellipse(q.X, q.Y, 1.2 * s), b);
            }
            Icon::Close => {
                line(cv, (7.0, 7.0), (17.0, 17.0));
                line(cv, (17.0, 7.0), (7.0, 17.0));
            }
            Icon::Maximize => {
                let (a, z) = (p(6.5, 6.5), p(17.5, 17.5));
                cv.stroke_rrect(&rrect(a.X, a.Y, z.X, z.Y, 2.5 * s), b, sw, st);
            }
            Icon::Restore => {
                // The front window, and the edge of the one behind it.
                let (a, z) = (p(5.5, 9.0), p(15.0, 18.5));
                cv.stroke_rrect(&rrect(a.X, a.Y, z.X, z.Y, 2.2 * s), b, sw, st);
                let geo = path(p(9.0, 5.5), false, |k| {
                    line_to(k, p(16.3, 5.5));
                    arc_to(k, p(18.5, 7.7), 2.2 * s, false, true);
                    line_to(k, p(18.5, 15.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Pause => {
                line(cv, (9.0, 6.5), (9.0, 17.5));
                line(cv, (15.0, 6.5), (15.0, 17.5));
            }
            Icon::Play => {
                let geo = path(p(8.0, 6.0), false, |k| {
                    line_to(k, p(18.0, 12.0));
                    line_to(k, p(8.0, 18.0));
                    line_to(k, p(8.0, 6.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Skip => {
                let geo = path(p(6.0, 6.5), false, |k| {
                    line_to(k, p(14.5, 12.0));
                    line_to(k, p(6.0, 17.5));
                    line_to(k, p(6.0, 6.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (18.0, 6.5), (18.0, 17.5));
            }
            Icon::Stop => {
                cv.stroke_rrect(
                    &rrect(
                        cx - 6.0 * s,
                        cy - 6.0 * s,
                        cx + 6.0 * s,
                        cy + 6.0 * s,
                        2.0 * s,
                    ),
                    b,
                    sw,
                    st,
                );
            }
            Icon::Clock => {
                cv.stroke_ellipse(&ellipse(cx, cy, 9.0 * s), b, sw, st);
                let geo = path(p(12.0, 7.5), false, |k| {
                    line_to(k, p(12.0, 12.0));
                    line_to(k, p(15.0, 14.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Check => {
                cv.stroke_rrect(
                    &rrect(
                        cx - 7.0 * s,
                        cy - 7.0 * s,
                        cx + 7.0 * s,
                        cy + 7.0 * s,
                        3.0 * s,
                    ),
                    b,
                    sw,
                    st,
                );
            }
            Icon::Inbox => {
                let geo = path(p(3.5, 12.5), false, |k| {
                    line_to(k, p(8.0, 12.5));
                    line_to(k, p(9.5, 15.0));
                    line_to(k, p(14.5, 15.0));
                    line_to(k, p(16.0, 12.5));
                    line_to(k, p(20.5, 12.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                let box_ = path(p(6.5, 5.0), false, |k| {
                    line_to(k, p(17.5, 5.0));
                    line_to(k, p(20.5, 12.5));
                    line_to(k, p(20.5, 19.0));
                    line_to(k, p(3.5, 19.0));
                    line_to(k, p(3.5, 12.5));
                    line_to(k, p(6.5, 5.0));
                });
                cv.stroke_path(&box_, b, sw, st);
            }
            Icon::Sun => {
                cv.stroke_ellipse(&ellipse(cx, cy, 4.0 * s), b, sw, st);
                for i in 0..8 {
                    let a = (i as f32 * 45.0).to_radians();
                    let (dx, dy) = (a.sin(), -a.cos());
                    line(
                        cv,
                        (12.0 + dx * 7.0, 12.0 + dy * 7.0),
                        (12.0 + dx * 9.0, 12.0 + dy * 9.0),
                    );
                }
            }
            Icon::Person => {
                cv.stroke_ellipse(&ellipse(cx, cy - 4.0 * s, 3.5 * s), b, sw, st);
                let geo = path(p(5.5, 20.0), false, |k| {
                    arc_to(k, p(18.5, 20.0), 6.5 * s, false, true)
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::People => {
                cv.stroke_ellipse(&ellipse(p(9.0, 8.5).X, p(9.0, 8.5).Y, 3.0 * s), b, sw, st);
                let geo = path(p(3.5, 19.0), false, |k| {
                    arc_to(k, p(14.5, 19.0), 5.5 * s, false, true)
                });
                cv.stroke_path(&geo, b, sw, st);
                let geo = path(p(15.5, 5.8), false, |k| {
                    arc_to(k, p(15.5, 11.2), 2.7 * s, false, true)
                });
                cv.stroke_path(&geo, b, sw, st);
                let geo = path(p(17.5, 14.0), false, |k| {
                    arc_to(k, p(20.5, 19.0), 5.0 * s, false, true)
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Lock | Icon::Private => {
                cv.stroke_rrect(
                    &rrect(
                        cx - 7.0 * s,
                        cy - 1.0 * s,
                        cx + 7.0 * s,
                        cy + 8.0 * s,
                        2.0 * s,
                    ),
                    b,
                    sw,
                    st,
                );
                let geo = path(p(8.0, 11.0), false, |k| {
                    line_to(k, p(8.0, 8.0));
                    arc_to(k, p(16.0, 8.0), 4.0 * s, false, true);
                    line_to(k, p(16.0, 11.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Folder => {
                let geo = path(p(3.5, 6.5), false, |k| {
                    line_to(k, p(9.5, 6.5));
                    line_to(k, p(11.5, 8.5));
                    line_to(k, p(20.5, 8.5));
                    line_to(k, p(20.5, 18.5));
                    line_to(k, p(3.5, 18.5));
                    line_to(k, p(3.5, 6.5));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::List => {
                for y in [7.0, 12.0, 17.0] {
                    line(cv, (9.0, y), (20.0, y));
                    let q = p(4.8, y);
                    cv.fill_ellipse(&ellipse(q.X, q.Y, 1.2 * s), b);
                }
            }
            Icon::Plus => {
                line(cv, (12.0, 5.0), (12.0, 19.0));
                line(cv, (5.0, 12.0), (19.0, 12.0));
            }
            Icon::ChevronRight => {
                let geo = path(p(10.0, 7.0), false, |k| {
                    line_to(k, p(15.0, 12.0));
                    line_to(k, p(10.0, 17.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::ChevronLeft => {
                let geo = path(p(14.0, 7.0), false, |k| {
                    line_to(k, p(9.0, 12.0));
                    line_to(k, p(14.0, 17.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Grid => {
                for (x, y) in [(4.0, 4.0), (13.5, 4.0), (4.0, 13.5), (13.5, 13.5)] {
                    let (a, z) = (p(x, y), p(x + 6.5, y + 6.5));
                    cv.stroke_rrect(&rrect(a.X, a.Y, z.X, z.Y, 1.8 * s), b, sw, st);
                }
            }
            Icon::ChevronDown => {
                let geo = path(p(7.0, 10.0), false, |k| {
                    line_to(k, p(12.0, 15.0));
                    line_to(k, p(17.0, 10.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Repeat => {
                let geo = path(p(5.0, 11.0), false, |k| {
                    line_to(k, p(5.0, 9.0));
                    arc_to(k, p(8.0, 6.0), 3.0 * s, false, true);
                    line_to(k, p(19.0, 6.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (16.0, 3.0), (19.0, 6.0));
                line(cv, (16.0, 9.0), (19.0, 6.0));
                let geo = path(p(19.0, 13.0), false, |k| {
                    line_to(k, p(19.0, 15.0));
                    arc_to(k, p(16.0, 18.0), 3.0 * s, false, true);
                    line_to(k, p(5.0, 18.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (8.0, 15.0), (5.0, 18.0));
                line(cv, (8.0, 21.0), (5.0, 18.0));
            }
            Icon::Link => {
                let r1 = rrect(
                    p(3.0, 9.0).X,
                    p(3.0, 9.0).Y,
                    p(13.0, 15.0).X,
                    p(13.0, 15.0).Y,
                    3.0 * s,
                );
                let r2 = rrect(
                    p(11.0, 9.0).X,
                    p(11.0, 9.0).Y,
                    p(21.0, 15.0).X,
                    p(21.0, 15.0).Y,
                    3.0 * s,
                );
                cv.stroke_rrect(&r1, b, sw, st);
                cv.stroke_rrect(&r2, b, sw, st);
            }
            Icon::CloudOff => {
                let geo = path(p(7.0, 18.0), false, |k| {
                    arc_to(k, p(7.0, 11.0), 3.5 * s, false, true);
                    arc_to(k, p(17.0, 10.0), 5.2 * s, false, true);
                    arc_to(k, p(17.0, 18.0), 4.0 * s, false, true);
                    line_to(k, p(7.0, 18.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (4.0, 4.0), (20.0, 20.0));
            }
            Icon::Video => {
                let (a, c) = (p(2.0, 6.0), p(16.0, 18.0));
                cv.stroke_rrect(&rrect(a.X, a.Y, c.X, c.Y, 2.0 * s), b, sw, st);
                let geo = path(p(16.0, 10.5), false, |k| {
                    line_to(k, p(21.5, 7.5));
                    line_to(k, p(21.5, 16.5));
                    line_to(k, p(16.0, 13.5));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Film => {
                let (a, c) = (p(3.0, 5.0), p(21.0, 19.0));
                cv.stroke_rrect(&rrect(a.X, a.Y, c.X, c.Y, 3.0 * s), b, sw, st);
                let geo = path(p(10.0, 9.0), false, |k| {
                    line_to(k, p(15.0, 12.0));
                    line_to(k, p(10.0, 15.0));
                    line_to(k, p(10.0, 9.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Music => {
                let geo = path(p(9.0, 18.0), false, |k| {
                    line_to(k, p(9.0, 5.0));
                    line_to(k, p(20.0, 3.0));
                    line_to(k, p(20.0, 16.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                for (x, y) in [(6.0, 18.0), (17.0, 16.0)] {
                    let q = p(x, y);
                    cv.stroke_ellipse(&ellipse(q.X, q.Y, 3.0 * s), b, sw, st);
                }
            }
            Icon::Download => {
                line(cv, (12.0, 4.0), (12.0, 15.0));
                let geo = path(p(7.0, 10.5), false, |k| {
                    line_to(k, p(12.0, 15.5));
                    line_to(k, p(17.0, 10.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (5.0, 20.0), (19.0, 20.0));
            }
            Icon::Tick => {
                let geo = path(p(5.0, 12.5), false, |k| {
                    line_to(k, p(10.0, 17.0));
                    line_to(k, p(19.0, 7.5));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Alert => {
                cv.stroke_ellipse(&ellipse(cx, cy, 9.0 * s), b, sw, st);
                line(cv, (12.0, 7.5), (12.0, 12.8));
                let q = p(12.0, 16.2);
                cv.fill_ellipse(&ellipse(q.X, q.Y, 1.2 * s), b);
            }
            Icon::Trash => {
                line(cv, (4.0, 6.5), (20.0, 6.5));
                let geo = path(p(9.0, 6.5), false, |k| {
                    line_to(k, p(9.0, 4.0));
                    line_to(k, p(15.0, 4.0));
                    line_to(k, p(15.0, 6.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                let geo = path(p(6.0, 6.5), false, |k| {
                    line_to(k, p(7.0, 20.0));
                    line_to(k, p(17.0, 20.0));
                    line_to(k, p(18.0, 6.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (10.0, 10.5), (10.0, 16.0));
                line(cv, (14.0, 10.5), (14.0, 16.0));
            }
            Icon::Refresh => {
                let geo = path(p(19.5, 12.0), false, |k| {
                    arc_to(k, p(7.0, 6.4), 7.5 * s, true, true)
                });
                cv.stroke_path(&geo, b, sw, st);
                let geo = path(p(6.5, 3.0), false, |k| {
                    line_to(k, p(6.5, 7.0));
                    line_to(k, p(10.5, 7.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Shield => {
                let geo = path(p(12.0, 3.0), false, |k| {
                    line_to(k, p(19.0, 6.0));
                    line_to(k, p(19.0, 11.0));
                    quad_to(k, p(19.0, 18.0), p(12.0, 21.0));
                    quad_to(k, p(5.0, 18.0), p(5.0, 11.0));
                    line_to(k, p(5.0, 6.0));
                    line_to(k, p(12.0, 3.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Sliders => {
                for (x, knob) in [(6.0, 15.0), (12.0, 8.0), (18.0, 13.0)] {
                    line(cv, (x, 4.0), (x, knob - 2.0));
                    line(cv, (x, knob + 2.0), (x, 20.0));
                    line(cv, (x - 2.5, knob - 2.0), (x + 2.5, knob - 2.0));
                    line(cv, (x - 2.5, knob + 2.0), (x + 2.5, knob + 2.0));
                }
            }
            Icon::Gear => {
                // Eight teeth with flat tops and an open hub, closed by hand so the stroke has no seam.
                let pts: Vec<(f32, f32)> = (0..8)
                    .flat_map(|i| {
                        let a = i as f32 * 45.0;
                        [
                            (7.2, a - 19.0),
                            (9.6, a - 9.0),
                            (9.6, a + 9.0),
                            (7.2, a + 19.0),
                        ]
                    })
                    .map(|(r, d): (f32, f32)| {
                        let t = d.to_radians();
                        (12.0 + r * t.sin(), 12.0 - r * t.cos())
                    })
                    .collect();
                let geo = path(p(pts[0].0, pts[0].1), false, |k| {
                    for &(x, y) in pts.iter().skip(1) {
                        line_to(k, p(x, y));
                    }
                    line_to(k, p(pts[0].0, pts[0].1));
                });
                cv.stroke_path(&geo, b, sw, st);
                cv.stroke_ellipse(&ellipse(cx, cy, 3.0 * s), b, sw, st);
            }
            Icon::Layers => {
                let geo = path(p(12.0, 3.5), false, |k| {
                    line_to(k, p(21.0, 8.0));
                    line_to(k, p(12.0, 12.5));
                    line_to(k, p(3.0, 8.0));
                    line_to(k, p(12.0, 3.5));
                });
                cv.stroke_path(&geo, b, sw, st);
                for y in [12.5, 16.5] {
                    let geo = path(p(3.0, y), false, |k| {
                        line_to(k, p(12.0, y + 4.5));
                        line_to(k, p(21.0, y));
                    });
                    cv.stroke_path(&geo, b, sw, st);
                }
            }
            Icon::Globe => {
                cv.stroke_ellipse(&ellipse(cx, cy, 9.0 * s), b, sw, st);
                cv.stroke_ellipse(
                    &Ellipse {
                        point: pt(cx, cy),
                        rx: 3.8 * s,
                        ry: 9.0 * s,
                    },
                    b,
                    sw,
                    st,
                );
                line(cv, (3.0, 12.0), (21.0, 12.0));
            }
            Icon::Archive => {
                let (a, c) = (p(3.0, 4.5), p(21.0, 9.0));
                cv.stroke_rrect(&rrect(a.X, a.Y, c.X, c.Y, 1.5 * s), b, sw, st);
                let geo = path(p(5.0, 9.0), false, |k| {
                    line_to(k, p(5.0, 19.5));
                    line_to(k, p(19.0, 19.5));
                    line_to(k, p(19.0, 9.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (10.0, 13.0), (14.0, 13.0));
            }
            Icon::File => {
                let geo = path(p(6.0, 3.0), false, |k| {
                    line_to(k, p(14.0, 3.0));
                    line_to(k, p(19.0, 8.0));
                    line_to(k, p(19.0, 21.0));
                    line_to(k, p(6.0, 21.0));
                    line_to(k, p(6.0, 3.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                let geo = path(p(14.0, 3.0), false, |k| {
                    line_to(k, p(14.0, 8.0));
                    line_to(k, p(19.0, 8.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::External => {
                let geo = path(p(11.0, 5.0), false, |k| {
                    line_to(k, p(5.0, 5.0));
                    line_to(k, p(5.0, 19.0));
                    line_to(k, p(19.0, 19.0));
                    line_to(k, p(19.0, 13.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                line(cv, (11.0, 13.0), (20.0, 4.0));
                let geo = path(p(14.5, 4.0), false, |k| {
                    line_to(k, p(20.0, 4.0));
                    line_to(k, p(20.0, 9.5));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::ArrowLeft => {
                line(cv, (19.0, 12.0), (5.0, 12.0));
                let geo = path(p(11.0, 6.0), false, |k| {
                    line_to(k, p(5.0, 12.0));
                    line_to(k, p(11.0, 18.0));
                });
                cv.stroke_path(&geo, b, sw, st);
            }
            Icon::Search => {
                let c0 = p(10.5, 10.5);
                cv.stroke_ellipse(&ellipse(c0.X, c0.Y, 6.0 * s), b, sw, st);
                line(cv, (15.0, 15.0), (20.0, 20.0));
            }
            Icon::Pin => {
                let geo = path(p(12.0, 21.0), false, |k| {
                    quad_to(k, p(5.0, 14.0), p(5.0, 10.0));
                    arc_to(k, p(19.0, 10.0), 7.0 * s, false, true);
                    quad_to(k, p(19.0, 14.0), p(12.0, 21.0));
                });
                cv.stroke_path(&geo, b, sw, st);
                cv.stroke_ellipse(
                    &ellipse(p(12.0, 10.0).X, p(12.0, 10.0).Y, 2.5 * s),
                    b,
                    sw,
                    st,
                );
            }
            Icon::Keyboard => {
                let (a, c) = (p(2.5, 6.0), p(21.5, 18.0));
                cv.stroke_rrect(&rrect(a.X, a.Y, c.X, c.Y, 2.5 * s), b, sw, st);
                for (x, y) in [(7.0, 10.0), (10.5, 10.0), (14.0, 10.0), (17.5, 10.0)] {
                    let q = p(x, y);
                    cv.fill_ellipse(&ellipse(q.X, q.Y, 0.9 * s), b);
                }
                line(cv, (8.0, 14.5), (16.0, 14.5));
            }
            Icon::Drive | Icon::HardDrive => {
                let (a, c) = (p(3.0, 6.0), p(21.0, 18.0));
                cv.stroke_rrect(&rrect(a.X, a.Y, c.X, c.Y, 3.0 * s), b, sw, st);
                line(cv, (3.0, 12.5), (21.0, 12.5));
                for x in [7.0, 10.5] {
                    let q = p(x, 15.3);
                    cv.fill_ellipse(&ellipse(q.X, q.Y, 1.1 * s), b);
                }
            }
        }
    }
}

// ---- pill ----

#[derive(Clone, PartialEq, Debug)]
pub enum PillAction {
    AddDrink(u32),
    UndoDrink,
    UndoFocus,
    ResumeFocus,
    StartFocus,
    ShowWaited,
    TaskDone(app_store::TaskId),
    OpenTasks,
    /// Opens an event's Meet, Zoom or Teams link in the browser.
    Join(String),
    /// The offer pill's Save: a split button whose chevron (or a right-click) opens the format menu.
    SaveLink(app_downloads::MediaLink, app_downloads::Format),
    OpenFile(std::path::PathBuf),
    OpenDownloads,
    ChangeFolder(app_downloads::JobId),
    RetryDownload(app_downloads::JobId),
    /// The low-space pill: scan this drive and open the Scanning panel.
    FindBigFolders(char),
    OpenDisk,
    /// Puts what the last move sent to the Recycle Bin back where it was.
    UndoRecycle,
    RetryScan,
    /// Opens Settings on General (the shortcut is taken at launch).
    OpenSettings,
    /// Puts the shortcut back to the one before the last Save.
    UndoShortcut,
    /// "Memasuki waktu": stops the adzan that is playing (Hentikan).
    StopAdzan,
}

impl PillAction {
    /// The one split button: its chevron opens a menu.
    pub fn has_menu(&self) -> bool {
        matches!(self, PillAction::SaveLink(..))
    }
}

/// The split button's chevron part, at the right end of the action button.
pub const CHEVRON_W: f32 = 24.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub icon: Icon,
    pub dot: bool, // urgent: a filled ink dot instead of an icon
    pub title: String,
    pub value: String,
    pub action: Option<(String, PillAction)>,
    pub closable: bool,
    pub ttl_ms: u32,
}

#[derive(Clone, Copy, Default)]
pub struct PillLayout {
    pub w: f32,
    pub title_x: f32,
    pub value_x: f32,
    pub title_w: f32,
    pub value_w: f32,
    pub action: Option<(f32, f32)>, // x, width
    pub close: Option<f32>,         // x
}

pub const PILL_H: f32 = 48.0;
pub const PILL_PAD: f32 = 4.0; // window margin around the pill

pub fn layout_pill(f: &Fonts, p: &Pill) -> PillLayout {
    let mut l = PillLayout::default();
    let mut x = 6.0 + 36.0 + 10.0;
    l.title_x = x;
    l.title_w = text_width(&p.title, &f.title);
    x += l.title_w;
    if !p.value.is_empty() {
        x += 4.0;
        l.value_x = x;
        l.value_w = text_width(&p.value, &f.body);
        x += l.value_w;
    }
    let mut last_is_text = true;
    if let Some((label, a)) = &p.action {
        x += 10.0;
        let bw =
            text_width(label, &f.title) + 24.0 + if a.has_menu() { CHEVRON_W - 6.0 } else { 0.0 };
        l.action = Some((x, bw));
        x += bw;
        last_is_text = false;
    }
    if p.closable {
        x += 4.0;
        l.close = Some(x);
        x += 28.0;
        last_is_text = false;
    }
    x += if last_is_text { 16.0 } else { 6.0 };
    l.w = x.ceil();
    l
}

#[derive(Clone, Copy, PartialEq)]
pub enum PillPart {
    None,
    Action,
    /// The split button's chevron.
    Menu,
    Close,
    Body,
}

pub fn hit_pill(l: &PillLayout, menu: bool, x: f32, y: f32) -> PillPart {
    let (x, y) = (x - PILL_PAD, y - PILL_PAD);
    if x < 0.0 || x > l.w || y < 0.0 || y > PILL_H {
        return PillPart::None;
    }
    if let Some((ax, aw)) = l.action {
        if x >= ax && x <= ax + aw && (8.0..=40.0).contains(&y) {
            return if menu && x >= ax + aw - CHEVRON_W {
                PillPart::Menu
            } else {
                PillPart::Action
            };
        }
    }
    if let Some(cx) = l.close {
        if x >= cx && x <= cx + 28.0 {
            return PillPart::Close;
        }
    }
    PillPart::Body
}

/// `reveal` 0..1 grows the pill out of the drop. `grow_left` = the orb is on the right. The
/// pill's full shape has its top left at (`ox`, `oy`).
pub fn draw_pill(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    p: &Pill,
    l: &PillLayout,
    reveal: f32,
    grow_left: bool,
    hover: PillPart,
    ox: f32,
    oy: f32,
) {
    let r = reveal.clamp(0.0, 1.2);
    let width = 48.0 + (l.w - 48.0) * r.min(1.0);
    let (left, right) = if grow_left {
        (ox + l.w - width, ox + l.w)
    } else {
        (ox, ox + width)
    };
    let top = oy;
    let ca = ((r - 0.55) / 0.4).clamp(0.0, 1.0); // content fades in late
    {
        let shape = rrect(left, top, right, top + PILL_H, PILL_H / 2.0);
        cv.fill_rrect(&shape, t.surface);
        let hair = rrect(
            left + 0.5,
            top + 0.5,
            right - 0.5,
            top + PILL_H - 0.5,
            PILL_H / 2.0 - 0.5,
        );
        cv.stroke_rrect(&hair, t.hair, 1.0, Stroke::Plain);
        lip(cv, t, left, top, right, top + PILL_H, PILL_H / 2.0, 1.0);
        if ca <= 0.0 {
            return;
        }
        cv.push_clip(left, top, right, top + PILL_H);
        // Content sits at its final place; while growing it is clipped by the shape.
        let cy = top + PILL_H / 2.0;
        let chip = ellipse(ox + 6.0 + 18.0, cy, 18.0);
        let ember = p.icon.is_ember();
        cv.fill_ellipse(
            &chip,
            with_alpha(
                if ember {
                    with_alpha(t.ember, 0.14)
                } else {
                    t.chip
                },
                ca,
            ),
        );
        let ink = with_alpha(t.text, ca);
        if p.dot {
            cv.fill_ellipse(&ellipse(chip.point.X, cy, 5.0), ink);
        } else {
            let icon_c = if ember { with_alpha(t.ember, ca) } else { ink };
            paint_icon(cv, p.icon, chip.point.X, cy, 16.0, icon_c);
        }
        cv.text(
            &p.title,
            &f.title,
            ox + l.title_x,
            top,
            l.title_w + 2.0,
            PILL_H,
            ink,
        );
        if !p.value.is_empty() {
            let muted = with_alpha(t.muted, ca);
            cv.text(
                &p.value,
                &f.body,
                ox + l.value_x,
                top,
                l.value_w + 2.0,
                PILL_H,
                muted,
            );
        }
        if let (Some((label, a)), Some((ax, aw))) = (&p.action, l.action) {
            let bg = if hover == PillPart::Action || hover == PillPart::Menu {
                t.button_hover
            } else {
                t.button
            };
            cv.fill_rrect(
                &rrect(ox + ax, top + 8.0, ox + ax + aw, top + 40.0, 16.0),
                with_alpha(bg, ca),
            );
            lip(
                cv,
                t,
                ox + ax,
                top + 8.0,
                ox + ax + aw,
                top + 40.0,
                16.0,
                ca,
            );
            cv.text(label, &f.title, ox + ax + 12.0, top, aw - 12.0, PILL_H, ink);
            if a.has_menu() {
                // The split: a hairline, then the chevron for the format menu.
                let sx = ox + ax + aw - CHEVRON_W;
                if hover == PillPart::Menu {
                    cv.fill_rrect(
                        &rrect(sx, top + 8.0, ox + ax + aw, top + 40.0, 16.0),
                        with_alpha(t.button_hover, ca),
                    );
                }
                cv.line(
                    pt(sx, top + 15.0),
                    pt(sx, top + 33.0),
                    with_alpha(t.hair, ca),
                    1.0,
                    Stroke::Plain,
                );
                paint_icon(
                    cv,
                    Icon::ChevronDown,
                    sx + CHEVRON_W / 2.0 - 1.0,
                    cy,
                    14.0,
                    ink,
                );
            }
        }
        if let Some(cx) = l.close {
            if hover == PillPart::Close {
                cv.fill_ellipse(&ellipse(ox + cx + 14.0, cy, 14.0), with_alpha(t.button, ca));
            }
            let m = with_alpha(t.muted, ca);
            paint_icon(cv, Icon::Close, ox + cx + 14.0, cy, 14.0, m);
        }
        cv.pop_clip();
    }
}

// ---- dial ----

pub struct DialItem {
    pub icon: Icon,
    pub name: &'static str,
    pub value: String,
}

/// A branch child: a number ("25") or an icon, and its aimed label ("Focus" "25 min").
pub struct DialChild {
    pub text: String,
    pub icon: Option<Icon>,
    pub name: String,
    pub value: String,
}

pub struct DialBranch<'a> {
    pub parent: usize,
    pub children: &'a [DialChild],
    pub aimed: Option<usize>,
    pub reveal: f32,
}

pub const DIAL_W: f32 = 440.0;
pub const DIAL_H: f32 = 430.0;
pub const DIAL_R: f32 = 92.0; // chip centres
pub const DIAL_BAND: f32 = 46.0;
pub const DIAL_CHIP: f32 = 22.0;
pub const BRANCH_R: f32 = 146.0;
pub const BRANCH_BAND: f32 = 42.0;
pub const CHILD_CHIP: f32 = 19.0;
/// Past this distance from the hub, with a branch open, the pointer aims the children.
pub const BRANCH_AIM: f32 = 122.0;
const CHILD_STEP: f32 = 32.0;
/// Past four children a fan would run the whole half dial and crowd the main band at both
/// ends, so a long branch sits further out on a tighter angle: about the same room between
/// chips along the arc, and clear air between the two rings.
const WIDE_AFTER: usize = 4;
const WIDE_R: f32 = 164.0;
const WIDE_STEP: f32 = 28.0;

/// The radius and angle step of a branch with `n` children.
pub fn branch_geom(n: usize) -> (f32, f32) {
    if n > WIDE_AFTER {
        (WIDE_R, WIDE_STEP)
    } else {
        (BRANCH_R, CHILD_STEP)
    }
}

/// The orb centre inside the dial window.
pub fn dial_center(mirrored: bool) -> (f32, f32) {
    if mirrored {
        (30.0, DIAL_H / 2.0)
    } else {
        (DIAL_W - 30.0, DIAL_H / 2.0)
    }
}

/// Item angle, clockwise from 12 o'clock. Docked right, the dial opens to the left.
pub fn item_angle(i: usize, mirrored: bool) -> f32 {
    let a = 345.0 - 30.0 * i as f32;
    if mirrored {
        360.0 - a
    } else {
        a
    }
}

/// Child angles, top to bottom, `branch_geom` apart and centred on the parent, then shifted so no
/// child runs past the band's ends (8 to 172 degrees from the edge).
pub fn child_angles(parent: usize, n: usize, mirrored: bool) -> Vec<f32> {
    let base = item_angle(parent, false);
    let half = (n as f32 - 1.0) / 2.0;
    let mut v: Vec<f32> = (0..n)
        .map(|j| base - (j as f32 - half) * branch_geom(n).1)
        .collect();
    let (top, bottom) = (
        v.first().copied().unwrap_or(base),
        v.last().copied().unwrap_or(base),
    );
    let shift = if top > 352.0 {
        352.0 - top
    } else if bottom < 188.0 {
        188.0 - bottom
    } else {
        0.0
    };
    for a in &mut v {
        *a += shift;
        if mirrored {
            *a = 360.0 - *a;
        }
    }
    v
}

fn polar(cx: f32, cy: f32, r: f32, deg: f32) -> Vector2 {
    on_circle(cx, cy, r, deg)
}

fn pointer_deg(dx: f32, dy: f32) -> f32 {
    let deg = dx.atan2(-dy).to_degrees();
    if deg < 0.0 {
        deg + 360.0
    } else {
        deg
    }
}

fn nearest(deg: f32, angles: impl Iterator<Item = f32>, within: f32) -> Option<usize> {
    angles
        .enumerate()
        .map(|(i, a)| {
            let mut diff = (deg - a).abs();
            if diff > 180.0 {
                diff = 360.0 - diff;
            }
            (i, diff)
        })
        .filter(|(_, d)| *d <= within)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(i, _)| i)
}

/// Which item the pointer aims at: direction only, beyond the drop.
pub fn aim(dx: f32, dy: f32, mirrored: bool, n: usize) -> Option<usize> {
    if (dx * dx + dy * dy).sqrt() < 30.0 {
        return None;
    }
    nearest(
        pointer_deg(dx, dy),
        (0..n).map(|i| item_angle(i, mirrored)),
        18.0,
    )
}

/// Which child of an open branch the pointer aims at: past the main band, by direction.
pub fn aim_child(dx: f32, dy: f32, mirrored: bool, parent: usize, n: usize) -> Option<usize> {
    if (dx * dx + dy * dy).sqrt() < BRANCH_AIM {
        return None;
    }
    nearest(
        pointer_deg(dx, dy),
        child_angles(parent, n, mirrored).into_iter(),
        branch_geom(n).1 / 2.0,
    )
}

/// A two-tone label pill: the noun in text colour 500, the value muted. Sits beside `c`.
fn label_pill(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    name: &str,
    value: &str,
    c: Vector2,
    off: f32,
    mirrored: bool,
    e: f32,
) {
    let name_w = if name.is_empty() {
        0.0
    } else {
        text_width(name, &f.title)
    };
    let val_w = if value.is_empty() {
        0.0
    } else {
        text_width(value, &f.body)
    };
    let gap = if name_w > 0.0 && val_w > 0.0 {
        5.0
    } else {
        0.0
    };
    let tw = name_w + gap + val_w + 22.0;
    let th = 28.0;
    let (tl, tr) = if mirrored {
        (c.X + off, c.X + off + tw)
    } else {
        (c.X - off - tw, c.X - off)
    };
    let tt = c.Y - th / 2.0;
    {
        cv.fill_rrect(&rrect(tl, tt, tr, tt + th, th / 2.0), with_alpha(t.band, e));
        cv.stroke_rrect(
            &rrect(tl + 0.5, tt + 0.5, tr - 0.5, tt + th - 0.5, th / 2.0 - 0.5),
            with_alpha(t.hair, e),
            1.0,
            Stroke::Plain,
        );
    }
    let x = tl + 11.0;
    if name_w > 0.0 {
        cv.text(
            name,
            &f.title,
            x,
            tt,
            name_w + 2.0,
            th,
            with_alpha(t.text, e),
        );
    }
    if val_w > 0.0 {
        cv.text(
            value,
            &f.body,
            x + name_w + gap,
            tt,
            val_w + 2.0,
            th,
            with_alpha(t.muted, e),
        );
    }
}

pub fn draw_dial(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    items: &[DialItem],
    aimed: Option<usize>,
    reveal: f32,
    mirrored: bool,
    keyboard: bool,
    branch: Option<&DialBranch>,
    focus_ring: Option<f32>,
    tags: f32,
) {
    let (cx, cy) = dial_center(mirrored);
    let rv = reveal.clamp(0.0, 1.1);
    if rv <= 0.01 {
        return;
    }
    let band_a = rv.min(1.0);
    let st = Stroke::Round;
    {
        // The band: a thick half-ring stroke with round ends, opening away from the edge.
        let (a0, a1) = if mirrored {
            (5.0, 175.0)
        } else {
            (185.0, 355.0)
        };
        let r = DIAL_R * (0.82 + 0.18 * rv.min(1.0));
        let start = polar(cx, cy, r, a0);
        let end = polar(cx, cy, r, a1);
        let geo = path(start, false, |k| arc_to(k, end, r, false, true));
        cv.stroke_path(&geo, with_alpha(t.hair, band_a), DIAL_BAND + 1.5, st);
        cv.stroke_path(&geo, with_alpha(t.band, band_a), DIAL_BAND, st);

        for (i, it) in items.iter().enumerate() {
            // Chips ripple in from the middle out, 30 ms apart.
            let order = (i as f32 - 2.5).abs();
            let e = ((rv - order * 0.06) / 0.7).clamp(0.0, 1.0);
            if e <= 0.0 {
                continue;
            }
            let c = polar(cx, cy, r, item_angle(i, mirrored));
            let on = aimed == Some(i);
            let chip_bg = if on { t.text } else { t.chip };
            cv.fill_ellipse(
                &ellipse(c.X, c.Y, DIAL_CHIP * (0.7 + 0.3 * e)),
                with_alpha(chip_bg, e),
            );
            if on && keyboard {
                cv.stroke_ellipse(
                    &ellipse(c.X, c.Y, DIAL_CHIP + 3.0),
                    with_alpha(t.text, e),
                    2.0,
                    Stroke::Plain,
                );
            }
            // A running focus draws its ring around the Focus chip too: dotted ahead, solid spent.
            if let (Some(p), Icon::Focus) = (focus_ring, it.icon) {
                let rr = DIAL_CHIP + 3.5;
                cv.stroke_ellipse(
                    &ellipse(c.X, c.Y, rr),
                    with_alpha(t.track, e),
                    1.5,
                    Stroke::Dotted,
                );
                cv.arc(c.X, c.Y, rr, 0.0, 360.0 * p, with_alpha(t.text, e), 2.0, st);
            }
            let icon_c = if on { t.band } else { t.text };
            paint_icon(cv, it.icon, c.X, c.Y, 16.0, with_alpha(icon_c, e));

            // Tag outside the band: the live value, or the name and value when aimed.
            // While a branch parent is aimed or open the main tags step aside for it.
            let tag_e = e * tags.clamp(0.0, 1.0);
            if tag_e <= 0.01 {
                continue;
            }
            let e = tag_e;
            let show_name = on || keyboard;
            if it.value.is_empty() && !show_name {
                continue;
            }
            let name_w = if show_name {
                text_width(it.name, &f.small_medium)
            } else {
                0.0
            };
            let val_w = if it.value.is_empty() {
                0.0
            } else {
                text_width(&it.value, &f.small)
            };
            let gap = if show_name && val_w > 0.0 { 4.0 } else { 0.0 };
            let tw = name_w + gap + val_w + 20.0;
            let th = 24.0;
            let (tl, tr) = if mirrored {
                (c.X + 32.0, c.X + 32.0 + tw)
            } else {
                (c.X - 32.0 - tw, c.X - 32.0)
            };
            let tt = c.Y - th / 2.0;
            cv.fill_rrect(&rrect(tl, tt, tr, tt + th, th / 2.0), with_alpha(t.band, e));
            cv.stroke_rrect(
                &rrect(tl + 0.5, tt + 0.5, tr - 0.5, tt + th - 0.5, th / 2.0 - 0.5),
                with_alpha(t.hair, e),
                1.0,
                Stroke::Plain,
            );
            let mut x = tl + 10.0;
            if show_name {
                cv.text(
                    it.name,
                    &f.small_medium,
                    x,
                    tt,
                    name_w + 2.0,
                    th,
                    with_alpha(t.text, e),
                );
                x += name_w + gap;
            }
            if val_w > 0.0 {
                cv.text(
                    &it.value,
                    &f.small,
                    x,
                    tt,
                    val_w + 2.0,
                    th,
                    with_alpha(t.muted, e),
                );
            }
        }
    }

    // The branch band fans out from its parent, past the main band.
    let Some(b) = branch else { return };
    let e = (b.reveal.clamp(0.0, 1.1) * band_a).min(1.0);
    if e <= 0.01 || b.children.is_empty() {
        return;
    }
    let angles = child_angles(b.parent, b.children.len(), mirrored);
    let br = branch_geom(b.children.len()).0 * (0.9 + 0.1 * b.reveal.clamp(0.0, 1.1));
    // The band runs about 30 px past its end chips on any radius.
    let pad = 12.0 * BRANCH_R / branch_geom(b.children.len()).0;
    let lo = angles.iter().copied().fold(f32::MAX, f32::min) - pad;
    let hi = angles.iter().copied().fold(f32::MIN, f32::max) + pad;
    {
        let geo = path(polar(cx, cy, br, lo), false, |k| {
            arc_to(k, polar(cx, cy, br, hi), br, false, true)
        });
        cv.stroke_path(&geo, with_alpha(t.hair, e), BRANCH_BAND + 1.5, st);
        cv.stroke_path(&geo, with_alpha(t.band, e), BRANCH_BAND, st);
        for (j, ch) in b.children.iter().enumerate() {
            let c = polar(cx, cy, br, angles[j]);
            let on = b.aimed == Some(j);
            cv.fill_ellipse(
                &ellipse(c.X, c.Y, CHILD_CHIP * (0.7 + 0.3 * e)),
                with_alpha(if on { t.text } else { t.chip }, e),
            );
            if on && keyboard {
                cv.stroke_ellipse(
                    &ellipse(c.X, c.Y, CHILD_CHIP + 3.0),
                    with_alpha(t.text, e),
                    2.0,
                    Stroke::Plain,
                );
            }
            let ink = with_alpha(if on { t.band } else { t.text }, e);
            match ch.icon {
                Some(icon) => paint_icon(cv, icon, c.X, c.Y, 15.0, ink),
                None => cv.center_text(&ch.text, &f.title, c.X, c.Y - 10.0, 20.0, ink),
            }
        }
        if let Some(j) = b.aimed {
            let ch = &b.children[j];
            let c = polar(cx, cy, br, angles[j]);
            label_pill(
                cv,
                t,
                f,
                &ch.name,
                &ch.value,
                c,
                CHILD_CHIP + 8.0,
                mirrored,
                e,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_fan_around_their_parent_top_to_bottom() {
        // Focus (item 3) sits at 255 degrees docked right; its three children are 32 apart.
        assert_eq!(child_angles(3, 3, false), vec![287.0, 255.0, 223.0]);
        // Mirrored for the left edge.
        assert_eq!(child_angles(3, 3, true), vec![73.0, 105.0, 137.0]);
    }

    #[test]
    fn a_fan_near_the_bottom_is_shifted_back_onto_the_band() {
        // More (item 5) at 195 degrees would put its last child at 147; it is shifted to end at 188.
        let a = child_angles(5, 4, false);
        assert_eq!(*a.last().unwrap(), 188.0);
        assert!(a.iter().all(|x| (188.0..=352.0).contains(x)));
    }

    #[test]
    fn aim_is_direction_only_and_children_need_distance() {
        // Left and a little down from the hub, docked right, is Focus (255 degrees), however far out.
        assert_eq!(aim(-100.0, 10.0, false, 6), Some(3));
        assert_eq!(aim(-400.0, 40.0, false, 6), Some(3));
        assert_eq!(aim(-10.0, 0.0, false, 6), None);
        // Toward Drink's middle child at 225 degrees: close in aims nothing, far out aims the child.
        let (dx, dy) = (
            -(225f32.to_radians().sin().abs()),
            225f32.to_radians().cos().abs(),
        );
        assert_eq!(aim_child(dx * 100.0, dy * 100.0, false, 4, 3), None);
        assert_eq!(aim_child(dx * 150.0, dy * 150.0, false, 4, 3), Some(1));
    }
}
