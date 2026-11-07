#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Disk panel (frames "v6.2 · 13" and "v6.2 · 14"): the drive list, Scanning, Results with
//! the treemap, folder list, Files and Types, Safe to clean, a suggestion's Review list, the state
//! cards and the confirm sheet. Pure drawing and hit testing over a `View` that disk.rs builds.

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::disk_model::{tile_alpha, Rect, Tile};
use crate::gfx::*;
use crate::tasks_panel::clipped_text;
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

pub const W_SMALL: f32 = 400.0;
pub const W_REVIEW: f32 = 404.0;
pub const W_RESULTS: f32 = 756.0;
const X0: f32 = 16.0;
const SCAN_H: f32 = 324.0;
const RESULTS_H: f32 = 524.0;
const ROW: f32 = 36.0;
const FILE_ROW: f32 = 40.0;
/// Folder rows beside the treemap, then the tail row.
pub const LIST_ROWS: usize = 6;
pub const FILES_ROWS: usize = 7;
pub const REVIEW_ROWS: usize = 9;
const MAP: Rect = (16.0, 166.0, 352.0, 418.0);
const LIST: Rect = (368.0, 166.0, 740.0, 418.0);
const SEL_H: f32 = 68.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Folders,
    Files,
    Types,
}

pub struct DriveRow {
    pub name: String,
    pub free: String,
    pub used: f32,
}

pub struct ScanningView {
    pub title: String,
    pub big: String,
    pub of: String,
    /// None for a folder: its size is not known before the scan.
    pub frac: Option<f32>,
    pub files: String,
    pub folders: String,
    pub took: String,
    pub current: String,
    /// Name, bytes, size label.
    pub biggest: Vec<(String, u64, String)>,
}

pub struct ListRow {
    pub name: String,
    pub bytes: u64,
    pub size: String,
    pub pct: f64,
    /// Against the biggest sibling, 0..1.
    pub bar: f32,
    pub dir: bool,
    pub hidden: bool,
}

pub struct FileRow {
    pub name: String,
    /// Muted after the name: the folder below the scanned place.
    pub sub: String,
    pub month: String,
    pub size: String,
    pub icon: Icon,
    pub checked: bool,
}

pub struct TypeRow {
    pub label: &'static str,
    pub bytes: u64,
    pub size: String,
    pub files: String,
    /// Shade by rank; 5 is Other, the palest, drawn with a hairline.
    pub grey: usize,
}

pub struct ResultsView {
    pub folder: bool,
    pub title: String,
    pub sub: String,
    pub used: f32,
    pub used_text: String,
    pub used_pct: String,
    pub free_text: String,
    pub crumbs: Vec<String>,
    pub here_size: String,
    pub here: String,
    pub tab: Tab,
    pub rows: Vec<ListRow>,
    pub first: usize,
    pub sel: Option<usize>,
    pub tiles: Vec<Tile>,
    pub types: Vec<TypeRow>,
    pub files: Vec<FileRow>,
    pub files_first: usize,
    pub files_count: String,
    pub selected: Option<(String, String)>,
    pub can_recycle: bool,
}

pub struct Suggestion {
    pub icon: Icon,
    pub title: String,
    pub body: String,
    pub check_first: bool,
    pub size: String,
    pub can_review: bool,
}

pub struct CleanView {
    pub place: String,
    pub total: String,
    pub rows: Vec<Suggestion>,
    pub bin: String,
    pub nothing: Option<String>,
}

pub struct ReviewView {
    pub title: String,
    pub sub: String,
    pub rows: Vec<FileRow>,
    pub first: usize,
    pub selected: Option<(String, String)>,
    pub can_recycle: bool,
    pub all: bool,
}

pub struct ConfirmView {
    pub title: String,
    pub items: Vec<(Icon, String, String)>,
    pub more: Option<String>,
    pub body: &'static str,
    pub honest: String,
    /// Keyboard focus is on Move (it starts on Cancel).
    pub on_move: bool,
}

pub struct Card {
    pub icon: Icon,
    pub ember: bool,
    pub title: String,
    pub body: String,
    pub button: Option<String>,
}

pub enum Screen {
    Start(Vec<DriveRow>),
    Scanning(ScanningView),
    Results(ResultsView),
    Clean(CleanView),
    Review(ReviewView),
}

pub struct View {
    pub screen: Screen,
    pub cards: Vec<Card>,
    pub confirm: Option<ConfirmView>,
    /// The keyboard's row, drawn with a focus ring after a key press.
    pub cursor: Option<usize>,
    pub kb: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    ScanDrive(usize),
    ScanFolder,
    OpenClean,
    Stop,
    ScanAgain,
    Back,
    Crumb(usize),
    Tab(Tab),
    Tile(usize),
    Row(usize),
    RowOpen(usize),
    RowRecycle(usize),
    File(usize),
    SelOpen,
    SelRecycle,
    SelectAll,
    Review(usize),
    BinOpen,
    Settings,
    ReviewFolder,
    Card(usize),
    Cancel,
    Move,
    /// Inside the confirm sheet, on nothing: swallowed so it does not reach the panel under it.
    Sheet,
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

fn width(v: &View) -> f32 {
    match &v.screen {
        Screen::Results(_) => W_RESULTS,
        Screen::Review(_) => W_REVIEW,
        _ => W_SMALL,
    }
}

/// Height of wrapped text at this width.
fn wrap_h(g: &Shared, s: &str, fmt: &IDWriteTextFormat, w: f32) -> f32 {
    let t: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        match g.dw.CreateTextLayout(&t, fmt, w.max(1.0), 1000.0) {
            Ok(l) => {
                let mut m = DWRITE_TEXT_METRICS::default();
                let _ = l.GetMetrics(&mut m);
                m.height.ceil()
            }
            Err(_) => 16.0,
        }
    }
}

// ---- heights ----

fn start_h(n: usize) -> f32 {
    52.0 + n as f32 * 60.0 + 12.0 + 28.0 + 16.0
}

fn clean_rows_top() -> f32 {
    92.0
}

const SUGGESTION_H: f32 = 104.0;
const EXTRA_H: f32 = 76.0;

fn clean_h(c: &CleanView) -> f32 {
    let rows = if c.nothing.is_some() {
        84.0
    } else {
        c.rows.len() as f32 * SUGGESTION_H
    };
    clean_rows_top() + rows + 30.0 + 2.0 * EXTRA_H + 52.0
}

fn review_list_top() -> f32 {
    80.0
}

fn review_h(r: &ReviewView) -> f32 {
    let n = r.rows.len().clamp(1, REVIEW_ROWS);
    review_list_top() + n as f32 * FILE_ROW + 8.0 + SEL_H
}

fn base_h(v: &View) -> f32 {
    match &v.screen {
        Screen::Start(d) => start_h(d.len()),
        Screen::Scanning(_) => SCAN_H,
        Screen::Results(_) => RESULTS_H,
        Screen::Clean(c) => clean_h(c),
        Screen::Review(r) => review_h(r),
    }
}

fn card_h(g: &Shared, f: &Fonts, c: &Card, w: f32) -> f32 {
    let body = wrap_h(g, &c.body, &f.small_wrap, w - X0 * 2.0 - 64.0);
    12.0 + 18.0 + 4.0 + body + if c.button.is_some() { 10.0 + 28.0 } else { 0.0 } + 12.0
}

/// Each card's top and height, stacked under the screen.
fn card_tops(g: &Shared, f: &Fonts, v: &View) -> Vec<(f32, f32)> {
    let w = width(v);
    let mut y = base_h(v);
    v.cards
        .iter()
        .map(|c| {
            let h = card_h(g, f, c, w);
            let r = (y, h);
            y += h + 8.0;
            r
        })
        .collect()
}

fn sheet_w(v: &View) -> f32 {
    (width(v) - 32.0).min(340.0)
}

fn sheet_h(g: &Shared, f: &Fonts, v: &View, c: &ConfirmView) -> f32 {
    let w = sheet_w(v) - 36.0;
    16.0 + 32.0
        + 12.0
        + well_h(c)
        + 12.0
        + wrap_h(g, c.body, &f.body_wrap, w)
        + 8.0
        + wrap_h(g, &c.honest, &f.small_wrap, w)
        + 16.0
        + 36.0
        + 16.0
}

fn well_h(c: &ConfirmView) -> f32 {
    8.0 + c.items.len() as f32 * ROW + if c.more.is_some() { 28.0 } else { 0.0 }
}

/// The panel's logical size.
pub fn size(g: &Shared, f: &Fonts, v: &View) -> (f32, f32) {
    let w = width(v);
    let tops = card_tops(g, f, v);
    let mut h = tops.last().map_or(base_h(v), |(y, hh)| y + hh + 12.0);
    if let Some(c) = &v.confirm {
        h = h.max(sheet_h(g, f, v, c) + 32.0);
    }
    (w, h.ceil())
}

