#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Settings window (frame "v6.2 · 22", 720 × 580: a sidebar of sections and the section's
//! cards) and the first run (frame "v6.2 · 21", five steps in one small window). One painter
//! walks the layout: with a device context it draws, without one it only records where each
//! control is, so drawing, hit testing and Tab order can never disagree.

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

use crate::gfx::*;
use crate::notices_model::Source;
use crate::settings_model::{
    self as m, AtTimeSound, Capture, Locate, PrayerMethod, Section, Shortcut, Startup, Step,
    ThemeMode, CITIES,
};
use crate::tasks_panel::clipped_text;
use crate::textfield::TextField;
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

pub type Rect = (f32, f32, f32, f32);

pub const W: f32 = 720.0;
pub const H: f32 = 580.0;
pub const FR_W: f32 = 284.0;
pub const FR_H: f32 = 396.0;
const SIDE: f32 = 200.0;
const X0: f32 = 224.0;
const X1: f32 = W - 24.0;
/// The content scrolls under the title from here.
pub const CONTENT_TOP: f32 = 64.0;
/// Where the scrolling content is clipped: the window region rounds only the corners, and General
/// ends with its footer just above this line, as in the frame.
pub const CONTENT_BOTTOM: f32 = H - 4.0;
const CITY_ROW: f32 = 40.0;
const FR_CITY_ROW: f32 = 34.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Section(Section),
    Close,
    StartWithWindows,
    ShowAtStart,
    Theme(ThemeMode),
    ChangeShortcut,
    CaptureCancel,
    CaptureSave,
    CaptureReset,
    PickAnother,
    UseSuggestion,
    StartupApps,
    PlaceWindows,
    PlaceCity,
    LocationSettings,
    CityBack,
    CityRow(usize),
    Reminder(usize),
    Method(PrayerMethod),
    /// Prayer (0 Subuh to 4 Isya) and the step, −1 or +1 minute.
    Adjust(usize, i32),
    AtTime(AtTimeSound),
    /// Choose the adzan file (true: Subuh's own).
    PickAdzan(bool),
    ClearAdzanSubuh,
    Source(Source),
    OpenData,
    // the first run
    Next,
    Back,
    Finish,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    FirstRun(Step),
    Settings(Section),
}

pub struct CityPick<'a> {
    pub field: &'a TextField,
    pub rows: &'a [usize],
    pub sel: usize,
    pub scroll: usize,
    /// The city in use, ticked in the list.
    pub current: Option<&'a str>,
}

pub struct SourceRow {
    pub source: Source,
    pub on: bool,
    pub words: String,
}

pub struct View<'a> {
    pub screen: Screen,
    pub prefs: &'a m::Prefs,
    pub windows_dark: bool,
    pub startup: Startup,
    /// The first run's Start with Windows, applied at Finish.
    pub fr_start: bool,
    pub capture: Option<Capture>,
    /// The last Save hit a shortcut another app holds.
    pub taken: Option<Shortcut>,
    /// The shortcut in use could not be registered (another app holds it).
    pub hotkey_ok: bool,
    pub suggestion: Option<Shortcut>,
    pub tried: bool,
    pub want_windows: bool,
    pub locate: Locate,
    /// The city on the radio's second row (the first run's draft, or the one in use).
    pub city: Option<String>,
    pub picking: Option<CityPick<'a>>,
    pub day: String,
    pub times: [(&'static str, String); 5],
    pub qibla: String,
    pub sources: Vec<SourceRow>,
    pub data_dir: String,
    pub scroll: f32,
    pub hover: Option<Hit>,
    pub focus: Option<Hit>,
}

/// Draws when it has a device context, and always records each control's rectangle.
pub struct Painter<'a> {
    g: &'a Shared,
    dc: Option<&'a ID2D1DeviceContext>,
    t: &'a Theme,
    f: &'a Fonts,
    hover: Option<Hit>,
    focus: Option<Hit>,
    pub hits: Vec<(Hit, Rect)>,
    /// Hits outside this band (the scrolled content's clip) are not recorded.
    clip: (f32, f32),
}

impl<'a> Painter<'a> {
    fn col(&self, c: D2D1_COLOR_F) -> ID2D1SolidColorBrush {
        brush(self.dc.expect("drawing"), c)
    }