// ---- layout (hit rects) ----

fn btn_w(g: &Shared, f: &Fonts, label: &str, icon: bool) -> f32 {
    g.text_width(label, &f.small_medium) + 24.0 + if icon { 20.0 } else { 0.0 }
}

fn start_rects(g: &Shared, f: &Fonts, n: usize) -> Vec<(Hit, Rect)> {
    let x1 = W_SMALL - X0;
    let mut out = Vec::new();
    for i in 0..n {
        let t = 52.0 + i as f32 * 60.0;
        out.push((Hit::ScanDrive(i), (X0, t, x1, t + 56.0)));
    }
    let y = 52.0 + n as f32 * 60.0 + 12.0;
    let w1 = btn_w(g, f, "Scan a folder", true);
    out.insert(0, (Hit::ScanFolder, (X0, y, X0 + w1, y + 28.0)));
    let w2 = btn_w(g, f, "Safe to clean", true);
    out.insert(
        0,
        (
            Hit::OpenClean,
            (X0 + w1 + 8.0, y, X0 + w1 + 8.0 + w2, y + 28.0),
        ),
    );
    out
}

fn header_buttons(g: &Shared, f: &Fonts) -> [(Hit, Rect); 2] {
    let x1 = W_RESULTS - X0;
    let w1 = btn_w(g, f, "Scan again", true);
    let w2 = btn_w(g, f, "Safe to clean", true);
    [
        (Hit::ScanAgain, (x1 - w1, 18.0, x1, 46.0)),
        (
            Hit::OpenClean,
            (x1 - w1 - 8.0 - w2, 18.0, x1 - w1 - 8.0, 46.0),
        ),
    ]
}

fn tabs_rects(g: &Shared, f: &Fonts) -> [(Tab, Rect); 3] {
    let x1 = W_RESULTS - X0;
    let ws: Vec<f32> = ["Folders", "Files", "Types"]
        .iter()
        .map(|l| g.text_width(l, &f.title) + 28.0)
        .collect();
    let total: f32 = ws.iter().sum::<f32>() + 8.0;
    let mut x = x1 - total + 4.0;
    let mut out = [(Tab::Folders, (0.0, 0.0, 0.0, 0.0)); 3];
    for (i, tab) in [Tab::Folders, Tab::Files, Tab::Types]
        .into_iter()
        .enumerate()
    {
        out[i] = (tab, (x, 106.0, x + ws[i], 134.0));
        x += ws[i];
    }
    out
}

/// The breadcrumb's pieces: (index into crumbs, or None for the ellipsis, x, width).
fn crumb_layout(
    g: &Shared,
    f: &Fonts,
    r: &ResultsView,
    right: f32,
) -> Vec<(Option<usize>, f32, f32)> {
    let n = r.crumbs.len();
    let wid = |i: usize| g.text_width(&r.crumbs[i], if i + 1 == n { &f.title } else { &f.body });
    let sep = 20.0;
    let size_w = g.text_width(&r.here_size, &f.body) + 8.0;
    let mut from = 0;
    loop {
        let mut w: f32 = (from..n).map(|i| wid(i) + sep).sum::<f32>() + size_w;
        if from > 0 {
            w += g.text_width("…", &f.body) + sep;
        }
        if 56.0 + w <= right || from + 1 >= n {
            break;
        }
        from += 1;
    }
    let mut x = 56.0;
    let mut out = Vec::new();
    if from > 0 {
        let w = g.text_width("…", &f.body);
        out.push((None, x, w));
        x += w + sep;
    }
    for i in from..n {
        let w = wid(i);
        out.push((Some(i), x, w));
        x += w + sep;
    }
    out
}

fn sel_bar_rects(x0: f32, x1: f32, top: f32) -> [(Hit, Rect); 2] {
    let mid = (x0 + x1) / 2.0;
    [
        (Hit::SelOpen, (x0, top + 36.0, mid - 4.0, top + 64.0)),
        (Hit::SelRecycle, (mid + 4.0, top + 36.0, x1, top + 64.0)),
    ]
}

/// The two icon buttons a hovered or focused folder row gets, in place of its bar.
fn row_buttons(top: f32) -> [(bool, Rect); 2] {
    let right = LIST.2 - 8.0 - 34.0 - 64.0 - 12.0;
    [
        (false, (right - 64.0, top + 4.0, right - 36.0, top + 32.0)),
        (true, (right - 30.0, top + 4.0, right - 2.0, top + 32.0)),
    ]
}

fn results_rects(g: &Shared, f: &Fonts, r: &ResultsView) -> Vec<(Hit, Rect)> {
    let mut out: Vec<(Hit, Rect)> = header_buttons(g, f).to_vec();
    out.push((Hit::Back, (16.0, 106.0, 44.0, 134.0)));
    let tabs = tabs_rects(g, f);
    for (i, x, w) in crumb_layout(g, f, r, tabs[0].1 .0 - 12.0) {
        if let Some(i) = i {
            out.push((Hit::Crumb(i), (x - 2.0, 108.0, x + w + 2.0, 132.0)));
        }
    }
    for (tab, rect) in tabs {
        out.push((Hit::Tab(tab), rect));
    }
    match r.tab {
        Tab::Folders => {
            for (i, t) in r.tiles.iter().enumerate() {
                let _ = i;
                if let Some(row) = t.row {
                    out.push((Hit::Tile(row), t.rect));
                }
            }
            for (k, i) in (r.first..r.rows.len().min(r.first + LIST_ROWS)).enumerate() {
                let top = LIST.1 + k as f32 * ROW;
                let [(_, open), (_, bin)] = row_buttons(top);
                out.push((Hit::RowOpen(i), open));
                if r.can_recycle {
                    out.push((Hit::RowRecycle(i), bin));
                }
                out.push((Hit::Row(i), (LIST.0, top, LIST.2, top + ROW)));
            }
        }
        Tab::Files => {
            for (k, i) in (r.files_first..r.files.len().min(r.files_first + FILES_ROWS)).enumerate()
            {
                let top = 166.0 + k as f32 * FILE_ROW;
                out.push((Hit::File(i), (X0, top, W_RESULTS - X0, top + ROW)));
            }
            out.extend(sel_bar_rects(X0, W_RESULTS - X0, RESULTS_H - 8.0 - SEL_H));
        }
        Tab::Types => {}
    }
    out
}

fn clean_rects(g: &Shared, f: &Fonts, c: &CleanView) -> Vec<(Hit, Rect)> {
    let x1 = W_SMALL - X0;
    let mut out = Vec::new();
    let rw = btn_w(g, f, "Review", false);
    if c.nothing.is_none() {
        for (i, s) in c.rows.iter().enumerate() {
            let t = clean_rows_top() + i as f32 * SUGGESTION_H;
            if s.can_review {
                out.push((Hit::Review(i), (x1 - rw, t + 36.0, x1, t + 64.0)));
            }
        }
    }
    let t = extras_top(c);
    let ow = btn_w(g, f, "Open", false);
    out.push((Hit::BinOpen, (x1 - ow, t + 34.0, x1, t + 62.0)));
    let sw = btn_w(g, f, "Settings", false);
    out.push((
        Hit::Settings,
        (x1 - sw, t + EXTRA_H + 12.0, x1, t + EXTRA_H + 40.0),
    ));
    out.push((Hit::Back, (x1 - 28.0, 14.0, x1, 42.0)));
    out
}

fn extras_top(c: &CleanView) -> f32 {
    let rows = if c.nothing.is_some() {
        84.0
    } else {
        c.rows.len() as f32 * SUGGESTION_H
    };
    clean_rows_top() + rows + 30.0
}

fn review_rects(r: &ReviewView, g: &Shared, f: &Fonts) -> Vec<(Hit, Rect)> {
    let x1 = W_REVIEW - X0;
    let mut out = vec![
        (Hit::Back, (X0, 16.0, X0 + 28.0, 44.0)),
        (Hit::ReviewFolder, (x1 - 28.0, 16.0, x1, 44.0)),
    ];
    let aw = g.text_width(if r.all { "Select none" } else { "Select all" }, &f.small) + 12.0;
    out.push((Hit::SelectAll, (x1 - aw, 50.0, x1, 70.0)));
    for (k, i) in (r.first..r.rows.len().min(r.first + REVIEW_ROWS)).enumerate() {
        let top = review_list_top() + k as f32 * FILE_ROW;
        out.push((Hit::File(i), (X0, top, x1, top + ROW)));
    }
    out.extend(sel_bar_rects(X0, x1, review_h(r) - SEL_H));
    out
}

fn sheet_rect(g: &Shared, f: &Fonts, v: &View, c: &ConfirmView) -> Rect {
    let (w, h) = size(g, f, v);
    let (sw, sh) = (sheet_w(v), sheet_h(g, f, v, c));
    let l = (w - sw) / 2.0;
    let t = ((h - sh) / 2.0).max(16.0);
    (l, t, l + sw, t + sh)
}