    fn fill(&self, r: Rect, rad: f32, c: D2D1_COLOR_F) {
        if let Some(dc) = self.dc {
            unsafe { dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, rad), &brush(dc, c)) };
        }
    }

    /// The T3 lip on a card or control (depth.rs).
    fn lip(&self, r: Rect, rad: f32, a: f32) {
        if let Some(dc) = self.dc {
            crate::depth::lip(self.g, dc, self.t, r.0, r.1, r.2, r.3, rad, a);
        }
    }

    fn stroke(&self, r: Rect, rad: f32, c: D2D1_COLOR_F, w: f32) {
        if let Some(dc) = self.dc {
            unsafe {
                dc.DrawRoundedRectangle(
                    &rrect(
                        r.0 + w / 2.0,
                        r.1 + w / 2.0,
                        r.2 - w / 2.0,
                        r.3 - w / 2.0,
                        rad,
                    ),
                    &brush(dc, c),
                    w,
                    None,
                )
            };
        }
    }

    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: D2D1_COLOR_F) {
        if let Some(dc) = self.dc {
            unsafe { dc.DrawLine(pt(x0, y0), pt(x1, y1), &brush(dc, c), 1.0, None) };
        }
    }

    fn text(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
    ) {
        if let Some(dc) = self.dc {
            clipped_text(self.g, dc, s, fmt, x, y, w, h, &self.col(c));
        }
    }

    fn right(&self, s: &str, fmt: &IDWriteTextFormat, right: f32, y: f32, h: f32, c: D2D1_COLOR_F) {
        if let Some(dc) = self.dc {
            right_text(self.g, dc, s, fmt, right, y, h, &self.col(c));
        }
    }

    fn wrap(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
    ) {
        if let Some(dc) = self.dc {
            text(dc, s, fmt, x, y, w, h, &self.col(c));
        }
    }

    fn icon(&self, i: Icon, cx: f32, cy: f32, size: f32, c: D2D1_COLOR_F) {
        if let Some(dc) = self.dc {
            draw_icon(self.g, dc, i, cx, cy, size, &self.col(c));
        }
    }

    fn width(&self, s: &str, fmt: &IDWriteTextFormat) -> f32 {
        self.g.text_width(s, fmt)
    }

    fn hit(&mut self, h: Hit, r: Rect) {
        let mid = (r.1 + r.3) / 2.0;
        if mid >= self.clip.0 && mid <= self.clip.1 {
            self.hits.push((h, r));
        }
    }

    fn hovered(&self, h: Hit) -> bool {
        self.hover == Some(h)
    }

    fn ring(&self, h: Hit, r: Rect, rad: f32) {
        if self.focus == Some(h) {
            self.stroke(
                (r.0 - 3.0, r.1 - 3.0, r.2 + 3.0, r.3 + 3.0),
                rad + 3.0,
                self.t.text,
                2.0,
            );
        }
    }

    /// A pill button. `primary` is ink on the panel colour.
    fn button(&mut self, r: Rect, label: &str, primary: bool, enabled: bool, h: Option<Hit>) {
        let t = *self.t;
        let a = if enabled { 1.0 } else { 0.4 };
        let hover = h.is_some_and(|h| self.hovered(h)) && enabled;
        let fill = if primary {
            with_alpha(t.text, if hover { 0.85 } else { a })
        } else if hover {
            t.button_hover
        } else {
            t.button
        };
        let ink = if primary {
            t.panel
        } else {
            with_alpha(t.text, a)
        };
        let rad = (r.3 - r.1) / 2.0;
        self.fill(r, rad, fill);
        if !primary {
            self.lip(r, rad, a);
        }
        let fmt = &self.f.small_medium;
        let w = self.width(label, fmt);
        self.text(
            label,
            fmt,
            (r.0 + r.2) / 2.0 - w / 2.0,
            r.1,
            w + 2.0,
            r.3 - r.1,
            ink,
        );
        if let (Some(h), true) = (h, enabled) {
            self.ring(h, r, rad);
            self.hit(h, r);
        }
    }

    /// A button sized to its label, right-aligned at `right`. Returns its left edge.
    fn button_at(
        &mut self,
        right: f32,
        cy: f32,
        label: &str,
        primary: bool,
        enabled: bool,
        h: Option<Hit>,
    ) -> f32 {
        let w = self.width(label, &self.f.small_medium) + 26.0;
        let r = (right - w, cy - 14.0, right, cy + 14.0);
        self.button(r, label, primary, enabled, h);
        r.0
    }

    fn toggle(&mut self, right: f32, cy: f32, on: bool, enabled: bool, h: Hit) {
        let t = *self.t;
        let r = (right - 34.0, cy - 10.0, right, cy + 10.0);
        let a = if enabled { 1.0 } else { 0.45 };
        let track = if on {
            with_alpha(t.text, a)
        } else {
            with_alpha(t.track, a)
        };
        self.fill(r, 10.0, track);
        let kx = if on { r.2 - 10.0 } else { r.0 + 10.0 };
        let knob = if on {
            t.panel
        } else if t.dark {
            t.muted
        } else {
            t.surface
        };
        if let Some(dc) = self.dc {
            unsafe {
                dc.FillEllipse(
                    &ellipse(kx, cy, 7.0),
                    &brush(dc, with_alpha(knob, a.max(0.8))),
                )
            };
        }
        if enabled {
            self.ring(h, r, 10.0);
            self.hit(h, (r.0 - 6.0, r.1 - 6.0, r.2 + 4.0, r.3 + 6.0));
        }
    }

    fn radio(&self, cx: f32, cy: f32, on: bool) {
        let t = *self.t;
        if let Some(dc) = self.dc {
            unsafe {
                dc.DrawEllipse(
                    &ellipse(cx, cy, 7.5),
                    &brush(dc, if on { t.text } else { t.muted }),
                    1.5,
                    None,
                );
                if on {
                    dc.FillEllipse(&ellipse(cx, cy, 4.0), &brush(dc, t.text));
                }
            }
        }
    }

    /// Keycaps from `x`, vertically centred on `cy`. Returns the right edge.
    fn caps(&self, parts: &[String], x: f32, cy: f32) -> f32 {
        let t = *self.t;
        let fmt = &self.f.small_medium;
        let mut x = x;
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                self.text("+", &self.f.small, x + 4.0, cy - 10.0, 10.0, 20.0, t.muted);
                x += 16.0;
            }
            let w = (self.width(p, fmt) + 18.0).max(30.0);
            let r = (x, cy - 13.0, x + w, cy + 13.0);
            self.fill(r, 6.0, t.surface);
            self.stroke(r, 6.0, t.hair, 1.0);
            self.text(
                p,
                fmt,
                x + (w - self.width(p, fmt)) / 2.0,
                r.1,
                w,
                26.0,
                t.text,
            );
            x += w;
        }
        x
    }

    fn caps_width(&self, parts: &[String]) -> f32 {
        let fmt = &self.f.small_medium;
        parts
            .iter()
            .map(|p| (self.width(p, fmt) + 18.0).max(30.0))
            .sum::<f32>()
            + 16.0 * parts.len().saturating_sub(1) as f32
    }

    /// A card from `top`: the raised fill, the hairline and row dividers at `rows` heights.
    fn card(&self, top: f32, rows: &[f32]) -> f32 {
        let t = *self.t;
        let h: f32 = rows.iter().sum();
        self.fill((X0, top, X1, top + h), 10.0, t.raised);
        self.stroke((X0, top, X1, top + h), 10.0, t.hair, 1.0);
        self.lip((X0, top, X1, top + h), 10.0, 1.0);
        let mut y = top;
        for r in &rows[..rows.len().saturating_sub(1)] {
            y += r;
            self.line(X0 + 1.0, y, X1 - 1.0, y, t.hair);
        }
        top + h
    }

    /// A row's two lines at the card's left padding.
    fn row_words(&self, top: f32, h: f32, title: &str, sub: &str, right_room: f32) {
        let t = *self.t;
        let w = X1 - X0 - 28.0 - right_room;
        if sub.is_empty() {
            self.text(title, &self.f.title, X0 + 14.0, top, w, h, t.text);
        } else {
            let mid = top + h / 2.0;
            self.text(title, &self.f.title, X0 + 14.0, mid - 18.0, w, 18.0, t.text);
            self.text(sub, &self.f.small, X0 + 14.0, mid + 1.0, w, 17.0, t.muted);
        }
    }

    fn group(&self, y: f32, label: &str) -> f32 {
        self.text(label, &self.f.small, X0, y, 300.0, 18.0, self.t.muted);
        y + 24.0
    }

    /// An ember state card: icon, two lines, one action.
    fn problem(&mut self, top: f32, title: &str, sub: &str, action: Option<(&str, Hit)>) -> f32 {
        let t = *self.t;
        let h = 56.0;
        self.fill((X0, top, X1, top + h), 10.0, t.raised);
        self.stroke((X0, top, X1, top + h), 10.0, t.hair, 1.0);
        self.lip((X0, top, X1, top + h), 10.0, 1.0);
        self.fill(
            (X0 + 14.0, top + 16.0, X0 + 38.0, top + 40.0),
            6.0,
            with_alpha(t.ember, 0.14),
        );
        self.icon(Icon::Alert, X0 + 26.0, top + 28.0, 14.0, t.ember);
        let room = action.map_or(0.0, |(l, _)| self.width(l, &self.f.small_medium) + 40.0);
        let w = X1 - X0 - 64.0 - room;
        self.text(title, &self.f.title, X0 + 50.0, top + 10.0, w, 18.0, t.text);
        self.text(sub, &self.f.small, X0 + 50.0, top + 29.0, w, 17.0, t.muted);
        if let Some((l, hit)) = action {
            self.button_at(X1 - 12.0, top + h / 2.0, l, false, true, Some(hit));
        }
        top + h
    }
}

fn painter<'a>(
    g: &'a Shared,
    dc: Option<&'a ID2D1DeviceContext>,
    t: &'a Theme,
    f: &'a Fonts,
    v: &View,
) -> Painter<'a> {
    Painter {
        g,
        dc,
        t,
        f,
        hover: v.hover,
        focus: v.focus,
        hits: Vec::new(),
        clip: (f32::MIN, f32::MAX),
    }
}

/// The window's logical size for this screen.
pub fn size(screen: Screen) -> (f32, f32) {
    match screen {
        Screen::FirstRun(_) => (FR_W, FR_H),
        Screen::Settings(_) => (W, H),
    }
}

/// Every control's rectangle, in Tab order; the content's height for scrolling.
pub fn layout(g: &Shared, t: &Theme, f: &Fonts, v: &View) -> (Vec<(Hit, Rect)>, f32) {
    let mut p = painter(g, None, t, f, v);
    let bottom = paint(&mut p, v);
    (p.hits, bottom)
}

pub fn hit_at(hits: &[(Hit, Rect)], x: f32, y: f32) -> Option<Hit> {
    hits.iter()
        .rev()
        .find(|(_, r)| x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3)
        .map(|(h, _)| *h)
}

pub fn draw(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, v: &View) {
    let mut p = painter(g, Some(dc), t, f, v);
    paint(&mut p, v);
}

/// Returns the bottom of the scrolled content (unscrolled), so the caller can bound the scroll.
fn paint(p: &mut Painter, v: &View) -> f32 {
    match v.screen {
        Screen::Settings(s) => paint_settings(p, v, s),
        Screen::FirstRun(step) => {
            paint_first_run(p, v, step);
            FR_H
        }
    }
}