fn sheet_buttons(g: &Shared, f: &Fonts, s: Rect) -> [(Hit, Rect); 2] {
    let mw = g.text_width("Move to Recycle Bin", &f.title) + 28.0;
    let cw = g.text_width("Cancel", &f.title) + 28.0;
    let (b, r) = (s.3 - 16.0, s.2 - 18.0);
    [
        (Hit::Cancel, (r - mw - 8.0 - cw, b - 36.0, r - mw - 8.0, b)),
        (Hit::Move, (r - mw, b - 36.0, r, b)),
    ]
}

fn layout(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    if let Some(c) = &v.confirm {
        let s = sheet_rect(g, f, v, c);
        let mut out: Vec<(Hit, Rect)> = sheet_buttons(g, f, s).to_vec();
        out.push((Hit::Sheet, (0.0, 0.0, 10_000.0, 10_000.0)));
        return out;
    }
    let mut out = match &v.screen {
        Screen::Start(d) => start_rects(g, f, d.len()),
        Screen::Scanning(_) => {
            let w = btn_w(g, f, "Stop", true);
            vec![(Hit::Stop, (W_SMALL - X0 - w, 14.0, W_SMALL - X0, 42.0))]
        }
        Screen::Results(r) => results_rects(g, f, r),
        Screen::Clean(c) => clean_rects(g, f, c),
        Screen::Review(r) => review_rects(r, g, f),
    };
    let w = width(v);
    for (i, ((top, h), c)) in card_tops(g, f, v).into_iter().zip(&v.cards).enumerate() {
        if let Some(label) = &c.button {
            let bw = btn_w(g, f, label, false);
            let x = X0 + 48.0;
            out.push((
                Hit::Card(i),
                (x, top + h - 12.0 - 28.0, x + bw, top + h - 12.0),
            ));
        }
        let _ = w;
    }
    out
}

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    layout(g, f, v)
        .into_iter()
        .find(|(_, r)| inside(r, x, y))
        .map(|(h, _)| h)
}

// ---- drawing helpers ----

fn ink_on(t: &Theme) -> D2D1_COLOR_F {
    t.text
}

fn button(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: Rect,
    label: &str,
    icon: Option<Icon>,
    primary: bool,
    hover: bool,
    enabled: bool,
) {
    let a = if enabled { 1.0 } else { 0.4 };
    let fill = if primary {
        with_alpha(t.text, if hover && enabled { 0.85 } else { a })
    } else if hover && enabled {
        t.button_hover
    } else {
        t.button
    };
    let ink = if primary {
        t.panel
    } else {
        with_alpha(t.text, a)
    };
    let h = r.3 - r.1;
    unsafe { dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, h / 2.0), &brush(dc, fill)) };
    let fnt = if h >= 34.0 { &f.title } else { &f.small_medium };
    let tw = g.text_width(label, fnt);
    let iw = if icon.is_some() { 20.0 } else { 0.0 };
    let x = (r.0 + r.2) / 2.0 - (tw + iw) / 2.0;
    let b = brush(dc, ink);
    if let Some(i) = icon {
        draw_icon(g, dc, i, x + 7.0, (r.1 + r.3) / 2.0, 13.0, &b);
    }
    text(dc, label, fnt, x + iw, r.1, tw + 2.0, h, &b);
}

fn icon_button(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, r: Rect, icon: Icon, hover: bool) {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    unsafe {
        dc.FillEllipse(
            &ellipse(cx, cy, (r.2 - r.0) / 2.0),
            &brush(dc, if hover { t.button_hover } else { t.button }),
        )
    };
    draw_icon(g, dc, icon, cx, cy, 14.0, &brush(dc, t.text));
}

/// The 2 px keyboard ring, 2 px outside the shape.
fn focus_ring(dc: &ID2D1DeviceContext, t: &Theme, r: Rect, radius: f32) {
    unsafe {
        dc.DrawRoundedRectangle(
            &rrect(r.0 - 2.0, r.1 - 2.0, r.2 + 2.0, r.3 + 2.0, radius + 2.0),
            &brush(dc, ink_on(t)),
            2.0,
            None,
        )
    };
}

fn chip(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    cx: f32,
    cy: f32,
    r: f32,
    icon: Icon,
    ember: bool,
) {
    let (fill, ink) = if ember {
        (with_alpha(t.ember, 0.14), t.ember)
    } else {
        (t.chip, t.text)
    };
    unsafe { dc.FillEllipse(&ellipse(cx, cy, r), &brush(dc, fill)) };
    draw_icon(g, dc, icon, cx, cy, (r * 0.9).min(16.0), &brush(dc, ink));
}

/// The 24 px rounded-square chip of a list row.
fn square_chip(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, x: f32, cy: f32, icon: Icon) {
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(x, cy - 12.0, x + 24.0, cy + 12.0, 7.0),
            &brush(dc, t.chip),
        )
    };
    draw_icon(g, dc, icon, x + 12.0, cy, 14.0, &brush(dc, t.text));
}

fn checkbox(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, x: f32, cy: f32, on: bool) {
    let r = rrect(x, cy - 8.0, x + 16.0, cy + 8.0, 5.0);
    unsafe {
        if on {
            dc.FillRoundedRectangle(&r, &brush(dc, t.text));
            draw_icon(g, dc, Icon::Tick, x + 8.0, cy, 13.0, &brush(dc, t.panel));
        } else {
            let edge = with_alpha(t.text, if t.dark { 0.5 } else { 0.45 });
            dc.DrawRoundedRectangle(
                &rrect(x + 0.75, cy - 7.25, x + 15.25, cy + 7.25, 4.5),
                &brush(dc, edge),
                1.5,
                None,
            );
        }
    }
}

fn bar(dc: &ID2D1DeviceContext, t: &Theme, x: f32, y: f32, w: f32, h: f32, frac: f32) {
    unsafe {
        dc.FillRoundedRectangle(&rrect(x, y, x + w, y + h, h / 2.0), &brush(dc, t.track));
        let fw = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
        if fw > 0.0 {
            dc.FillRoundedRectangle(&rrect(x, y, x + fw, y + h, h / 2.0), &brush(dc, t.text));
        }
    }
}

fn panel_bg(dc: &ID2D1DeviceContext, t: &Theme, w: f32, h: f32) {
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, w, h, 20.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(
            &rrect(0.5, 0.5, w - 0.5, h - 0.5, 19.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
}

fn hairline(dc: &ID2D1DeviceContext, t: &Theme, x0: f32, x1: f32, y: f32) {
    unsafe {
        dc.DrawLine(
            pt(x0, y + 0.5),
            pt(x1, y + 0.5),
            &brush(dc, t.hair),
            1.0,
            None,
        )
    };
}

/// The six greys of the type bar, biggest first; Other is always the lightest.
fn type_grey(t: &Theme, i: usize) -> D2D1_COLOR_F {
    let light = [0x171717, 0x525252, 0x8A8A8A, 0xB5B5B5, 0xD4D4D4, 0xE5E5E5];
    let dark = [0xFAFAFA, 0xBDBDBD, 0x8A8A8A, 0x5C5C5C, 0x3D3D3D, 0x2B2B2B];
    rgba(
        if t.dark {
            dark[i.min(5)]
        } else {
            light[i.min(5)]
        },
        1.0,
    )
}

// ---- screens ----

fn draw_start(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    drives: &[DriveRow],
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let x1 = W_SMALL - X0;
    text(dc, "Disk", &f.panel_title, X0, 16.0, 200.0, 18.0, &ink);
    right_text(
        g,
        dc,
        "What is taking the room",
        &f.small,
        x1,
        17.0,
        16.0,
        &muted,
    );
    for (i, d) in drives.iter().enumerate() {
        let top = 52.0 + i as f32 * 60.0;
        let on = hover == Some(Hit::ScanDrive(i));
        if on {
            unsafe {
                dc.FillRoundedRectangle(&rrect(X0, top, x1, top + 56.0, 10.0), &brush(dc, t.raised))
            };
        }
        if v.kb && v.cursor == Some(i) {
            focus_ring(dc, t, (X0, top, x1, top + 56.0), 10.0);
        }
        chip(
            g,
            dc,
            t,
            X0 + 22.0,
            top + 22.0,
            16.0,
            Icon::HardDrive,
            false,
        );
        let bw = btn_w(g, f, "Scan", false);
        clipped_text(
            g,
            dc,
            &d.name,
            &f.title,
            X0 + 48.0,
            top + 6.0,
            x1 - X0 - 60.0 - bw,
            18.0,
            &ink,
        );
        text(
            dc,
            &d.free,
            &f.small,
            X0 + 48.0,
            top + 24.0,
            260.0,
            16.0,
            &muted,
        );
        bar(
            dc,
            t,
            X0 + 48.0,
            top + 45.0,
            x1 - X0 - 56.0 - bw - 12.0,
            4.0,
            d.used,
        );
        button(
            g,
            dc,
            t,
            f,
            (x1 - 8.0 - bw, top + 14.0, x1 - 8.0, top + 42.0),
            "Scan",
            None,
            false,
            on,
            true,
        );
    }
    let y = 52.0 + drives.len() as f32 * 60.0 + 12.0;
    for (h, r) in start_rects(g, f, drives.len()).into_iter().take(2) {
        let (label, icon) = if h == Hit::ScanFolder {
            ("Scan a folder", Icon::Folder)
        } else {
            ("Safe to clean", Icon::Shield)
        };
        button(
            g,
            dc,
            t,
            f,
            (r.0, y, r.2, y + 28.0),
            label,
            Some(icon),
            false,
            hover == Some(h),
            true,
        );
    }
}

fn draw_scanning(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    s: &ScanningView,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let x1 = W_SMALL - X0;
    clipped_text(g, dc, &s.title, &f.panel_title, X0, 16.0, 260.0, 22.0, &ink);
    let w = btn_w(g, f, "Stop", true);
    button(
        g,
        dc,
        t,
        f,
        (x1 - w, 14.0, x1, 42.0),
        "Stop",
        Some(Icon::Stop),
        false,
        hover == Some(Hit::Stop),
        true,
    );
    let bw = g.text_width(&s.big, &f.big);
    text(dc, &s.big, &f.big, X0, 50.0, bw + 4.0, 34.0, &ink);
    clipped_text(
        g,
        dc,
        &s.of,
        &f.small,
        X0 + bw + 8.0,
        60.0,
        x1 - X0 - bw - 8.0,
        18.0,
        &muted,
    );
    match s.frac {
        Some(fr) => bar(dc, t, X0, 92.0, x1 - X0, 4.0, fr),
        None => unsafe {
            dc.DrawLine(
                pt(X0, 94.0),
                pt(x1, 94.0),
                &brush(dc, t.track),
                1.5,
                &dotted_stroke(g),
            )
        },
    }
    let cw = (x1 - X0 - 16.0) / 3.0;
    for (k, (num, label)) in [
        (&s.files, "files"),
        (&s.folders, "folders"),
        (&s.took, "so far"),
    ]
    .into_iter()
    .enumerate()
    {
        let x = X0 + k as f32 * (cw + 8.0);
        unsafe {
            dc.FillRoundedRectangle(&rrect(x, 108.0, x + cw, 144.0, 12.0), &brush(dc, t.raised))
        };
        let nw = g.text_width(num, &f.title);
        text(dc, num, &f.title, x + 10.0, 108.0, nw + 2.0, 36.0, &ink);
        clipped_text(
            g,
            dc,
            label,
            &f.small,
            x + 14.0 + nw,
            108.0,
            cw - nw - 20.0,
            36.0,
            &muted,
        );
    }
    draw_icon(g, dc, Icon::Folder, X0 + 7.0, 166.0, 13.0, &muted);
    clipped_text(
        g,
        dc,
        &s.current,
        &f.small,
        X0 + 20.0,
        158.0,
        x1 - X0 - 20.0,
        16.0,
        &muted,
    );
    text(
        dc,
        "Biggest so far",
        &f.small,
        X0,
        190.0,
        160.0,
        16.0,
        &muted,
    );
    right_text(
        g,
        dc,
        "fills in while it scans",
        &f.small,
        x1,
        190.0,
        16.0,
        &muted,
    );
    let top_bytes = s.biggest.first().map_or(1, |b| b.1.max(1));
    for (k, (name, bytes, size)) in s.biggest.iter().enumerate() {
        let top = 212.0 + k as f32 * ROW;
        square_chip(g, dc, t, X0 + 2.0, top + 18.0, Icon::Folder);
        let sw = g.text_width(size, &f.body);
        clipped_text(
            g,
            dc,
            name,
            &f.title,
            X0 + 38.0,
            top,
            x1 - X0 - 38.0 - sw - 84.0,
            ROW,
            &ink,
        );
        bar(
            dc,
            t,
            x1 - sw - 76.0,
            top + 16.0,
            64.0,
            4.0,
            *bytes as f32 / top_bytes as f32,
        );
        right_text(g, dc, size, &f.body, x1, top, ROW, &ink);
    }
}

fn draw_folder_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    i: usize,
    top: f32,
    hover: Option<Hit>,
    kb: bool,
) {
    let row = &r.rows[i];
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let rect = (LIST.0, top, LIST.2, top + ROW);
    let hovered =
        matches!(hover, Some(Hit::Row(j) | Hit::RowOpen(j) | Hit::RowRecycle(j)) if j == i);
    let selected = r.sel == Some(i);
    if hovered {
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(rect.0, rect.1, rect.2, rect.3, 8.0),
                &brush(dc, t.raised),
            )
        };
    }
    if selected {
        let w = if kb { 2.0 } else { 1.5 };
        unsafe {
            dc.DrawRoundedRectangle(
                &rrect(rect.0 + 1.0, rect.1 + 1.0, rect.2 - 1.0, rect.3 - 1.0, 8.0),
                &brush(dc, t.text),
                w,
                None,
            )
        };
    }
    let cy = top + ROW / 2.0;
    square_chip(
        g,
        dc,
        t,
        LIST.0 + 6.0,
        cy,
        if row.dir { Icon::Folder } else { Icon::File },
    );
    let pct_r = LIST.2 - 8.0;
    let size_r = pct_r - 34.0 - 8.0;
    let bar_r = size_r - 64.0 - 12.0;
    right_text(
        g,
        dc,
        &format!("{}%", row.pct.round() as u64),
        &f.small,
        pct_r,
        top,
        ROW,
        &muted,
    );
    right_text(g, dc, &row.size, &f.body, size_r, top, ROW, &ink);
    let name_r = if hovered || selected {
        let [(_, open), (_, bin)] = row_buttons(top);
        icon_button(
            g,
            dc,
            t,
            open,
            Icon::External,
            hover == Some(Hit::RowOpen(i)),
        );
        if r.can_recycle {
            icon_button(
                g,
                dc,
                t,
                bin,
                Icon::Trash,
                hover == Some(Hit::RowRecycle(i)),
            );
        }
        open.0 - 8.0
    } else {
        bar(dc, t, bar_r - 64.0, cy - 2.0, 64.0, 4.0, row.bar);
        bar_r - 64.0 - 10.0
    };
    let nx = LIST.0 + 40.0;
    let nw = g.text_width(&row.name, &f.title);
    let room = name_r - nx;
    let hw = g.text_width("hidden", &f.body) + 6.0;
    if row.hidden && nw + hw < room {
        text(dc, &row.name, &f.title, nx, top, nw + 2.0, ROW, &ink);
        text(dc, "hidden", &f.body, nx + nw + 6.0, top, 60.0, ROW, &muted);
    } else {
        clipped_text(g, dc, &row.name, &f.title, nx, top, room, ROW, &ink);
    }
}