// ---- the Settings window ----

fn section_icon(s: Section) -> Icon {
    match s {
        Section::General => Icon::Gear,
        Section::Prayer => Icon::Prayer,
        Section::Notices => Icon::Bell,
        Section::Account => Icon::Person,
        Section::About => Icon::Info,
    }
}

fn paint_settings(p: &mut Painter, v: &View, s: Section) -> f32 {
    let t = *p.t;
    p.fill((0.0, 0.0, W, H), 16.0, t.panel);
    p.stroke((0.0, 0.0, W, H), 16.0, t.hair, 1.0);
    p.line(SIDE, 1.0, SIDE, H - 1.0, t.hair);
    // The sidebar: General / Prayer, Notices / Account, About.
    let mut y = 20.0;
    for (i, sec) in Section::ALL.iter().enumerate() {
        if i == 1 || i == 3 {
            p.line(20.0, y + 4.0, SIDE - 20.0, y + 4.0, t.hair);
            y += 9.0;
        }
        let r = (8.0, y, SIDE - 8.0, y + 32.0);
        let h = Hit::Section(*sec);
        if *sec == s {
            p.fill(r, 8.0, t.button);
        } else if p.hovered(h) {
            p.fill(r, 8.0, t.raised);
        }
        p.icon(
            section_icon(*sec),
            28.0,
            y + 16.0,
            15.0,
            if *sec == s { t.text } else { t.muted },
        );
        let fmt = if *sec == s { &p.f.title } else { &p.f.body };
        p.text(sec.name(), fmt, 46.0, y, 140.0, 32.0, t.text);
        p.ring(h, r, 8.0);
        p.hit(h, r);
        y += 34.0;
    }
    p.text(
        "On this PC only · sync is off",
        &p.f.small,
        20.0,
        H - 36.0,
        SIDE - 30.0,
        20.0,
        t.muted,
    );

    // The title and the close button stay put; the cards scroll under them.
    let title = if v.picking.is_some() {
        "Pick a city"
    } else {
        s.name()
    };
    let tx = if v.picking.is_some() {
        let r = (X0 - 4.0, 22.0, X0 + 24.0, 50.0);
        if p.hovered(Hit::CityBack) {
            p.fill(r, 14.0, t.button_hover);
        }
        p.icon(Icon::ArrowLeft, X0 + 10.0, 36.0, 16.0, t.text);
        p.ring(Hit::CityBack, r, 14.0);
        p.hit(Hit::CityBack, r);
        X0 + 32.0
    } else {
        X0
    };
    p.text(title, &p.f.heading, tx, 20.0, 360.0, 32.0, t.text);
    let cr = (W - 52.0, 22.0, W - 24.0, 50.0);
    p.fill(
        cr,
        14.0,
        if p.hovered(Hit::Close) {
            t.button_hover
        } else {
            t.button
        },
    );
    p.icon(Icon::Close, W - 38.0, 36.0, 14.0, t.muted);
    p.ring(Hit::Close, cr, 14.0);
    p.hit(Hit::Close, cr);

    if let Some(pick) = &v.picking {
        city_list(p, pick, (X0, CONTENT_TOP + 4.0, X1, H - 20.0), CITY_ROW);
        return H;
    }
    let clip = (CONTENT_TOP, CONTENT_BOTTOM);
    if let Some(dc) = p.dc {
        unsafe {
            dc.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: SIDE + 1.0,
                    top: clip.0,
                    right: W - 1.0,
                    bottom: clip.1,
                },
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            )
        };
        unsafe { dc.SetTransform(&translate(0.0, -v.scroll)) };
    }
    // Hits are recorded in window coordinates, so shift them by the scroll as they land.
    let first = p.hits.len();
    p.clip = (clip.0 + v.scroll, clip.1 + v.scroll);
    let top = CONTENT_TOP + 4.0;
    let bottom = match s {
        Section::General => general(p, v, top),
        Section::Prayer => prayer(p, v, top),
        Section::Notices => notices(p, v, top),
        Section::Account => account(p, top),
        Section::About => about(p, v, top),
    };
    p.clip = (f32::MIN, f32::MAX);
    for (_, r) in &mut p.hits[first..] {
        r.1 -= v.scroll;
        r.3 -= v.scroll;
    }
    if let Some(dc) = p.dc {
        unsafe { dc.SetTransform(&translate(0.0, 0.0)) };
        unsafe { dc.PopAxisAlignedClip() };
    }
    bottom
}

fn translate(x: f32, y: f32) -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: x,
        M32: y,
    }
}

fn general(p: &mut Painter, v: &View, top: f32) -> f32 {
    let t = *p.t;
    let mut y = p.group(top, "Startup");
    let c = p.card(y, &[56.0, 56.0]);
    p.row_words(
        y,
        56.0,
        "Start with Windows",
        "Opens quietly as the orb when you sign in",
        50.0,
    );
    p.toggle(
        X1 - 14.0,
        y + 28.0,
        v.startup == Startup::On,
        true,
        Hit::StartWithWindows,
    );
    let sc = v.prefs.shortcut.label();
    p.row_words(
        y + 56.0,
        56.0,
        "Show the orb at start",
        &format!("Off starts it hidden; {sc} brings it back"),
        50.0,
    );
    p.toggle(
        X1 - 14.0,
        y + 84.0,
        v.prefs.show_orb_at_start,
        true,
        Hit::ShowAtStart,
    );
    y = c + 8.0;
    if v.startup == Startup::Blocked {
        y = p.problem(
            y,
            "Couldn't add to startup",
            "Windows blocked it. Allow Orby there.",
            Some(("Startup apps", Hit::StartupApps)),
        ) + 8.0;
    }

    y = p.group(y + 8.0, "Appearance");
    let c = p.card(y, &[56.0]);
    p.row_words(
        y,
        56.0,
        "Theme",
        &v.prefs.theme.words(v.windows_dark),
        220.0,
    );
    segmented(p, X1 - 12.0, y + 28.0, v.prefs.theme);
    y = c + 16.0;

    y = p.group(y, "Keyboard");
    match v.capture {
        None => {
            let c = p.card(y, &[56.0]);
            let sub = if v.hotkey_ok {
                "Works from any app"
            } else {
                "Another app holds it"
            };
            p.row_words(y, 56.0, "Show or hide the orb", sub, 230.0);
            let left = p.button_at(
                X1 - 12.0,
                y + 28.0,
                "Change",
                false,
                true,
                Some(Hit::ChangeShortcut),
            );
            let parts = v.prefs.shortcut.parts();
            let cw = p.caps_width(&parts);
            p.caps(&parts, left - 10.0 - cw, y + 28.0);
            y = c + 8.0;
        }
        Some(cap) => {
            let h = 150.0;
            p.fill((X0, y, X1, y + h), 10.0, t.raised);
            p.stroke((X0, y, X1, y + h), 10.0, t.hair, 1.0);
            p.text(
                "Change shortcut",
                &p.f.title,
                X0 + 14.0,
                y + 10.0,
                200.0,
                22.0,
                t.text,
            );
            p.right(
                "Esc cancels",
                &p.f.small,
                X1 - 14.0,
                y + 10.0,
                22.0,
                t.muted,
            );
            let field = (X0 + 14.0, y + 40.0, X1 - 14.0, y + 84.0);
            p.fill(field, 10.0, t.surface);
            p.stroke(field, 10.0, t.text, 1.5);
            let (parts, whole) = cap.caps();
            let prompt = if whole { "" } else { "press a key" };
            let pw = if prompt.is_empty() {
                0.0
            } else {
                p.width(prompt, &p.f.small) + 14.0
            };
            let cw = if parts.is_empty() {
                0.0
            } else {
                p.caps_width(&parts) + 8.0
            };
            let x = (field.0 + field.2) / 2.0 - (cw + pw) / 2.0;
            let cy = (field.1 + field.3) / 2.0;
            let end = if parts.is_empty() {
                x
            } else {
                p.caps(&parts, x, cy) + 8.0
            };
            if !prompt.is_empty() {
                p.line(end + 1.0, cy - 9.0, end + 1.0, cy + 9.0, t.text);
                p.text(prompt, &p.f.small, end + 8.0, cy - 10.0, pw, 20.0, t.muted);
            }
            p.text(
                cap.hint(),
                &p.f.small,
                X0 + 14.0,
                y + 90.0,
                360.0,
                18.0,
                t.muted,
            );
            let by = y + 128.0;
            let reset = format!("Reset to {}", Shortcut::DEFAULT.label());
            let rw = p.width(&reset, &p.f.small_medium) + 26.0;
            p.button(
                (X0 + 14.0, by - 14.0, X0 + 14.0 + rw, by + 14.0),
                &reset,
                false,
                true,
                Some(Hit::CaptureReset),
            );
            let l = p.button_at(
                X1 - 14.0,
                by,
                "Save",
                true,
                cap.got.is_some(),
                Some(Hit::CaptureSave),
            );
            p.button_at(l - 8.0, by, "Cancel", false, true, Some(Hit::CaptureCancel));
            y += h + 8.0;
        }
    }
    if let Some(s) = v.taken {
        y = p.problem(
            y,
            &format!("Couldn't use {}", s.label()),
            "Another app already uses it.",
            Some(("Pick another", Hit::PickAnother)),
        ) + 8.0;
    } else if !v.hotkey_ok && v.capture.is_none() {
        y = p.problem(
            y,
            &format!("Couldn't use {}", v.prefs.shortcut.label()),
            "Another app already uses it.",
            Some(("Pick another", Hit::ChangeShortcut)),
        ) + 8.0;
    }

    y = p.group(y + 8.0, "Language");
    let c = p.card(y, &[56.0, 56.0]);
    p.row_words(y, 56.0, "App language", "Menus, panels and settings", 150.0);
    p.right("English", &p.f.body, X1 - 16.0, y, 56.0, t.text);
    p.row_words(
        y + 56.0,
        56.0,
        "Prayer and drink notices",
        "\"Memasuki waktu Isya\", \"Minum dulu\"",
        150.0,
    );
    p.right(
        "Bahasa Indonesia",
        &p.f.body,
        X1 - 16.0,
        y + 56.0,
        56.0,
        t.text,
    );
    y = c + 8.0;
    p.text(
        "Changes save as you make them.",
        &p.f.small,
        X0,
        y,
        300.0,
        18.0,
        t.muted,
    );
    y + 18.0
}