fn draw_treemap(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    hover: Option<Hit>,
) {
    let biggest = r.rows.first().map_or(0, |x| x.bytes);
    let ink = brush(dc, t.text);
    if r.tiles.is_empty() {
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(MAP.0, MAP.1, MAP.2, MAP.3, 6.0),
                &brush(dc, t.raised),
            )
        };
        center_text(
            g,
            dc,
            "Nothing in here takes room",
            &f.small,
            (MAP.0 + MAP.2) / 2.0,
            (MAP.1 + MAP.3) / 2.0 - 8.0,
            16.0,
            &brush(dc, t.muted),
        );
        return;
    }
    for tile in &r.tiles {
        let rc = (
            tile.rect.0 + 1.0,
            tile.rect.1 + 1.0,
            tile.rect.2 - 1.0,
            tile.rect.3 - 1.0,
        );
        let (w, h) = (rc.2 - rc.0, rc.3 - rc.1);
        if w < 1.0 || h < 1.0 {
            continue;
        }
        let bytes = tile.row.map_or_else(
            || {
                r.rows
                    .iter()
                    .skip(r.rows.len() - tile.more)
                    .map(|x| x.bytes)
                    .sum()
            },
            |i| r.rows[i].bytes,
        );
        let mut a = tile_alpha(bytes, biggest);
        if tile.row.is_some() && matches!(hover, Some(Hit::Tile(j)) if Some(j) == tile.row) {
            a += 0.03;
        }
        let rad = 6.0f32.min(w / 2.0).min(h / 2.0);
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(rc.0, rc.1, rc.2, rc.3, rad),
                &brush(dc, with_alpha(t.text, a)),
            )
        };
        for (ir, name, bytes) in &tile.inner {
            let ir = (ir.0 + 1.0, ir.1 + 1.0, ir.2 - 1.0, ir.3 - 1.0);
            if ir.2 - ir.0 < 2.0 || ir.3 - ir.1 < 2.0 {
                continue;
            }
            let inner_fill = if t.dark {
                rgba(0xFFFFFF, 0.05)
            } else {
                rgba(0xFFFFFF, 0.45)
            };
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(ir.0, ir.1, ir.2, ir.3, 4.0),
                    &brush(dc, inner_fill),
                );
                dc.DrawRoundedRectangle(
                    &rrect(ir.0 + 0.5, ir.1 + 0.5, ir.2 - 0.5, ir.3 - 0.5, 3.5),
                    &brush(dc, t.hair),
                    1.0,
                    None,
                );
            }
            if ir.2 - ir.0 >= 46.0 && ir.3 - ir.1 >= 22.0 {
                clipped_text(
                    g,
                    dc,
                    name,
                    &f.small_medium,
                    ir.0 + 6.0,
                    ir.1 + 3.0,
                    ir.2 - ir.0 - 10.0,
                    16.0,
                    &ink,
                );
            }
            if ir.2 - ir.0 >= 46.0 && ir.3 - ir.1 >= 40.0 {
                clipped_text(
                    g,
                    dc,
                    &crate::disk_model::size(*bytes),
                    &f.small,
                    ir.0 + 6.0,
                    ir.1 + 19.0,
                    ir.2 - ir.0 - 10.0,
                    16.0,
                    &ink,
                );
            }
        }
        let (name, size) = match tile.row {
            Some(i) => (r.rows[i].name.clone(), r.rows[i].size.clone()),
            None => (format!("{} more", tile.more), String::new()),
        };
        if w >= 30.0 && h >= 20.0 {
            clipped_text(
                g,
                dc,
                &name,
                &f.small_medium,
                rc.0 + 8.0,
                rc.1 + 6.0,
                w - 14.0,
                16.0,
                &ink,
            );
        }
        if w >= 44.0 && h >= 40.0 && !size.is_empty() {
            clipped_text(
                g,
                dc,
                &size,
                &f.small,
                rc.0 + 8.0,
                rc.1 + 22.0,
                w - 14.0,
                16.0,
                &ink,
            );
        }
        if tile.row.is_some() && tile.row == r.sel {
            unsafe {
                dc.DrawRoundedRectangle(
                    &rrect(
                        rc.0 + 1.0,
                        rc.1 + 1.0,
                        rc.2 - 1.0,
                        rc.3 - 1.0,
                        (rad - 1.0).max(0.0),
                    ),
                    &brush(dc, t.text),
                    2.0,
                    None,
                )
            };
        }
    }
}

fn draw_file_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    row: &FileRow,
    x0: f32,
    x1: f32,
    top: f32,
    hovered: bool,
    focused: bool,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    if row.checked || hovered {
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(x0, top, x1, top + ROW, 8.0),
                &brush(dc, if row.checked { t.chip } else { t.raised }),
            )
        };
    }
    if focused {
        focus_ring(dc, t, (x0, top, x1, top + ROW), 8.0);
    }
    let cy = top + ROW / 2.0;
    checkbox(g, dc, t, x0 + 10.0, cy, row.checked);
    square_chip(g, dc, t, x0 + 34.0, cy, row.icon);
    right_text(g, dc, &row.size, &f.body, x1 - 8.0, top, ROW, &ink);
    let month_r = x1 - 8.0 - 64.0;
    text(
        dc,
        &row.month,
        &f.small,
        month_r - 58.0,
        top,
        58.0,
        ROW,
        &muted,
    );
    let nx = x0 + 66.0;
    let room = month_r - 58.0 - 8.0 - nx;
    let nw = g.text_width(&row.name, &f.title);
    if row.sub.is_empty() || nw + 60.0 > room {
        clipped_text(g, dc, &row.name, &f.title, nx, top, room, ROW, &ink);
    } else {
        text(dc, &row.name, &f.title, nx, top, nw + 2.0, ROW, &ink);
        clipped_text(
            g,
            dc,
            &row.sub,
            &f.small,
            nx + nw + 8.0,
            top,
            room - nw - 8.0,
            ROW,
            &muted,
        );
    }
}

fn draw_sel_bar(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    x0: f32,
    x1: f32,
    top: f32,
    selected: &Option<(String, String)>,
    can_recycle: bool,
    hint: &str,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    hairline(dc, t, x0, x1, top);
    match selected {
        Some((n, size)) => {
            let nw = g.text_width(n, &f.title);
            text(dc, n, &f.title, x0, top + 8.0, nw + 2.0, 20.0, &ink);
            text(
                dc,
                size,
                &f.body,
                x0 + nw + 6.0,
                top + 8.0,
                120.0,
                20.0,
                &muted,
            );
        }
        None => text(
            dc,
            "Nothing selected",
            &f.body,
            x0,
            top + 8.0,
            200.0,
            20.0,
            &muted,
        ),
    }
    right_text(g, dc, hint, &f.small, x1, top + 10.0, 16.0, &muted);
    let [(_, open), (_, bin)] = sel_bar_rects(x0, x1, top);
    let any = selected.is_some();
    button(
        g,
        dc,
        t,
        f,
        open,
        "Open in Explorer",
        Some(Icon::External),
        false,
        hover == Some(Hit::SelOpen),
        any,
    );
    button(
        g,
        dc,
        t,
        f,
        bin,
        "Move to Recycle Bin",
        Some(Icon::Trash),
        false,
        hover == Some(Hit::SelRecycle),
        any && can_recycle,
    );
}