fn segmented(p: &mut Painter, right: f32, cy: f32, on: ThemeMode) {
    let t = *p.t;
    let fmt = p.f.small_medium.clone();
    let fmt = &fmt;
    let ws: Vec<f32> = ThemeMode::ALL
        .iter()
        .map(|m| p.width(m.name(), fmt) + 24.0)
        .collect();
    let total: f32 = ws.iter().sum::<f32>() + 8.0;
    let r = (right - total, cy - 16.0, right, cy + 16.0);
    p.fill(r, 10.0, t.button);
    let mut x = r.0 + 4.0;
    for (m, w) in ThemeMode::ALL.iter().zip(ws) {
        let ir = (x, r.1 + 4.0, x + w, r.3 - 4.0);
        let h = Hit::Theme(*m);
        if *m == on {
            p.fill(ir, 7.0, t.surface);
            p.stroke(ir, 7.0, t.hair, 1.0);
        } else if p.hovered(h) {
            p.fill(ir, 7.0, t.button_hover);
        }
        let tw = p.width(m.name(), fmt);
        p.text(
            m.name(),
            fmt,
            x + (w - tw) / 2.0,
            ir.1,
            tw + 2.0,
            ir.3 - ir.1,
            if *m == on { t.text } else { t.muted },
        );
        p.ring(h, ir, 7.0);
        p.hit(h, ir);
        x += w;
    }
}

/// The city radio's second line: the city once it is the choice, else what the list holds.
fn city_words(city: Option<&str>, chosen: bool) -> String {
    city.filter(|_| chosen)
        .and_then(m::find_city)
        .map(|c| format!("{}, {}", c.name, c.region))
        .unwrap_or_else(|| "Any kabupaten or kota".into())
}

/// The two place choices: Windows location and a city (the Prayer section and the first run).
fn place_rows(
    p: &mut Painter,
    v: &View,
    x0: f32,
    x1: f32,
    top: f32,
    row: f32,
    compact: bool,
) -> f32 {
    let t = *p.t;
    let rows = [
        (
            Hit::PlaceWindows,
            "Windows location",
            v.locate.words(),
            v.want_windows,
        ),
        (
            Hit::PlaceCity,
            "Pick a city",
            city_words(v.city.as_deref(), !v.want_windows),
            !v.want_windows,
        ),
    ];
    let mut y = top;
    for (h, title, sub, on) in rows {
        let r = (x0, y, x1, y + row);
        if on && compact {
            p.fill(r, 8.0, t.button);
        } else if p.hovered(h) {
            p.fill(r, 8.0, if compact { t.raised } else { t.button });
        }
        p.radio(x0 + 18.0, y + row / 2.0, on);
        let room = if !compact && h == Hit::PlaceWindows && matches!(v.locate, Locate::Off) {
            150.0
        } else {
            0.0
        };
        let w = x1 - x0 - 46.0 - room;
        let mid = y + row / 2.0;
        p.text(
            title,
            if compact { &p.f.body } else { &p.f.title },
            x0 + 36.0,
            mid - 17.0,
            w,
            17.0,
            t.text,
        );
        p.text(
            &sub,
            &p.f.small,
            x0 + 36.0,
            mid + 1.0,
            w,
            16.0,
            if matches!(v.locate, Locate::Off | Locate::Failed) && h == Hit::PlaceWindows {
                t.text
            } else {
                t.muted
            },
        );
        p.ring(h, r, 8.0);
        p.hit(h, r);
        if !compact && h == Hit::PlaceWindows && matches!(v.locate, Locate::Off) {
            p.button_at(
                x1 - 12.0,
                mid,
                "Location settings",
                false,
                true,
                Some(Hit::LocationSettings),
            );
        }
        if !compact && h == Hit::PlaceCity {
            p.icon(Icon::ChevronRight, x1 - 20.0, mid, 14.0, t.muted);
        }
        y += row;
    }
    y
}

fn times_row(p: &Painter, v: &View, x0: f32, x1: f32, y: f32) {
    let t = *p.t;
    let colw = (x1 - x0) / 5.0;
    for (i, (name, at)) in v.times.iter().enumerate() {
        let cx = x0 + colw * (i as f32 + 0.5);
        let nw = p.width(name, &p.f.small);
        p.text(name, &p.f.small, cx - nw / 2.0, y, nw + 2.0, 18.0, t.muted);
        let tw = p.width(at, &p.f.title);
        p.text(
            at,
            &p.f.title,
            cx - tw / 2.0,
            y + 18.0,
            tw + 2.0,
            20.0,
            t.text,
        );
    }
}

fn prayer(p: &mut Painter, v: &View, top: f32) -> f32 {
    let t = *p.t;
    let mut y = p.group(top, "Place");
    let c = p.card(y, &[56.0, 56.0]);
    place_rows(p, v, X0, X1, y, 56.0, false);
    y = c + 16.0;

    y = p.group(y, "Method");
    let c = p.card(y, &[56.0, 56.0]);
    method_rows(p, v, y);
    y = c + 16.0;

    y = p.group(y, &v.day);
    let c = p.card(y, &[60.0]);
    times_row(p, v, X0, X1, y + 11.0);
    y = c + 16.0;

    y = p.group(y, "Adjust by minutes, to follow your mosque");
    let c = p.card(y, &[64.0]);
    adjust_row(p, v, y);
    y = c + 16.0;

    y = p.group(y, "Reminders");
    let rows = [56.0, 56.0, 56.0];
    let c = p.card(y, &rows);
    let subs = [
        "\"30 menit lagi Maghrib\", shows for a few seconds",
        "\"10 menit lagi Maghrib\", stays until you close it",
        "\"Memasuki waktu Maghrib\", always shows",
    ];
    for (i, sub) in subs.iter().enumerate() {
        let ry = y + 56.0 * i as f32;
        p.row_words(ry, 56.0, m::REMINDERS[i], sub, 50.0);
        p.toggle(
            X1 - 14.0,
            ry + 28.0,
            v.prefs.prayer_reminders[i],
            true,
            Hit::Reminder(i),
        );
    }
    y = c + 12.0;
    p.text(
        "Only presenting holds these. Focus and full screen let them through.",
        &p.f.small,
        X0,
        y,
        X1 - X0,
        18.0,
        t.muted,
    );
    y += 34.0;

    y = p.group(y, "At the time");
    y = at_time_rows(p, v, y) + 16.0;

    y = p.group(y, "Qibla");
    let c = p.card(y, &[44.0]);
    p.row_words(y, 44.0, "From here", "", 200.0);
    p.right(&v.qibla, &p.f.body, X1 - 16.0, y, 44.0, t.text);
    c + 16.0
}

/// Kemenag RI or Muhammadiyah: two radio rows in the Method card.
fn method_rows(p: &mut Painter, v: &View, top: f32) {
    let t = *p.t;
    for (i, mm) in PrayerMethod::ALL.iter().enumerate() {
        let y = top + 56.0 * i as f32;
        let r = (X0, y, X1, y + 56.0);
        let h = Hit::Method(*mm);
        if p.hovered(h) {
            p.fill(r, 8.0, t.button);
        }
        p.radio(X0 + 18.0, y + 28.0, v.prefs.prayer_method == *mm);
        let w = X1 - X0 - 50.0;
        p.text(mm.name(), &p.f.title, X0 + 36.0, y + 11.0, w, 17.0, t.text);
        p.text(
            mm.words(),
            &p.f.small,
            X0 + 36.0,
            y + 29.0,
            w,
            16.0,
            t.muted,
        );
        p.ring(h, r, 8.0);
        p.hit(h, r);
    }
}

/// A small round − or + button.
fn step_button(p: &mut Painter, cx: f32, cy: f32, label: &str, enabled: bool, h: Hit) {
    let t = *p.t;
    let r = (cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0);
    let fill = if enabled && p.hovered(h) {
        t.button_hover
    } else {
        t.button
    };
    p.fill(r, 12.0, fill);
    let fmt = &p.f.body;
    let w = p.width(label, fmt);
    let ink = if enabled {
        t.text
    } else {
        with_alpha(t.text, 0.35)
    };
    p.text(label, fmt, cx - w / 2.0, r.1, w + 2.0, 24.0, ink);
    if enabled {
        p.ring(h, r, 12.0);
        p.hit(h, r);
    }
}

/// Subuh to Isya, each with its minutes and − / + (−10 to +10).
fn adjust_row(p: &mut Painter, v: &View, top: f32) {
    let t = *p.t;
    let colw = (X1 - X0) / 5.0;
    for (i, (name, _)) in v.times.iter().enumerate() {
        let cx = X0 + colw * (i as f32 + 0.5);
        let nw = p.width(name, &p.f.small);
        p.text(
            name,
            &p.f.small,
            cx - nw / 2.0,
            top + 8.0,
            nw + 2.0,
            18.0,
            t.muted,
        );
        let val = v.prefs.prayer_adjust[i];
        let s = m::adjust_words(val);
        let sw = p.width(&s, &p.f.title);
        let cy = top + 42.0;
        p.text(
            &s,
            &p.f.title,
            cx - sw / 2.0,
            cy - 10.0,
            sw + 2.0,
            20.0,
            t.text,
        );
        step_button(
            p,
            cx - 30.0,
            cy,
            "\u{2212}",
            val > -m::ADJUST_MAX,
            Hit::Adjust(i, -1),
        );
        step_button(
            p,
            cx + 30.0,
            cy,
            "+",
            val < m::ADJUST_MAX,
            Hit::Adjust(i, 1),
        );
    }
}

/// Diam, Bunyi or Adzan, and the adzan files.
fn at_time_rows(p: &mut Painter, v: &View, top: f32) -> f32 {
    let adzan = v.prefs.at_time_sound == AtTimeSound::Adzan;
    let rows: &[f32] = if adzan { &[56.0, 56.0, 56.0] } else { &[56.0] };
    let c = p.card(top, rows);
    p.row_words(
        top,
        56.0,
        "Sound",
        "Silent in full screen and while presenting",
        250.0,
    );
    sound_segmented(p, X1 - 12.0, top + 28.0, v.prefs.at_time_sound);
    if adzan {
        let y = top + 56.0;
        let main = v
            .prefs
            .adzan_file
            .as_deref()
            .map(m::file_name)
            .unwrap_or_else(|| "Choose an mp3 or wav on this PC".into());
        p.row_words(y, 56.0, "Adzan", &main, 120.0);
        p.button_at(
            X1 - 12.0,
            y + 28.0,
            "Choose file",
            false,
            true,
            Some(Hit::PickAdzan(false)),
        );
        let y = y + 56.0;
        let subuh = v.prefs.adzan_subuh_file.as_deref();
        let words = subuh
            .map(m::file_name)
            .unwrap_or_else(|| "Optional. Uses the adzan above if empty".into());
        p.row_words(y, 56.0, "Adzan for Subuh", &words, 200.0);
        let l = p.button_at(
            X1 - 12.0,
            y + 28.0,
            "Choose file",
            false,
            true,
            Some(Hit::PickAdzan(true)),
        );
        if subuh.is_some() {
            p.button_at(
                l - 8.0,
                y + 28.0,
                "Clear",
                false,
                true,
                Some(Hit::ClearAdzanSubuh),
            );
        }
    }
    c
}

fn sound_segmented(p: &mut Painter, right: f32, cy: f32, on: AtTimeSound) {
    let t = *p.t;
    let fmt = p.f.small_medium.clone();
    let fmt = &fmt;
    let ws: Vec<f32> = AtTimeSound::ALL
        .iter()
        .map(|s| p.width(s.name(), fmt) + 24.0)
        .collect();
    let total: f32 = ws.iter().sum::<f32>() + 8.0;
    let r = (right - total, cy - 16.0, right, cy + 16.0);
    p.fill(r, 10.0, t.button);
    let mut x = r.0 + 4.0;
    for (s, w) in AtTimeSound::ALL.iter().zip(ws) {
        let ir = (x, r.1 + 4.0, x + w, r.3 - 4.0);
        let h = Hit::AtTime(*s);
        if *s == on {
            p.fill(ir, 7.0, t.surface);
            p.stroke(ir, 7.0, t.hair, 1.0);
        } else if p.hovered(h) {
            p.fill(ir, 7.0, t.button_hover);
        }
        let tw = p.width(s.name(), fmt);
        p.text(
            s.name(),
            fmt,
            x + (w - tw) / 2.0,
            ir.1,
            tw + 2.0,
            ir.3 - ir.1,
            if *s == on { t.text } else { t.muted },
        );
        p.ring(h, ir, 7.0);
        p.hit(h, ir);
        x += w;
    }
}