fn draw_results(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let x1 = W_RESULTS - X0;
    // Header.
    chip(
        g,
        dc,
        t,
        X0 + 16.0,
        32.0,
        16.0,
        if r.folder {
            Icon::Folder
        } else {
            Icon::HardDrive
        },
        false,
    );
    let hb = header_buttons(g, f);
    clipped_text(
        g,
        dc,
        &r.title,
        &f.panel_title,
        X0 + 40.0,
        14.0,
        hb[1].1 .0 - X0 - 52.0,
        20.0,
        &ink,
    );
    clipped_text(
        g,
        dc,
        &r.sub,
        &f.small,
        X0 + 40.0,
        34.0,
        hb[1].1 .0 - X0 - 52.0,
        16.0,
        &muted,
    );
    button(
        g,
        dc,
        t,
        f,
        hb[0].1,
        "Scan again",
        Some(Icon::Refresh),
        false,
        hover == Some(Hit::ScanAgain),
        true,
    );
    button(
        g,
        dc,
        t,
        f,
        hb[1].1,
        "Safe to clean",
        Some(Icon::Shield),
        false,
        hover == Some(Hit::OpenClean),
        true,
    );
    // Capacity.
    unsafe { dc.FillRoundedRectangle(&rrect(X0, 60.0, x1, 68.0, 4.0), &brush(dc, t.track)) };
    let uw = ((x1 - X0) * r.used.clamp(0.0, 1.0)).max(8.0);
    unsafe { dc.FillRoundedRectangle(&rrect(X0, 60.0, X0 + uw, 68.0, 4.0), &brush(dc, t.text)) };
    let w1 = g.text_width(&r.used_text, &f.small_medium);
    text(
        dc,
        &r.used_text,
        &f.small_medium,
        X0,
        74.0,
        w1 + 2.0,
        16.0,
        &ink,
    );
    text(
        dc,
        &format!(" used, {}", r.used_pct),
        &f.small,
        X0 + w1,
        74.0,
        160.0,
        16.0,
        &muted,
    );
    let tail = &r.free_text;
    let tw = g.text_width(tail, &f.small);
    right_text(g, dc, tail, &f.small, x1, 74.0, 16.0, &muted);
    let _ = tw;
    // Toolbar.
    let can_back = true;
    let back = (16.0, 106.0, 44.0, 134.0);
    let _ = can_back;
    icon_button(g, dc, t, back, Icon::ArrowLeft, hover == Some(Hit::Back));
    let tabs = tabs_rects(g, f);
    let n = r.crumbs.len();
    let pieces = crumb_layout(g, f, r, tabs[0].1 .0 - 12.0);
    let mut end = 56.0;
    for (k, (i, x, w)) in pieces.iter().enumerate() {
        let last = *i == Some(n - 1);
        let label = i.map_or("…", |i| r.crumbs[i].as_str());
        let fnt = if last { &f.title } else { &f.body };
        let c = if last || matches!(hover, Some(Hit::Crumb(j)) if Some(j) == *i) {
            &ink
        } else {
            &muted
        };
        text(dc, label, fnt, *x, 106.0, w + 2.0, 28.0, c);
        if k + 1 < pieces.len() {
            draw_icon(
                g,
                dc,
                Icon::ChevronRight,
                x + w + 10.0,
                120.0,
                12.0,
                &brush(dc, t.passed),
            );
        }
        end = x + w;
    }
    text(
        dc,
        &r.here_size,
        &f.body,
        end + 8.0,
        106.0,
        120.0,
        28.0,
        &muted,
    );
    let seg = (tabs[0].1 .0 - 4.0, 104.0, tabs[2].1 .2 + 4.0, 136.0);
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(seg.0, seg.1, seg.2, seg.3, 16.0),
            &brush(dc, t.raised),
        )
    };
    for (tab, rc) in tabs {
        let on = tab == r.tab;
        if on {
            unsafe {
                dc.FillRoundedRectangle(&rrect(rc.0, rc.1, rc.2, rc.3, 14.0), &brush(dc, t.panel));
                dc.DrawRoundedRectangle(
                    &rrect(rc.0 + 0.5, rc.1 + 0.5, rc.2 - 0.5, rc.3 - 0.5, 13.5),
                    &brush(dc, t.hair),
                    1.0,
                    None,
                );
            }
        }
        let label = match tab {
            Tab::Folders => "Folders",
            Tab::Files => "Files",
            Tab::Types => "Types",
        };
        let c = if on || hover == Some(Hit::Tab(tab)) {
            &ink
        } else {
            &muted
        };
        center_text(
            g,
            dc,
            label,
            if on { &f.title } else { &f.body },
            (rc.0 + rc.2) / 2.0,
            rc.1,
            28.0,
            c,
        );
    }
    match r.tab {
        Tab::Folders => {
            text(
                dc,
                "Area is size. Greys only tell tiles apart.",
                &f.small,
                X0,
                144.0,
                340.0,
                16.0,
                &muted,
            );
            right_text(
                g,
                dc,
                "Largest first · Enter opens a folder",
                &f.small,
                x1,
                144.0,
                16.0,
                &muted,
            );
            draw_treemap(g, dc, t, f, r, hover);
            let shown = r.rows.len().min(r.first + LIST_ROWS);
            for (k, i) in (r.first..shown).enumerate() {
                draw_folder_row(g, dc, t, f, r, i, LIST.1 + k as f32 * ROW, hover, v.kb);
            }
            if r.rows.is_empty() {
                text(
                    dc,
                    "This folder is empty",
                    &f.body,
                    LIST.0 + 8.0,
                    LIST.1,
                    260.0,
                    ROW,
                    &muted,
                );
            }
            let rest = &r.rows[shown..];
            if !rest.is_empty() {
                let top = LIST.1 + (shown - r.first) as f32 * ROW;
                let bytes: u64 = rest.iter().map(|x| x.bytes).sum();
                let pct: f64 = rest.iter().map(|x| x.pct).sum();
                square_chip(g, dc, t, LIST.0 + 6.0, top + ROW / 2.0, Icon::More);
                let pct_r = LIST.2 - 8.0;
                right_text(
                    g,
                    dc,
                    &format!("{}%", pct.round() as u64),
                    &f.small,
                    pct_r,
                    top,
                    ROW,
                    &muted,
                );
                right_text(
                    g,
                    dc,
                    &crate::disk_model::size(bytes),
                    &f.body,
                    pct_r - 42.0,
                    top,
                    ROW,
                    &ink,
                );
                let top_b = r.rows.first().map_or(1, |x| x.bytes.max(1));
                bar(
                    dc,
                    t,
                    pct_r - 42.0 - 76.0 - 64.0,
                    top + ROW / 2.0 - 2.0,
                    64.0,
                    4.0,
                    bytes as f32 / top_b as f32,
                );
                let more = if rest.len() == 1 {
                    "1 more item".to_string()
                } else {
                    format!("{} more items", rest.len())
                };
                text(dc, &more, &f.title, LIST.0 + 40.0, top, 200.0, ROW, &ink);
            }
            // File types.
            text(
                dc,
                &format!("File types in {}", r.here),
                &f.small,
                X0,
                432.0,
                400.0,
                16.0,
                &muted,
            );
            right_text(g, dc, "by size", &f.small, x1, 432.0, 16.0, &muted);
            draw_type_bar(dc, t, r, X0, 454.0, x1);
            let cw = (x1 - X0) / 6.0;
            for (i, ty) in r.types.iter().enumerate() {
                let x = X0 + i as f32 * cw;
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(x, 478.0, x + 8.0, 486.0, 2.0),
                        &brush(dc, type_grey(t, ty.grey)),
                    );
                    if ty.grey == 5 {
                        dc.DrawRoundedRectangle(
                            &rrect(x + 0.5, 478.5, x + 7.5, 485.5, 1.5),
                            &brush(dc, t.hair),
                            1.0,
                            None,
                        );
                    }
                }
                clipped_text(
                    g,
                    dc,
                    ty.label,
                    &f.small_medium,
                    x + 14.0,
                    474.0,
                    cw - 18.0,
                    16.0,
                    &ink,
                );
                text(
                    dc,
                    &ty.size,
                    &f.small,
                    x + 14.0,
                    492.0,
                    cw - 18.0,
                    16.0,
                    &muted,
                );
            }
        }
        Tab::Files => {
            text(dc, &r.files_count, &f.small, X0, 144.0, 400.0, 16.0, &muted);
            right_text(
                g,
                dc,
                "Biggest first, anywhere in here",
                &f.small,
                x1,
                144.0,
                16.0,
                &muted,
            );
            if r.files.is_empty() {
                text(
                    dc,
                    "No files in here",
                    &f.body,
                    X0 + 8.0,
                    166.0,
                    260.0,
                    ROW,
                    &muted,
                );
            }
            let shown = r.files.len().min(r.files_first + FILES_ROWS);
            for (k, i) in (r.files_first..shown).enumerate() {
                let top = 166.0 + k as f32 * FILE_ROW;
                draw_file_row(
                    g,
                    dc,
                    t,
                    f,
                    &r.files[i],
                    X0,
                    x1,
                    top,
                    hover == Some(Hit::File(i)),
                    v.kb && v.cursor == Some(i),
                );
            }
            draw_scroll_thumb(
                dc,
                t,
                x1 + 6.0,
                166.0,
                FILES_ROWS as f32 * FILE_ROW,
                r.files.len(),
                FILES_ROWS,
                r.files_first,
            );
            draw_sel_bar(
                g,
                dc,
                t,
                f,
                X0,
                x1,
                RESULTS_H - 8.0 - SEL_H,
                &r.selected,
                r.can_recycle,
                "Space selects · Delete moves",
                hover,
            );
        }
        Tab::Types => {
            text(
                dc,
                &format!("File types in {}", r.here),
                &f.small,
                X0,
                144.0,
                400.0,
                16.0,
                &muted,
            );
            right_text(g, dc, "by size", &f.small, x1, 144.0, 16.0, &muted);
            draw_type_bar(dc, t, r, X0, 168.0, x1);
            let top_b = r.types.iter().map(|x| x.bytes).max().unwrap_or(1).max(1);
            for (i, ty) in r.types.iter().enumerate() {
                let top = 192.0 + i as f32 * FILE_ROW;
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(X0 + 8.0, top + 13.0, X0 + 18.0, top + 23.0, 3.0),
                        &brush(dc, type_grey(t, ty.grey)),
                    );
                    if ty.grey == 5 {
                        dc.DrawRoundedRectangle(
                            &rrect(X0 + 8.5, top + 13.5, X0 + 17.5, top + 22.5, 2.5),
                            &brush(dc, t.hair),
                            1.0,
                            None,
                        );
                    }
                }
                text(dc, ty.label, &f.title, X0 + 30.0, top, 200.0, ROW, &ink);
                text(dc, &ty.files, &f.body, X0 + 200.0, top, 160.0, ROW, &muted);
                bar(
                    dc,
                    t,
                    X0 + 380.0,
                    top + 16.0,
                    200.0,
                    4.0,
                    ty.bytes as f32 / top_b as f32,
                );
                right_text(g, dc, &ty.size, &f.body, x1 - 8.0, top, ROW, &ink);
            }
        }
    }
}

fn draw_type_bar(dc: &ID2D1DeviceContext, t: &Theme, r: &ResultsView, x0: f32, y: f32, x1: f32) {
    let total: u64 = r.types.iter().map(|x| x.bytes).sum();
    unsafe { dc.FillRoundedRectangle(&rrect(x0, y, x1, y + 10.0, 3.0), &brush(dc, t.track)) };
    if total == 0 {
        return;
    }
    let mut x = x0;
    for ty in &r.types {
        let w = (x1 - x0) * ty.bytes as f32 / total as f32;
        if w >= 1.0 {
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(x, y, (x + w - 2.0).max(x + 1.0), y + 10.0, 2.0),
                    &brush(dc, type_grey(t, ty.grey)),
                );
                if ty.grey == 5 {
                    dc.DrawRoundedRectangle(
                        &rrect(x + 0.5, y + 0.5, (x + w - 2.5).max(x + 1.0), y + 9.5, 1.5),
                        &brush(dc, t.hair),
                        1.0,
                        None,
                    );
                }
            }
        }
        x += w;
    }
}