fn notices(p: &mut Painter, v: &View, top: f32) -> f32 {
    let t = *p.t;
    let mut y = p.group(top, "Sources");
    let rows: Vec<f32> = v.sources.iter().map(|_| 56.0).collect();
    let c = p.card(y, &rows);
    for (i, s) in v.sources.iter().enumerate() {
        let ry = y + 56.0 * i as f32;
        p.row_words(ry, 56.0, s.source.name(), &s.words, 50.0);
        p.toggle(X1 - 14.0, ry + 28.0, s.on, true, Hit::Source(s.source));
    }
    y = c + 16.0;
    y = p.group(y, "Always on");
    let c = p.card(y, &[56.0]);
    p.row_words(
        y,
        56.0,
        "Prayer",
        "\"Memasuki waktu\" always shows. Its reminders are in Prayer.",
        50.0,
    );
    p.toggle(
        X1 - 14.0,
        y + 28.0,
        true,
        false,
        Hit::Source(Source::Prayer),
    );
    y = c + 12.0;
    p.text(
        "Mutes made from the Notices list show here too.",
        &p.f.small,
        X0,
        y,
        400.0,
        18.0,
        t.muted,
    );
    y + 24.0
}

fn account(p: &mut Painter, top: f32) -> f32 {
    let t = *p.t;
    let c = p.card(top, &[64.0]);
    p.fill(
        (X0 + 14.0, top + 16.0, X0 + 46.0, top + 48.0),
        16.0,
        t.button,
    );
    p.icon(Icon::CloudOff, X0 + 30.0, top + 32.0, 15.0, t.text);
    p.text(
        "Not signed in",
        &p.f.title,
        X0 + 58.0,
        top + 13.0,
        300.0,
        18.0,
        t.text,
    );
    p.text(
        "Everything works on this PC, offline.",
        &p.f.small,
        X0 + 58.0,
        top + 33.0,
        300.0,
        17.0,
        t.muted,
    );
    p.button_at(X1 - 14.0, top + 32.0, "Sign in", false, false, None);
    let y = c + 12.0;
    p.wrap("Sync, family spaces and signed-in devices come in a later version. Nothing leaves this PC until then.", &p.f.small_wrap, X0, y, X1 - X0, 40.0, t.muted);
    y + 44.0
}

fn about(p: &mut Painter, v: &View, top: f32) -> f32 {
    let t = *p.t;
    p.text("Orby", &p.f.big, X0, top, 300.0, 36.0, t.text);
    p.text(
        &format!(
            "Version {} · orb prototype for Windows",
            env!("CARGO_PKG_VERSION")
        ),
        &p.f.small,
        X0,
        top + 38.0,
        400.0,
        18.0,
        t.muted,
    );
    let y = top + 68.0;
    let c = p.card(y, &[56.0, 56.0, 56.0, 56.0]);
    p.row_words(y, 56.0, "Your data", &v.data_dir, 130.0);
    p.button_at(
        X1 - 12.0,
        y + 28.0,
        "Open folder",
        false,
        true,
        Some(Hit::OpenData),
    );
    p.row_words(y + 56.0, 56.0, "Sync", "Off. Nothing leaves this PC.", 0.0);
    p.row_words(
        y + 112.0,
        56.0,
        "Prayer times",
        &format!(
            "Worked out on this PC with the {} method",
            v.prefs.prayer_method.name()
        ),
        0.0,
    );
    p.row_words(
        y + 168.0,
        56.0,
        "Liquid metal",
        "The drop's shader is ported from Paper Shaders, Apache-2.0",
        0.0,
    );
    c + 16.0
}

/// The search field and the list of cities inside `r`.
fn city_list(p: &mut Painter, pick: &CityPick, r: Rect, row: f32) {
    let t = *p.t;
    let field = (r.0, r.1, r.2, r.1 + 36.0);
    p.fill(field, 10.0, t.surface);
    p.stroke(field, 10.0, t.text, 1.5);
    p.icon(Icon::Search, field.0 + 18.0, field.1 + 18.0, 14.0, t.muted);
    let tx = field.0 + 34.0;
    let text_s = pick.field.text();
    if text_s.is_empty() {
        p.text(
            "Search kabupaten or kota",
            &p.f.body,
            tx + 3.0,
            field.1,
            field.2 - tx - 12.0,
            36.0,
            t.muted,
        );
    } else {
        p.text(
            text_s,
            &p.f.body,
            tx,
            field.1,
            field.2 - tx - 12.0,
            36.0,
            t.text,
        );
    }
    let cx = tx + p.width(&text_s[..pick.field.caret()], &p.f.body) + 0.5;
    p.line(cx, field.1 + 9.0, cx, field.3 - 9.0, t.text);

    let list_top = field.3 + 8.0;
    let fit = ((r.3 - list_top) / row).floor().max(1.0) as usize;
    if pick.rows.is_empty() {
        p.wrap(
            "No city by that name. Try the kabupaten's seat.",
            &p.f.small_wrap,
            r.0 + 4.0,
            list_top + 8.0,
            r.2 - r.0 - 4.0,
            40.0,
            t.muted,
        );
        return;
    }
    for (k, i) in pick.rows.iter().enumerate().skip(pick.scroll).take(fit) {
        let y = list_top + row * (k - pick.scroll) as f32;
        let rr = (r.0, y, r.2, y + row - 2.0);
        let h = Hit::CityRow(*i);
        let city = &CITIES[*i];
        if k == pick.sel {
            p.fill(rr, 8.0, t.button);
        } else if p.hovered(h) {
            p.fill(rr, 8.0, t.raised);
        }
        let nw = p.width(city.name, &p.f.title);
        p.text(
            city.name,
            &p.f.title,
            rr.0 + 12.0,
            rr.1,
            nw + 2.0,
            row - 2.0,
            t.text,
        );
        p.text(
            city.region,
            &p.f.small,
            rr.0 + 18.0 + nw,
            rr.1,
            rr.2 - rr.0 - nw - 60.0,
            row - 2.0,
            t.muted,
        );
        if pick.current == Some(city.name) {
            p.icon(Icon::Tick, rr.2 - 18.0, (rr.1 + rr.3) / 2.0, 14.0, t.text);
        }
        p.hit(h, rr);
    }
    let more = pick.rows.len().saturating_sub(pick.scroll + fit);
    if more > 0 || pick.scroll > 0 {
        // A thin thumb shows there is more to scroll.
        let span = r.3 - list_top;
        let total = pick.rows.len() as f32;
        let th = (span * fit as f32 / total).max(16.0);
        let ty = list_top + (span - th) * pick.scroll as f32 / (total - fit as f32).max(1.0);
        p.fill((r.2 + 4.0, ty, r.2 + 7.0, ty + th), 1.5, t.track);
    }
}

// ---- the first run ----

fn paint_first_run(p: &mut Painter, v: &View, step: Step) {
    let t = *p.t;
    let (x0, x1) = (20.0, FR_W - 20.0);
    p.fill((0.0, 0.0, FR_W, FR_H), 20.0, t.panel);
    p.stroke((0.0, 0.0, FR_W, FR_H), 20.0, t.hair, 1.0);
    // Step dots: the current one is a short bar.
    let mut x = x0;
    for (i, _) in Step::ALL.iter().enumerate() {
        let cur = i == step.index();
        let c = if cur || i < step.index() {
            with_alpha(t.text, if cur { 1.0 } else { 0.45 })
        } else {
            t.track
        };
        let w = if cur { 16.0 } else { 5.0 };
        p.fill((x, 25.5, x + w, 30.5), 2.5, c);
        x += w + 4.0;
    }
    p.right(
        &format!("{} of 5", step.index() + 1),
        &p.f.small,
        x1,
        18.0,
        20.0,
        t.muted,
    );
    let by = FR_H - 36.0;
    match step {
        Step::Welcome => {
            let bx = (x0, 56.0, x1, 204.0);
            p.fill(bx, 12.0, t.raised);
            p.stroke(bx, 12.0, t.hair, 1.0);
            if let Some(dc) = p.dc {
                sphere(p.g, dc, t, (bx.0 + bx.2) / 2.0, (bx.1 + bx.3) / 2.0, 52.0);
            }
            p.text(
                "Welcome to Orby",
                &p.f.heading,
                x0,
                218.0,
                x1 - x0,
                30.0,
                t.text,
            );
            p.wrap("A small metal drop at the edge of your screen. It grows a pill when something needs you, and opens a panel when you click it.", &p.f.body_wrap, x0, 254.0, x1 - x0, 80.0, t.muted);
            p.text(
                "About a minute",
                &p.f.small,
                x0,
                by - 10.0,
                120.0,
                20.0,
                t.muted,
            );
            p.button_at(x1, by, "Get started", true, true, Some(Hit::Next));
        }
        Step::Place => {
            if let Some(pick) = &v.picking {
                p.text("Pick a city", &p.f.heading, x0, 48.0, x1 - x0, 30.0, t.text);
                city_list(p, pick, (x0, 88.0, x1, by - 24.0), FR_CITY_ROW);
                p.button_at(
                    x0 + p.width("Back", &p.f.small_medium) + 26.0,
                    by,
                    "Back",
                    false,
                    true,
                    Some(Hit::CityBack),
                );
                return;
            }
            p.text(
                "Where you are",
                &p.f.heading,
                x0,
                48.0,
                x1 - x0,
                30.0,
                t.text,
            );
            p.text(
                "Worked out on this PC, offline.",
                &p.f.body,
                x0,
                80.0,
                x1 - x0,
                20.0,
                t.muted,
            );
            let y = place_rows(p, v, x0, x1, 110.0, 44.0, true);
            p.line(x0, y + 10.0, x1, y + 10.0, t.hair);
            p.text("Kemenag RI", &p.f.title, x0, y + 18.0, 200.0, 18.0, t.text);
            p.text(
                "Subuh 20°, Isya 18°, +2 min",
                &p.f.small,
                x0,
                y + 36.0,
                220.0,
                17.0,
                t.muted,
            );
            p.text(&v.day, &p.f.small, x0, y + 62.0, x1 - x0, 17.0, t.muted);
            times_row(p, v, x0 - 6.0, x1 + 6.0, y + 82.0);
            let ready = m::place_ready(v.want_windows, v.locate);
            p.button_at(x1, by, "Continue", true, ready, Some(Hit::Next));
            p.button_at(
                x0 + p.width("Back", &p.f.small_medium) + 26.0,
                by,
                "Back",
                false,
                true,
                Some(Hit::Back),
            );
        }
        Step::Shortcut => {
            p.text(
                "One shortcut",
                &p.f.heading,
                x0,
                48.0,
                x1 - x0,
                30.0,
                t.text,
            );
            p.text(
                "Shows or hides the orb from any app.",
                &p.f.body,
                x0,
                80.0,
                x1 - x0,
                20.0,
                t.muted,
            );
            let bx = (x0, 112.0, x1, 210.0);
            p.fill(bx, 12.0, t.raised);
            p.stroke(
                bx,
                12.0,
                if v.capture.is_some() { t.text } else { t.hair },
                if v.capture.is_some() { 1.5 } else { 1.0 },
            );
            let (parts, prompt) = match v.capture {
                Some(c) => {
                    let (parts, whole) = c.caps();
                    (parts, if whole { None } else { Some("press a key") })
                }
                None => (v.prefs.shortcut.parts(), None),
            };
            let pw = prompt.map_or(0.0, |s| p.width(s, &p.f.small) + 10.0);
            let cw = if parts.is_empty() {
                0.0
            } else {
                p.caps_width(&parts)
            };
            let cx = (bx.0 + bx.2) / 2.0;
            let start = cx - (cw + pw) / 2.0;
            let end = if parts.is_empty() {
                start
            } else {
                p.caps(&parts, start, 140.0)
            };
            if let Some(s) = prompt {
                p.text(s, &p.f.small, end + 8.0, 130.0, pw, 20.0, t.muted);
            }
            // A taken shortcut: the box's button picks another (and offers a free one when known),
            // so the row below has the card's whole width for its words.
            let taken = v.taken.or(if v.hotkey_ok {
                None
            } else {
                Some(v.prefs.shortcut)
            });
            if let Some(c) = v.capture {
                p.button_at(
                    cx - 4.0,
                    184.0,
                    "Cancel",
                    false,
                    true,
                    Some(Hit::CaptureCancel),
                );
                p.button_at(
                    cx + 4.0 + p.width("Save", &p.f.small_medium) + 26.0,
                    184.0,
                    "Save",
                    true,
                    c.got.is_some(),
                    Some(Hit::CaptureSave),
                );
            } else if taken.is_some() {
                let use_label = v.suggestion.map(|s| format!("Use {}", s.label()));
                let pick_w = p.width("Pick another", &p.f.small_medium) + 26.0;
                let total = pick_w
                    + use_label
                        .as_deref()
                        .map_or(0.0, |l| 8.0 + p.width(l, &p.f.small_medium) + 26.0);
                let left = cx - total / 2.0;
                p.button(
                    (left, 170.0, left + pick_w, 198.0),
                    "Pick another",
                    false,
                    true,
                    Some(Hit::PickAnother),
                );
                if let Some(l) = use_label {
                    p.button_at(
                        left + total,
                        184.0,
                        &l,
                        true,
                        true,
                        Some(Hit::UseSuggestion),
                    );
                }
            } else {
                let w = p.width("Change", &p.f.small_medium) + 26.0;
                p.button(
                    (cx - w / 2.0, 170.0, cx + w / 2.0, 198.0),
                    "Change",
                    false,
                    true,
                    Some(Hit::ChangeShortcut),
                );
            }
            // "Try it": the shortcut pressed while this step shows lands here.
            let y = 222.0;
            if let Some(s) = taken {
                p.fill((x0, y, x0 + 22.0, y + 22.0), 6.0, with_alpha(t.ember, 0.14));
                p.icon(Icon::Alert, x0 + 11.0, y + 11.0, 13.0, t.ember);
                let w = x1 - x0 - 30.0;
                p.text(
                    &format!("Couldn't use {}", s.label()),
                    &p.f.title,
                    x0 + 30.0,
                    y - 2.0,
                    w,
                    18.0,
                    t.text,
                );
                p.text(
                    "Another app uses it.",
                    &p.f.small,
                    x0 + 30.0,
                    y + 15.0,
                    w,
                    16.0,
                    t.muted,
                );
            } else {
                let r = (x0, y, x0 + 22.0, y + 22.0);
                p.fill(r, 6.0, if v.tried { t.button } else { t.raised });
                p.stroke(r, 6.0, t.hair, 1.0);
                if v.tried {
                    p.icon(Icon::Tick, x0 + 11.0, y + 11.0, 13.0, t.text);
                    let w = p.width("It works.", &p.f.title);
                    p.text("It works.", &p.f.title, x0 + 32.0, y, w + 2.0, 22.0, t.text);
                    p.text(
                        "You pressed it just now.",
                        &p.f.small,
                        x0 + 36.0 + w,
                        y,
                        160.0,
                        22.0,
                        t.muted,
                    );
                } else {
                    p.text(
                        &format!("Try it: press {}", v.prefs.shortcut.label()),
                        &p.f.small,
                        x0 + 32.0,
                        y,
                        x1 - x0 - 32.0,
                        22.0,
                        t.muted,
                    );
                }
            }
            p.line(x0, 258.0, x1, 258.0, t.hair);
            p.text(
                "Start with Windows",
                &p.f.title,
                x0,
                268.0,
                190.0,
                18.0,
                t.text,
            );
            p.text(
                "Opens quietly when you sign in",
                &p.f.small,
                x0,
                286.0,
                190.0,
                17.0,
                t.muted,
            );
            p.toggle(x1, 286.0, v.fr_start, true, Hit::StartWithWindows);
            p.button_at(
                x1,
                by,
                "Continue",
                true,
                v.capture.is_none(),
                Some(Hit::Next),
            );
            p.button_at(
                x0 + p.width("Back", &p.f.small_medium) + 26.0,
                by,
                "Back",
                false,
                true,
                Some(Hit::Back),
            );
        }
        Step::Sync => {
            let tw = p.width("Family sync", &p.f.heading);
            p.text(
                "Family sync",
                &p.f.heading,
                x0,
                48.0,
                tw + 2.0,
                30.0,
                t.text,
            );
            let ow = p.width("Optional", &p.f.small) + 16.0;
            p.fill(
                (x0 + tw + 8.0, 53.0, x0 + tw + 8.0 + ow, 73.0),
                10.0,
                t.button,
            );
            p.text(
                "Optional",
                &p.f.small,
                x0 + tw + 16.0,
                53.0,
                ow,
                20.0,
                t.muted,
            );
            p.wrap(
                "Share task spaces with family and keep your PCs in step.",
                &p.f.small_wrap,
                x0,
                84.0,
                x1 - x0,
                36.0,
                t.muted,
            );
            p.button(
                (x0, 124.0, x1, 156.0),
                "Continue with Google",
                false,
                false,
                None,
            );
            p.line(x0, 172.0, (x0 + x1) / 2.0 - 16.0, 172.0, t.hair);
            p.line((x0 + x1) / 2.0 + 16.0, 172.0, x1, 172.0, t.hair);
            let ow = p.width("or", &p.f.small);
            p.text(
                "or",
                &p.f.small,
                (x0 + x1) / 2.0 - ow / 2.0,
                162.0,
                ow + 2.0,
                20.0,
                t.muted,
            );
            let fr = (x0, 188.0, x1, 220.0);
            p.stroke(fr, 16.0, t.hair, 1.0);
            p.text(
                "you@example.com",
                &p.f.body,
                x0 + 16.0,
                fr.1,
                200.0,
                32.0,
                with_alpha(t.muted, 0.6),
            );
            p.button(
                (x0, 228.0, x1, 260.0),
                "Email me a sign-in link",
                true,
                false,
                None,
            );
            p.wrap("Sync is off in this version. Skip it and everything still works on this PC, offline.", &p.f.small_wrap, x0, 272.0, x1 - x0, 50.0, t.muted);
            p.button_at(x1, by, "Skip for now", false, true, Some(Hit::Next));
            p.button_at(
                x0 + p.width("Back", &p.f.small_medium) + 26.0,
                by,
                "Back",
                false,
                true,
                Some(Hit::Back),
            );
        }
        Step::Done => {
            p.text("You're set", &p.f.heading, x0, 48.0, x1 - x0, 30.0, t.text);
            let place = v
                .city
                .as_deref()
                .map(str::to_string)
                .filter(|_| !v.want_windows)
                .unwrap_or_else(|| v.prefs.place.short());
            let rows = [
                (Icon::Pin, place, "Kemenag RI".to_string()),
                (
                    Icon::Keyboard,
                    v.prefs.shortcut.label(),
                    "show or hide".to_string(),
                ),
                (
                    Icon::CloudOff,
                    "Offline".to_string(),
                    "sign in from Settings".to_string(),
                ),
            ];
            for (i, (icon, a, b)) in rows.iter().enumerate() {
                let y = 96.0 + 36.0 * i as f32;
                p.fill((x0, y, x0 + 26.0, y + 26.0), 7.0, t.raised);
                p.stroke((x0, y, x0 + 26.0, y + 26.0), 7.0, t.hair, 1.0);
                p.icon(*icon, x0 + 13.0, y + 13.0, 14.0, t.text);
                let w = p.width(a, &p.f.title);
                p.text(a, &p.f.title, x0 + 36.0, y, w + 2.0, 26.0, t.text);
                p.text(
                    b,
                    &p.f.body,
                    x0 + 41.0 + w,
                    y,
                    x1 - x0 - 46.0 - w,
                    26.0,
                    t.muted,
                );
            }
            if v.fr_start {
                p.text(
                    "Starts with Windows.",
                    &p.f.small,
                    x0,
                    214.0,
                    x1 - x0,
                    18.0,
                    t.muted,
                );
            }
            p.button_at(x1, by, "Finish", true, true, Some(Hit::Finish));
            p.button_at(
                x0 + p.width("Back", &p.f.small_medium) + 26.0,
                by,
                "Back",
                false,
                true,
                Some(Hit::Back),
            );
        }
    }
}