fn draw_scroll_thumb(
    dc: &ID2D1DeviceContext,
    t: &Theme,
    x: f32,
    top: f32,
    track: f32,
    n: usize,
    shown: usize,
    first: usize,
) {
    if n <= shown {
        return;
    }
    let th = (track * shown as f32 / n as f32).max(20.0);
    let ty = top + (track - th) * first as f32 / (n - shown) as f32;
    unsafe { dc.FillRoundedRectangle(&rrect(x - 3.0, ty, x, ty + th, 1.5), &brush(dc, t.track)) };
}

fn draw_clean(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    c: &CleanView,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let x1 = W_SMALL - X0;
    text(
        dc,
        "Safe to clean",
        &f.panel_title,
        X0,
        16.0,
        200.0,
        22.0,
        &ink,
    );
    icon_button(
        g,
        dc,
        t,
        (x1 - 28.0, 14.0, x1, 42.0),
        Icon::Close,
        hover == Some(Hit::Back),
    );
    right_text(g, dc, &c.place, &f.small, x1 - 36.0, 19.0, 16.0, &muted);
    let bw = g.text_width(&c.total, &f.big);
    text(dc, &c.total, &f.big, X0, 46.0, bw + 4.0, 34.0, &ink);
    clipped_text(
        g,
        dc,
        "to review, nothing goes until you confirm",
        &f.small,
        X0 + bw + 8.0,
        57.0,
        x1 - X0 - bw - 8.0,
        16.0,
        &muted,
    );
    let rw = btn_w(g, f, "Review", false);
    if let Some(nothing) = &c.nothing {
        let top = clean_rows_top();
        unsafe {
            dc.FillRoundedRectangle(&rrect(X0, top, x1, top + 76.0, 12.0), &brush(dc, t.raised))
        };
        chip(g, dc, t, X0 + 26.0, top + 26.0, 13.0, Icon::Tick, false);
        text(
            dc,
            nothing,
            &f.title,
            X0 + 48.0,
            top + 12.0,
            280.0,
            18.0,
            &ink,
        );
        text(
            dc,
            "Temp files and caches are already small. Big folders are still in Results.",
            &f.small_wrap,
            X0 + 48.0,
            top + 32.0,
            x1 - X0 - 60.0,
            40.0,
            &muted,
        );
    } else {
        for (i, s) in c.rows.iter().enumerate() {
            let top = clean_rows_top() + i as f32 * SUGGESTION_H;
            if i > 0 {
                hairline(dc, t, X0, x1, top);
            }
            if v.kb && v.cursor == Some(i) {
                focus_ring(dc, t, (X0, top + 4.0, x1, top + SUGGESTION_H - 4.0), 10.0);
            }
            chip(g, dc, t, X0 + 16.0, top + 26.0, 16.0, s.icon, false);
            right_text(g, dc, &s.size, &f.title, x1, top + 10.0, 20.0, &ink);
            clipped_text(
                g,
                dc,
                &s.title,
                &f.title,
                X0 + 44.0,
                top + 10.0,
                x1 - X0 - 44.0 - rw - 12.0,
                20.0,
                &ink,
            );
            text(
                dc,
                &s.body,
                &f.small_wrap,
                X0 + 44.0,
                top + 32.0,
                x1 - X0 - 44.0 - rw - 12.0,
                40.0,
                &muted,
            );
            let tag = if s.check_first {
                "Check first"
            } else {
                "Safe to remove"
            };
            let tw = g.text_width(tag, &f.small_medium) + 16.0;
            let ty = top + 74.0;
            unsafe {
                if s.check_first {
                    dc.DrawRoundedRectangle(
                        &rrect(X0 + 44.5, ty + 0.5, X0 + 44.0 + tw - 0.5, ty + 19.5, 9.5),
                        &brush(dc, with_alpha(t.text, 0.5)),
                        1.0,
                        None,
                    );
                } else {
                    dc.FillRoundedRectangle(
                        &rrect(X0 + 44.0, ty, X0 + 44.0 + tw, ty + 20.0, 10.0),
                        &brush(dc, t.chip),
                    );
                }
            }
            center_text(
                g,
                dc,
                tag,
                &f.small_medium,
                X0 + 44.0 + tw / 2.0,
                ty,
                20.0,
                &ink,
            );
            button(
                g,
                dc,
                t,
                f,
                (x1 - rw, top + 36.0, x1, top + 64.0),
                "Review",
                None,
                false,
                hover == Some(Hit::Review(i)),
                s.can_review,
            );
        }
    }
    let top = extras_top(c);
    hairline(dc, t, X0, x1, top - 14.0);
    text(
        dc,
        "Not counted above",
        &f.small,
        X0,
        top - 8.0,
        200.0,
        16.0,
        &muted,
    );
    let top = top + 12.0;
    chip(g, dc, t, X0 + 16.0, top + 22.0, 16.0, Icon::Trash, false);
    text(
        dc,
        "Recycle Bin",
        &f.title,
        X0 + 44.0,
        top + 4.0,
        200.0,
        20.0,
        &ink,
    );
    right_text(g, dc, &c.bin, &f.title, x1, top + 4.0, 20.0, &ink);
    text(
        dc,
        "Already deleted. Emptying it is permanent, so that stays your call.",
        &f.small_wrap,
        X0 + 44.0,
        top + 26.0,
        x1 - X0 - 44.0 - 80.0,
        40.0,
        &muted,
    );
    let ow = btn_w(g, f, "Open", false);
    button(
        g,
        dc,
        t,
        f,
        (x1 - ow, top + 22.0, x1, top + 50.0),
        "Open",
        None,
        false,
        hover == Some(Hit::BinOpen),
        true,
    );
    let top = top + EXTRA_H;
    hairline(dc, t, X0, x1, top - 6.0);
    chip(g, dc, t, X0 + 16.0, top + 22.0, 16.0, Icon::Sliders, false);
    text(
        dc,
        "System files",
        &f.title,
        X0 + 44.0,
        top + 4.0,
        200.0,
        20.0,
        &ink,
    );
    text(
        dc,
        "Update leftovers and restore points. Windows manages these itself.",
        &f.small_wrap,
        X0 + 44.0,
        top + 26.0,
        x1 - X0 - 44.0 - 90.0,
        40.0,
        &muted,
    );
    let sw = btn_w(g, f, "Settings", false);
    button(
        g,
        dc,
        t,
        f,
        (x1 - sw, top, x1, top + 28.0),
        "Settings",
        None,
        false,
        hover == Some(Hit::Settings),
        true,
    );
    let top = top + EXTRA_H;
    hairline(dc, t, X0, x1, top - 6.0);
    draw_icon(g, dc, Icon::Shield, X0 + 8.0, top + 14.0, 13.0, &muted);
    text(
        dc,
        "Everything goes to the Recycle Bin. [App] never cleans by itself.",
        &f.small_wrap,
        X0 + 22.0,
        top + 6.0,
        x1 - X0 - 22.0,
        40.0,
        &muted,
    );
}

fn draw_review(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ReviewView,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let x1 = W_REVIEW - X0;
    icon_button(
        g,
        dc,
        t,
        (X0, 16.0, X0 + 28.0, 44.0),
        Icon::ArrowLeft,
        hover == Some(Hit::Back),
    );
    clipped_text(
        g,
        dc,
        &r.title,
        &f.panel_title,
        X0 + 38.0,
        16.0,
        x1 - X0 - 76.0,
        28.0,
        &ink,
    );
    icon_button(
        g,
        dc,
        t,
        (x1 - 28.0, 16.0, x1, 44.0),
        Icon::External,
        hover == Some(Hit::ReviewFolder),
    );
    text(dc, &r.sub, &f.small, X0, 52.0, 240.0, 16.0, &muted);
    let label = if r.all { "Select none" } else { "Select all" };
    right_text(
        g,
        dc,
        label,
        &f.small,
        x1,
        52.0,
        16.0,
        if hover == Some(Hit::SelectAll) {
            &ink
        } else {
            &muted
        },
    );
    if r.rows.is_empty() {
        text(
            dc,
            "Nothing left to review here",
            &f.body,
            X0 + 8.0,
            review_list_top(),
            260.0,
            ROW,
            &muted,
        );
    }
    let shown = r.rows.len().min(r.first + REVIEW_ROWS);
    for (k, i) in (r.first..shown).enumerate() {
        let top = review_list_top() + k as f32 * FILE_ROW;
        draw_file_row(
            g,
            dc,
            t,
            f,
            &r.rows[i],
            X0,
            x1,
            top,
            hover == Some(Hit::File(i)),
            v.kb && v.cursor == Some(i),
        );
    }
    draw_scroll_thumb(
        dc,
        t,
        x1 + 8.0,
        review_list_top(),
        REVIEW_ROWS as f32 * FILE_ROW,
        r.rows.len(),
        REVIEW_ROWS,
        r.first,
    );
    draw_sel_bar(
        g,
        dc,
        t,
        f,
        X0,
        x1,
        review_h(r) - SEL_H,
        &r.selected,
        r.can_recycle,
        "Space selects · Backspace goes up",
        hover,
    );
}