/// The welcome card's picture of the drop: a neutral chrome sphere (no colour, no gold).
fn sphere(g: &Shared, dc: &ID2D1DeviceContext, t: Theme, cx: f32, cy: f32, r: f32) {
    let _ = g;
    let stops = [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: rgba(0xFFFFFF, 1.0),
        },
        D2D1_GRADIENT_STOP {
            position: 0.28,
            color: rgba(0xD9D9DE, 1.0),
        },
        D2D1_GRADIENT_STOP {
            position: 0.62,
            color: rgba(0x4A4A52, 1.0),
        },
        D2D1_GRADIENT_STOP {
            position: 0.86,
            color: rgba(0x17171B, 1.0),
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: rgba(0x8A8A92, 1.0),
        },
    ];
    unsafe {
        let rt: &ID2D1RenderTarget = dc;
        let Ok(coll) =
            rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        else {
            return;
        };
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
            center: pt(cx, cy),
            gradientOriginOffset: pt(-r * 0.38, -r * 0.42),
            radiusX: r,
            radiusY: r,
        };
        let Ok(b) = dc.CreateRadialGradientBrush(&props, None, &coll) else {
            return;
        };
        dc.FillEllipse(&ellipse(cx, cy, r), &b);
        // The window's highlight and the floor's reflection, as on the drop.
        dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: pt(cx + r * 0.42, cy - r * 0.5),
                radiusX: r * 0.2,
                radiusY: r * 0.12,
            },
            &brush(dc, rgba(0xFFFFFF, 0.92)),
        );
        dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: pt(cx - r * 0.5, cy + r * 0.62),
                radiusX: r * 0.16,
                radiusY: r * 0.1,
            },
            &brush(dc, rgba(0xFFFFFF, 0.55)),
        );
        dc.DrawEllipse(&ellipse(cx, cy, r - 0.5), &brush(dc, t.hair), 1.0, None);
    }
}

/// The city list's rows that fit, so the keyboard can keep the selection in view.
pub fn city_rows_fit(first_run: bool) -> usize {
    if first_run {
        ((FR_H - 36.0 - 24.0 - (88.0 + 36.0 + 8.0)) / FR_CITY_ROW).floor() as usize
    } else {
        ((H - 20.0 - (CONTENT_TOP + 4.0 + 36.0 + 8.0)) / CITY_ROW).floor() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_city_list_shows_whole_rows_only() {
        assert_eq!(city_rows_fit(false), 11);
        assert_eq!(city_rows_fit(true), 6);
    }

    #[test]
    fn a_hit_takes_the_topmost_control() {
        let hits = vec![
            (Hit::PlaceCity, (0.0, 0.0, 100.0, 50.0)),
            (Hit::LocationSettings, (60.0, 10.0, 90.0, 40.0)),
        ];
        assert_eq!(hit_at(&hits, 70.0, 20.0), Some(Hit::LocationSettings));
        assert_eq!(hit_at(&hits, 10.0, 20.0), Some(Hit::PlaceCity));
        assert_eq!(hit_at(&hits, 10.0, 60.0), None);
    }

    #[test]
    fn the_city_radio_names_a_city_only_once_it_is_the_choice() {
        assert_eq!(
            city_words(Some("Yogyakarta"), false),
            "Any kabupaten or kota"
        );
        assert_eq!(
            city_words(Some("Makassar"), true),
            "Makassar, Sulawesi Selatan"
        );
        assert_eq!(city_words(None, true), "Any kabupaten or kota");
    }
}