fn draw_cards(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let w = width(v);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    for (i, ((top, h), c)) in card_tops(g, f, v).into_iter().zip(&v.cards).enumerate() {
        let x1 = w - X0;
        unsafe {
            dc.FillRoundedRectangle(&rrect(X0, top, x1, top + h, 12.0), &brush(dc, t.panel));
            dc.DrawRoundedRectangle(
                &rrect(X0 + 0.5, top + 0.5, x1 - 0.5, top + h - 0.5, 11.5),
                &brush(dc, t.hair),
                1.0,
                None,
            );
        }
        let (cx, cy) = (X0 + 24.0, top + 24.0);
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0, 7.0),
                &brush(
                    dc,
                    if c.ember {
                        with_alpha(t.ember, 0.14)
                    } else {
                        t.chip
                    },
                ),
            )
        };
        draw_icon(
            g,
            dc,
            c.icon,
            cx,
            cy,
            13.0,
            &brush(dc, if c.ember { t.ember } else { t.text }),
        );
        clipped_text(
            g,
            dc,
            &c.title,
            &f.title,
            X0 + 48.0,
            top + 12.0,
            x1 - X0 - 60.0,
            18.0,
            &ink,
        );
        text(
            dc,
            &c.body,
            &f.small_wrap,
            X0 + 48.0,
            top + 34.0,
            x1 - X0 - 64.0,
            h - 34.0,
            &muted,
        );
        if let Some(label) = &c.button {
            let bw = btn_w(g, f, label, false);
            let x = X0 + 48.0;
            button(
                g,
                dc,
                t,
                f,
                (x, top + h - 40.0, x + bw, top + h - 12.0),
                label,
                None,
                false,
                hover == Some(Hit::Card(i)),
                true,
            );
        }
    }
}

fn draw_confirm(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    c: &ConfirmView,
    hover: Option<Hit>,
) {
    let (w, h) = size(g, f, v);
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(0.0, 0.0, w, h, 20.0),
            &brush(dc, with_alpha(t.panel, 0.82)),
        )
    };
    let s = sheet_rect(g, f, v, c);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    unsafe {
        dc.FillRoundedRectangle(&rrect(s.0, s.1, s.2, s.3, 16.0), &brush(dc, t.surface));
        dc.DrawRoundedRectangle(
            &rrect(s.0 + 0.5, s.1 + 0.5, s.2 - 0.5, s.3 - 0.5, 15.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    let (x0, x1) = (s.0 + 18.0, s.2 - 18.0);
    let mut y = s.1 + 16.0;
    chip(g, dc, t, x0 + 16.0, y + 16.0, 16.0, Icon::Trash, false);
    clipped_text(
        g,
        dc,
        &c.title,
        &f.panel_title,
        x0 + 44.0,
        y,
        x1 - x0 - 44.0,
        32.0,
        &ink,
    );
    y += 32.0 + 12.0;
    let wh = well_h(c);
    unsafe { dc.FillRoundedRectangle(&rrect(x0, y, x1, y + wh, 12.0), &brush(dc, t.raised)) };
    let mut ry = y + 4.0;
    for (icon, name, size) in &c.items {
        square_chip(g, dc, t, x0 + 8.0, ry + ROW / 2.0, *icon);
        let sw = g.text_width(size, &f.body);
        clipped_text(
            g,
            dc,
            name,
            &f.title,
            x0 + 42.0,
            ry,
            x1 - x0 - 58.0 - sw,
            ROW,
            &ink,
        );
        right_text(g, dc, size, &f.body, x1 - 10.0, ry, ROW, &ink);
        ry += ROW;
    }
    if let Some(more) = &c.more {
        text(dc, more, &f.body, x0 + 42.0, ry, 200.0, 28.0, &muted);
    }
    y += wh + 12.0;
    let bw = x1 - x0;
    let bh = wrap_h(g, c.body, &f.body_wrap, bw);
    text(dc, c.body, &f.body_wrap, x0, y, bw, bh + 2.0, &ink);
    y += bh + 8.0;
    let hh = wrap_h(g, &c.honest, &f.small_wrap, bw);
    text(dc, &c.honest, &f.small_wrap, x0, y, bw, hh + 2.0, &muted);
    let [(_, cancel), (_, mv)] = sheet_buttons(g, f, s);
    let cancel_hover = hover == Some(Hit::Cancel);
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(cancel.0, cancel.1, cancel.2, cancel.3, 18.0),
            &brush(
                dc,
                if cancel_hover {
                    t.button_hover
                } else {
                    t.button
                },
            ),
        )
    };
    center_text(
        g,
        dc,
        "Cancel",
        &f.title,
        (cancel.0 + cancel.2) / 2.0,
        cancel.1,
        36.0,
        &ink,
    );
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(mv.0, mv.1, mv.2, mv.3, 18.0),
            &brush(
                dc,
                if hover == Some(Hit::Move) {
                    with_alpha(t.text, 0.85)
                } else {
                    t.text
                },
            ),
        )
    };
    center_text(
        g,
        dc,
        "Move to Recycle Bin",
        &f.title,
        (mv.0 + mv.2) / 2.0,
        mv.1,
        36.0,
        &brush(dc, t.panel),
    );
    focus_ring(dc, t, if c.on_move { mv } else { cancel }, 18.0);
}

pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let (w, h) = size(g, f, v);
    panel_bg(dc, t, w, h);
    let under = if v.confirm.is_some() { None } else { hover };
    match &v.screen {
        Screen::Start(d) => draw_start(g, dc, t, f, d, v, under),
        Screen::Scanning(s) => draw_scanning(g, dc, t, f, s, under),
        Screen::Results(r) => draw_results(g, dc, t, f, r, v, under),
        Screen::Clean(c) => draw_clean(g, dc, t, f, c, v, under),
        Screen::Review(r) => draw_review(g, dc, t, f, r, v, under),
    }
    draw_cards(g, dc, t, f, v, under);
    if let Some(c) = &v.confirm {
        draw_confirm(g, dc, t, f, v, c, hover);
    }
}

/// Which list a wheel over (x, y) scrolls.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scroll {
    Folders,
    Files,
    Review,
}

pub fn scroll_target(v: &View, x: f32, y: f32) -> Option<Scroll> {
    match &v.screen {
        Screen::Results(r) => match r.tab {
            Tab::Folders if inside(&LIST, x, y) || inside(&MAP, x, y) => Some(Scroll::Folders),
            Tab::Files if (160.0..RESULTS_H - SEL_H).contains(&y) => Some(Scroll::Files),
            _ => None,
        },
        Screen::Review(_) => Some(Scroll::Review),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_and_treemap_sit_side_by_side_inside_the_results_panel() {
        assert!(MAP.2 < LIST.0, "the map stays left of the list");
        assert_eq!(
            LIST.3 - LIST.1,
            (LIST_ROWS + 1) as f32 * ROW,
            "six rows and the tail fill the map's height"
        );
        assert!(LIST.2 <= W_RESULTS - X0);
        assert!(
            166.0 + FILES_ROWS as f32 * FILE_ROW <= RESULTS_H - 8.0 - SEL_H,
            "file rows end above the selection bar"
        );
    }

    #[test]
    fn row_buttons_stay_left_of_the_size_column() {
        let [(_, open), (_, bin)] = row_buttons(LIST.1);
        assert!(open.2 < bin.0);
        assert!(
            bin.2 < LIST.2 - 8.0 - 34.0 - 8.0 - 40.0,
            "the size stays readable next to the buttons"
        );
        assert!(open.0 > LIST.0 + 140.0, "the name keeps its room");
    }

    #[test]
    fn the_review_panel_grows_with_its_rows_up_to_a_limit() {
        let mk = |n: usize| ReviewView {
            title: String::new(),
            sub: String::new(),
            rows: (0..n)
                .map(|_| FileRow {
                    name: "a".into(),
                    sub: String::new(),
                    month: String::new(),
                    size: String::new(),
                    icon: Icon::File,
                    checked: false,
                })
                .collect(),
            first: 0,
            selected: None,
            can_recycle: true,
            all: false,
        };
        assert!(review_h(&mk(2)) < review_h(&mk(5)));
        assert_eq!(review_h(&mk(9)), review_h(&mk(40)));
    }
}
