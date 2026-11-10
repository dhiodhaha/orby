#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Disk window, Disk v2 (Doop page "v6.3 · Foundation + Disk v2", frames 02 to 06): the drive
//! list, Results (headline, capacity bar by top-level folder, treemap, folder list, detail strip)
//! that also shows a scan live under a veil, Files and Types, Safe to clean with its groups,
//! filter chips and selection bar, a suggestion's Review list, Duplicates (groups of identical
//! files with a keeper each), the state cards and the grouped confirm sheet, under a title band
//! with the close button. Pure drawing and hit testing over a
//! `View` that disk.rs builds; the moving parts arrive already eased in `View::motion`.

use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::disk_model::Rect;
use crate::gfx::*;
use crate::motion;
use crate::tasks_panel::clipped_text;
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

/// The window's logical width.
pub const W: f32 = 756.0;
/// The window's logical height when the screen has room for it.
pub const H: f32 = 704.0;
/// The title band on every screen: "Disk" and the close button. Its empty part drags the window.
pub const TOP: f32 = 44.0;
const X0: f32 = 20.0;
const X1: f32 = W - X0;
const ROW: f32 = 36.0;
const FILE_ROW: f32 = 40.0;
/// Results, top to bottom (y under the band): header row, headline, capacity bar, legend,
/// toolbar, then the treemap and the list.
const HEAD_Y: f32 = 64.0;
const CBAR_Y: f32 = 104.0;
const LEG_Y: f32 = 124.0;
const TOOL_Y: f32 = 158.0;
pub const BODY_Y: f32 = 202.0;
/// The treemap's right edge and the list beside it.
const MAP_R: f32 = 320.0;
const LIST_L: f32 = 336.0;
const LIST_R: f32 = X1;
/// The list's column header ("Name", "%", "Size") and the gap under it.
const LIST_HEAD: f32 = 20.0;
/// The detail strip under the map, and the selection bar where one shows.
const STRIP_H: f32 = 44.0;
const BAR_H: f32 = 56.0;
/// A list never shrinks below this many rows, however many cards sit under it.
const MIN_ROWS: usize = 4;
/// Columns from the right edge of a folder row: size 64, percent 44, bar 56, gaps 10.
const SIZE_W: f32 = 64.0;
const PCT_W: f32 = 44.0;
const MINI_W: f32 = 56.0;
/// Safe to clean: the list starts under the filter chips.
const CLEAN_TOP: f32 = 84.0;
const CLEAN_ROW: f32 = 42.0;
const REVIEW_TOP: f32 = 80.0;
/// Duplicates: the list starts under the line with "Select all extra copies".
const DUPES_TOP: f32 = 92.0;
const DUPE_HEAD: f32 = 32.0;
/// The keep column and the date column, from the right of a copy's row.
const KEEP_W: f32 = 76.0;
const DATE_W: f32 = 84.0;
/// The Start screen's "Also find duplicate files" row.
const OPTION_H: f32 = 44.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Folders,
    Files,
    Types,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Safe,
    Check,
}

pub struct DriveRow {
    pub name: String,
    pub free: String,
    pub used: f32,
}

/// One part of the capacity bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub label: String,
    pub bytes: u64,
    pub kind: SegKind,
    /// The folder row it stands for, when the list shows the drive's top level.
    pub row: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SegKind {
    /// One of the six greys, biggest first.
    Grey(usize),
    /// Used space the scan has not counted yet.
    Dotted,
    /// Free space: a light wash with a hairline.
    Free,
}

pub struct ListRow {
    pub name: String,
    pub bytes: u64,
    pub size: String,
    pub pct: String,
    pub pct_v: f64,
    /// Against the biggest sibling, 0..1.
    pub bar: f32,
    pub dir: bool,
    /// "hidden" or "system", muted after the name.
    pub tag: Option<&'static str>,
    pub checked: bool,
    /// The scan is inside this folder right now.
    pub counting: bool,
}

/// A treemap tile. `row` None is the "N smaller" tile (`more` rows), or, while scanning, the
/// dotted tile for space not counted yet (`skeleton`).
#[derive(Clone, Debug, PartialEq)]
pub struct MapTile {
    pub row: Option<usize>,
    pub rect: Rect,
    pub inner: Vec<(Rect, String, u64)>,
    pub more: usize,
    pub more_bytes: u64,
    pub skeleton: bool,
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

/// The strip under the map: what the pointer or the selection is on.
pub struct Detail {
    pub name: String,
    pub path: String,
    pub pct: String,
    pub size: String,
}

/// A scan in progress, shown inside Results under a veil.
pub struct LiveView {
    /// "Scanning Users".
    pub label: String,
    pub path: String,
    /// Bytes counted over bytes in use (None for a folder: the arc only travels).
    pub frac: Option<f32>,
    /// "512,340 files so far · 0:31".
    pub foot: String,
}

pub struct ResultsView {
    pub folder: bool,
    /// "(C:) Windows".
    pub chip: String,
    /// "· scanned 09:12" or "· scanning".
    pub chip_sub: String,
    pub big: String,
    pub of: String,
    pub right: String,
    pub segs: Vec<Seg>,
    pub crumbs: Vec<String>,
    pub tab: Tab,
    pub rows: Vec<ListRow>,
    pub first: usize,
    pub sel: Option<usize>,
    pub tiles: Vec<MapTile>,
    pub types: Vec<TypeRow>,
    pub files: Vec<FileRow>,
    pub files_first: usize,
    pub files_count: String,
    pub selected: Option<(String, String)>,
    pub can_recycle: bool,
    pub detail: Option<Detail>,
    /// A scan is running: the list and the map fill in live, skeleton rows hold the rest, and
    /// only Stop answers.
    pub scanning: bool,
    /// The veil's pill: while scanning, and while the veil fades over the finished results.
    pub live: Option<LiveView>,
}

pub struct CleanRow {
    /// The suggestion's index in disk.rs's list.
    pub index: usize,
    pub icon: Icon,
    pub title: String,
    pub body: String,
    pub check_first: bool,
    pub size: String,
    pub checked: bool,
    pub can_review: bool,
}

pub struct CleanGroup {
    pub title: String,
    /// "3 · 23 GB".
    pub sub: String,
    pub rows: Vec<CleanRow>,
}

pub struct CleanView {
    pub place: String,
    pub total: String,
    pub filter: Filter,
    /// All, Safe, Check first.
    pub counts: [usize; 3],
    pub groups: Vec<CleanGroup>,
    pub first: usize,
    pub bin: String,
    pub nothing: Option<String>,
    pub selected: Option<(String, String)>,
}

pub struct ReviewView {
    pub title: String,
    pub sub: String,
    /// Every item in the list.
    pub len: usize,
    /// The rows from `rows_from` on, as many as can show (`REVIEW_WINDOW`): a review of temp
    /// files can list thousands, and only these are ever drawn.
    pub rows_from: usize,
    pub rows: Vec<FileRow>,
    pub first: usize,
    pub selected: Option<(String, String)>,
    pub can_recycle: bool,
    pub all: bool,
}

pub struct ConfirmGroup {
    pub check_first: bool,
    /// "1 · 6.1 GB".
    pub count: String,
    /// Name, a muted line under it (a path, or "38 items"), size.
    pub items: Vec<(String, String, String)>,
    pub more: Option<String>,
}

pub struct ConfirmView {
    pub title: String,
    pub groups: Vec<ConfirmGroup>,
    pub note: &'static str,
    /// "Moving 11.3 GB to the Recycle Bin".
    pub footer: String,
    /// "Move 5 to Recycle Bin".
    pub action: String,
    /// Keyboard focus is on Move (it starts on Cancel).
    pub on_move: bool,
}

/// One copy of a duplicated file.
pub struct DupeRowView {
    /// The full path (cut in the middle when drawn).
    pub path: String,
    /// "6 Nov 2025".
    pub date: String,
    pub keep: bool,
    pub checked: bool,
}

/// One group of copies. A check can find thousands of groups, so disk.rs fills `name`, `sub`
/// and `rows` only for the groups that reach into the list's window (`DUPES_WINDOW` entries
/// from `first`); every other group carries only its row count.
#[derive(Default)]
pub struct DupeGroupView {
    /// The copies still there (the group's rows, whether filled or not).
    pub len: usize,
    /// The file's name, from the first copy.
    pub name: String,
    /// "3 copies · 1.2 GB to reclaim".
    pub sub: String,
    pub rows: Vec<DupeRowView>,
}

pub enum DupesBody {
    /// Before a check (or after Stop): stage 1's estimate and Check now (`check`).
    Offer {
        title: String,
        body: String,
        cost: &'static str,
        check: bool,
    },
    /// "Comparing whole files…", "1,204 of 3,400 files · 1.2 GB of 4.5 GB", and the bar.
    Running {
        stage: &'static str,
        line: String,
        frac: f32,
    },
    Groups(Vec<DupeGroupView>),
    /// A check that found nothing (or nothing is left).
    Empty {
        title: String,
        body: String,
    },
}

pub struct DupesView {
    /// "C:, from the 09:12 scan".
    pub place: String,
    /// "1.2 GB" with " to reclaim", or "up to 4.1 GB" with " possible".
    pub total: String,
    pub total_label: &'static str,
    /// "12 groups · 3 skipped".
    pub sub: String,
    pub body: DupesBody,
    pub first: usize,
    pub selected: Option<(String, String)>,
    /// Every extra copy is picked: the chip reads "Select none".
    pub all: bool,
    pub can_recycle: bool,
}

pub struct Card {
    pub icon: Icon,
    pub ember: bool,
    pub title: String,
    pub body: String,
    pub button: Option<String>,
}

// Built once per frame and never stored, so the big Results variant costs nothing to keep inline.
#[allow(clippy::large_enum_variant)]
pub enum Screen {
    /// The drives, and whether "Also find duplicate files" is ticked.
    Start(Vec<DriveRow>, bool),
    Results(ResultsView),
    Clean(CleanView),
    Review(ReviewView),
    Dupes(DupesView),
}

/// What is lit while the pointer rests on something: the others fade to 0.3.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Focus {
    /// A folder row (its tile, its row, its bar segment).
    Row(usize),
    /// A capacity segment with no row of its own (System files, Other, Free).
    Seg(usize),
}

/// The moving parts, eased by disk.rs on its frame clock. Every value is at rest at its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    /// The veil over a scan in progress, 0 (gone) to 1.
    pub veil: f32,
    /// How far the unlit tiles and segments have faded, 0..1 (0.3 opacity at 1).
    pub dim: f32,
    pub focus: Option<Focus>,
    /// Tile reflow, 0..1 on cubic-bezier(.7,0,.3,1), and where each tile comes from.
    pub reflow: f32,
    pub from: Vec<Option<Rect>>,
    /// Milliseconds since the rows started entering; None once they are all in.
    pub ripple: Option<f32>,
    /// The skeleton's shimmer, 0..1, while a scan runs (None when still).
    pub shimmer: Option<f32>,
    /// The primary button's size suffix: width 0..1 (spring) and opacity (fade).
    pub suffix: f32,
    pub suffix_alpha: f32,
    pub suffix_text: String,
    /// The pointer, in window coordinates, for the tile label card.
    pub mouse: Option<(f32, f32)>,
}

impl Default for Motion {
    fn default() -> Self {
        Motion {
            veil: 0.0,
            dim: 0.0,
            focus: None,
            reflow: 1.0,
            from: Vec::new(),
            ripple: None,
            shimmer: None,
            suffix: 0.0,
            suffix_alpha: 0.0,
            suffix_text: String::new(),
            mouse: None,
        }
    }
}

pub struct View {
    pub screen: Screen,
    pub cards: Vec<Card>,
    pub confirm: Option<ConfirmView>,
    /// The keyboard's row, drawn with a focus ring after a key press.
    pub cursor: Option<usize>,
    pub kb: bool,
    /// The window height the screen has room for (at most `H`); the lists fill it.
    pub room: f32,
    pub motion: Motion,
}

/// How many rows each list shows at the window's height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fit {
    /// Folder rows beside the treemap (the tail row comes after them).
    pub list: usize,
    pub files: usize,
    pub review: usize,
    /// Safe to clean entries (headers and rows) that fit.
    pub clean: usize,
    /// Duplicates entries (group headings and copies) that fit.
    pub dupes: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    ScanDrive(usize),
    ScanFolder,
    OpenClean,
    Stop,
    ScanAgain,
    Back,
    /// The drive chip in Results: back to the drive list.
    Drive,
    Crumb(usize),
    Tab(Tab),
    Tile(usize),
    /// A capacity bar segment.
    Seg(usize),
    Row(usize),
    RowCheck(usize),
    /// Open in Explorer on the detail strip.
    StripOpen,
    File(usize),
    SelOpen,
    SelRecycle,
    SelClear,
    SelectAll,
    Filter(Filter),
    /// A Safe to clean row's body (opens its Review) and its checkbox, by suggestion index.
    Clean(usize),
    CleanCheck(usize),
    BinOpen,
    Settings,
    ReviewFolder,
    Card(usize),
    Cancel,
    Move,
    /// Inside the confirm sheet, on nothing: swallowed so it does not reach the screen under it.
    Sheet,
    /// The title band's close button.
    Close,
    /// Duplicates in the Results header.
    OpenDupes,
    /// "Also find duplicate files" on the drive list.
    DupesOption,
    DupesCheck,
    DupesStop,
    /// "Select all extra copies" (or none).
    DupesAll,
    /// A copy's row, its checkbox and its keep column, by shown group and copy.
    DupeRow(usize, usize),
    DupeCheck(usize, usize),
    DupeKeep(usize, usize),
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

/// Sizes with one decimal, ".0" dropped ("182 GB", "9.4 GB"), base 1024 like Explorer.
pub fn size_label(bytes: u64) -> String {
    let s = crate::disk_model::size(bytes);
    s.replacen(".0 ", " ", 1)
}

/// A percentage with one decimal, ".0" dropped ("41%", "30.6%").
pub fn pct_label(p: f64) -> String {
    let s = format!("{:.1}", p.max(0.0));
    format!("{}%", s.strip_suffix(".0").unwrap_or(&s))
}

/// Height of wrapped text at this width.
fn wrap_h(g: &Shared, s: &str, fmt: &IDWriteTextFormat, w: f32) -> f32 {
    match g.layout(s, fmt, w.max(1.0), 1000.0) {
        Some(l) => unsafe {
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.height.ceil()
        },
        None => 16.0,
    }
}

/// Where the baseline of `s` falls in a box `h` tall (the formats centre their one line).
fn baseline_in(g: &Shared, s: &str, fmt: &IDWriteTextFormat, h: f32) -> f32 {
    let Some(l) = g.layout(s, fmt, 2000.0, h) else {
        return h * 0.7;
    };
    unsafe {
        let mut lm = [DWRITE_LINE_METRICS::default(); 1];
        let mut n = 0u32;
        if l.GetLineMetrics(Some(&mut lm), &mut n).is_ok() && n > 0 {
            (h - lm[0].height) / 2.0 + lm[0].baseline
        } else {
            h * 0.7
        }
    }
}

/// A layout with tracking (em, negative tightens), for the big numbers.
fn tracked(
    g: &Shared,
    s: &str,
    fmt: &IDWriteTextFormat,
    h: f32,
    em: f32,
    size: f32,
) -> Option<IDWriteTextLayout> {
    let l = g.layout(s, fmt, 2000.0, h)?;
    if let Ok(l1) = l.cast::<IDWriteTextLayout1>() {
        unsafe {
            let _ = l1.SetCharacterSpacing(
                0.0,
                em * size,
                0.0,
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: s.encode_utf16().count() as u32,
                },
            );
        }
    }
    Some(l)
}

fn layout_w(l: &IDWriteTextLayout) -> f32 {
    unsafe {
        let mut m = DWRITE_TEXT_METRICS::default();
        let _ = l.GetMetrics(&mut m);
        m.widthIncludingTrailingWhitespace
    }
}

/// The big number at 26/30, weight 500, −0.02em. Returns its width.
fn big_text(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    f: &Fonts,
    s: &str,
    x: f32,
    y: f32,
    b: &ID2D1SolidColorBrush,
) -> f32 {
    match tracked(g, s, &f.big, 30.0, -0.02, 26.0) {
        Some(l) => {
            unsafe { dc.DrawTextLayout(pt(x, y), &l, b, D2D1_DRAW_TEXT_OPTIONS_NONE) };
            layout_w(&l)
        }
        None => {
            text(dc, s, &f.big, x, y, 600.0, 30.0, b);
            g.text_width(s, &f.big)
        }
    }
}

/// Cuts the middle out of a path so its drive and its last folders both show: the longest cut
/// (most characters kept, a third of them from the start) that fits `w`.
///
/// A cut that keeps one more character is never narrower, so the longest fit is found by a
/// binary search: about 8 measurements for a long path instead of one per character dropped.
pub fn middle_cut(s: &str, w: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(s) <= w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let cut = |keep: usize| -> String {
        let head = keep / 3;
        let tail = keep - head;
        chars[..head]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(chars[chars.len() - tail..].iter())
            .collect()
    };
    // Keeps 2..len-1 are the candidates; find the largest that fits.
    let (mut lo, mut hi) = (2usize, chars.len());
    let mut best = None;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let c = cut(mid);
        if measure(&c) <= w {
            best = Some(c);
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    best.unwrap_or_else(|| "…".into())
}

thread_local! {
    /// Cut paths by (path, room, format), so a list that scrolls measures each path once
    /// instead of every frame. Widths are in DIPs, which do not change with the DPI.
    static CUTS: std::cell::RefCell<std::collections::HashMap<(String, u32, usize, u32), String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `middle_cut` measured in `fmt`, remembered per path, room and format.
fn cut_path(g: &Shared, s: &str, w: f32, fmt: &IDWriteTextFormat) -> String {
    let size = unsafe { fmt.GetFontSize() };
    let key = (
        s.to_string(),
        w.to_bits(),
        fmt.as_raw() as usize,
        size.to_bits(),
    );
    if let Some(c) = CUTS.with(|m| m.borrow().get(&key).cloned()) {
        return c;
    }
    let c = middle_cut(s, w, |x| g.text_width(x, fmt));
    CUTS.with(|m| {
        let mut m = m.borrow_mut();
        // Bounded: a long session's paths never pile up.
        if m.len() >= 4096 {
            m.clear();
        }
        m.insert(key, c.clone());
    });
    c
}

thread_local! {
    /// The 11/500 format of the tags (Fonts has no 11 px face; ui.rs belongs to Foundation).
    static TAG_FONT: std::cell::RefCell<Option<IDWriteTextFormat>> = const { std::cell::RefCell::new(None) };
}

fn tag_font(g: &Shared, f: &Fonts) -> IDWriteTextFormat {
    TAG_FONT.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = g.format(11.0, DWRITE_FONT_WEIGHT_MEDIUM).ok();
        }
        c.borrow().clone().unwrap_or_else(|| f.small_medium.clone())
    })
}

// ---- heights ----

/// The "Also find duplicate files" row, under the drives.
fn option_top(n: usize) -> f32 {
    52.0 + n as f32 * 60.0 + 4.0
}

/// Scan a folder and Safe to clean, under the option row.
fn start_buttons_y(n: usize) -> f32 {
    option_top(n) + OPTION_H + 12.0
}

fn start_h(n: usize) -> f32 {
    start_buttons_y(n) + 28.0 + 16.0
}

/// The least each screen needs above the cards. The list screens stretch past it to fill the
/// window; the drive list keeps its own height.
fn min_body(v: &View) -> f32 {
    match &v.screen {
        Screen::Start(d, _) => start_h(d.len()),
        Screen::Results(_) => results_min(),
        Screen::Clean(_) => CLEAN_TOP + 6.0 * CLEAN_ROW + BAR_H,
        Screen::Review(_) => REVIEW_TOP + MIN_ROWS as f32 * FILE_ROW + 8.0 + BAR_H,
        Screen::Dupes(_) => DUPES_TOP + MIN_ROWS as f32 * FILE_ROW + 8.0 + BAR_H,
    }
}

fn results_min() -> f32 {
    BODY_Y + LIST_HEAD + (MIN_ROWS + 1) as f32 * ROW + 4.0 + BAR_H
}

fn card_h(g: &Shared, f: &Fonts, c: &Card) -> f32 {
    let body = wrap_h(g, &c.body, &f.small_wrap, W - X0 * 2.0 - 64.0);
    12.0 + 18.0 + 4.0 + body + if c.button.is_some() { 10.0 + 28.0 } else { 0.0 } + 12.0
}

/// The cards' block under the screen: each card, 8 px between, 12 px under the last.
fn cards_h(g: &Shared, f: &Fonts, cards: &[Card]) -> f32 {
    if cards.is_empty() {
        return 0.0;
    }
    cards.iter().map(|c| card_h(g, f, c) + 8.0).sum::<f32>() + 4.0
}

/// The screen's height above the cards (below the title band).
fn body_from(room: f32, cards: f32, min: f32) -> f32 {
    (room - TOP - cards).floor().max(min)
}

fn body_h(g: &Shared, f: &Fonts, v: &View) -> f32 {
    body_from(v.room, cards_h(g, f, &v.cards), min_body(v))
}

/// Folder rows beside the treemap at this body height; the tail row comes after them.
fn folder_rows(b: f32) -> usize {
    (((b - BAR_H - 4.0 - BODY_Y - LIST_HEAD) / ROW).floor() as usize).max(MIN_ROWS + 1) - 1
}

/// More folder rows than the list can show at the full height: the window disk.rs puts words
/// in.
pub const FOLDER_WINDOW: usize = ((H - TOP - BODY_Y - LIST_HEAD) / ROW) as usize + 1;

/// The treemap's rectangle: as tall as the list header, the rows and the tail row beside it.
fn map_at(b: f32) -> Rect {
    (
        X0,
        BODY_Y,
        MAP_R,
        BODY_Y + LIST_HEAD + (folder_rows(b) + 1) as f32 * ROW + 4.0,
    )
}

fn list_top() -> f32 {
    BODY_Y + LIST_HEAD
}

fn list_at(b: f32) -> Rect {
    let m = map_at(b);
    (LIST_L, list_top(), LIST_R, m.3)
}

fn bar_top(b: f32) -> f32 {
    b - BAR_H
}

fn files_rows(b: f32) -> usize {
    (((bar_top(b) - 8.0 - BODY_Y) / FILE_ROW).floor() as usize).max(1)
}

/// The most Review rows the list can show at once (at the full height): the window disk.rs
/// fills rows for.
pub const REVIEW_WINDOW: usize = ((H - TOP - REVIEW_TOP) / FILE_ROW) as usize + 1;

fn review_rows(b: f32) -> usize {
    (((bar_top(b) - 8.0 - REVIEW_TOP) / FILE_ROW).floor() as usize).max(1)
}

/// Safe to clean, one entry of its scrolling list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Entry {
    Group(usize),
    /// Group, row in the group.
    Row(usize, usize),
    Nothing,
    Extras,
    Bin,
    System,
}

fn entry_h(e: Entry) -> f32 {
    match e {
        Entry::Group(0) => 24.0,
        Entry::Group(_) => 28.0,
        Entry::Row(..) => CLEAN_ROW,
        Entry::Nothing => 76.0,
        Entry::Extras => 30.0,
        Entry::Bin | Entry::System => 52.0,
    }
}

/// Every entry of the list, in order.
pub fn clean_entries(c: &CleanView) -> Vec<Entry> {
    let mut out = Vec::new();
    if c.nothing.is_some() {
        out.push(Entry::Nothing);
    } else {
        for (gi, g) in c.groups.iter().enumerate() {
            out.push(Entry::Group(gi));
            out.extend((0..g.rows.len()).map(|k| Entry::Row(gi, k)));
        }
    }
    out.extend([Entry::Extras, Entry::Bin, Entry::System]);
    out
}

/// The visible entries from `first`, each with its top.
fn clean_visible(c: &CleanView, b: f32) -> Vec<(Entry, f32)> {
    let bottom = bar_top(b) - 4.0;
    let mut y = CLEAN_TOP;
    let mut out = Vec::new();
    for e in clean_entries(c).into_iter().skip(c.first) {
        let h = entry_h(e);
        if y + h > bottom {
            break;
        }
        out.push((e, y));
        y += h;
    }
    out
}

/// Duplicates, one entry of its scrolling list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DupeEntry {
    Group(usize),
    /// Shown group, copy.
    Row(usize, usize),
}

/// The most Duplicates entries the list can show at once (all headings at the full height),
/// plus one: the window disk.rs fills rows for.
pub const DUPES_WINDOW: usize = ((H - TOP - DUPES_TOP) / DUPE_HEAD) as usize + 2;

/// Every entry of the Duplicates list, in order (none before a check has found groups),
/// without collecting them: the list can run to thousands.
pub fn dupe_entry_iter(d: &DupesView) -> impl Iterator<Item = DupeEntry> + '_ {
    dupe_entries_from(d, 0)
}

/// The entries from `first` on, skipping whole groups to get there.
fn dupe_entries_from(d: &DupesView, first: usize) -> impl Iterator<Item = DupeEntry> + '_ {
    let groups: &[DupeGroupView] = match &d.body {
        DupesBody::Groups(groups) => groups,
        _ => &[],
    };
    let mut skip = first;
    let mut start = groups.len();
    for (gi, g) in groups.iter().enumerate() {
        if skip < 1 + g.len {
            start = gi;
            break;
        }
        skip -= 1 + g.len;
    }
    groups[start..]
        .iter()
        .enumerate()
        .flat_map(move |(k, g)| {
            let gi = start + k;
            std::iter::once(DupeEntry::Group(gi))
                .chain((0..g.len).map(move |r| DupeEntry::Row(gi, r)))
        })
        .skip(skip)
}

/// Every entry of the Duplicates list, in order.
#[cfg(test)]
pub fn dupe_entries(d: &DupesView) -> Vec<DupeEntry> {
    dupe_entry_iter(d).collect()
}

/// How many entries the Duplicates list has (headings and copies).
pub fn dupe_entry_count(d: &DupesView) -> usize {
    match &d.body {
        DupesBody::Groups(groups) => groups.iter().map(|g| 1 + g.len).sum(),
        _ => 0,
    }
}

fn dupe_entry_h(e: DupeEntry) -> f32 {
    match e {
        DupeEntry::Group(_) => DUPE_HEAD,
        DupeEntry::Row(..) => FILE_ROW,
    }
}

/// The visible entries from `first`, each with its top.
fn dupe_visible(d: &DupesView, b: f32) -> Vec<(DupeEntry, f32)> {
    let bottom = bar_top(b) - 4.0;
    let mut y = DUPES_TOP;
    let mut out = Vec::new();
    for e in dupe_entries_from(d, d.first) {
        let h = dupe_entry_h(e);
        if y + h > bottom {
            break;
        }
        out.push((e, y));
        y += h;
    }
    out
}

fn clean_fit(b: f32) -> usize {
    (((bar_top(b) - 4.0 - CLEAN_TOP) / CLEAN_ROW).floor() as usize).max(1)
}

/// The rows each list shows in this view.
pub fn fit(g: &Shared, f: &Fonts, v: &View) -> Fit {
    let b = body_h(g, f, v);
    Fit {
        list: folder_rows(b),
        files: files_rows(b),
        review: review_rows(b),
        clean: match &v.screen {
            Screen::Clean(c) => clean_visible(c, b).len().max(1),
            _ => clean_fit(b),
        },
        dupes: match &v.screen {
            Screen::Dupes(d) => dupe_visible(d, b).len().max(1),
            _ => 1,
        },
    }
}

/// The treemap's rectangle for Results at this room with these cards (disk.rs lays the tiles
/// out in it before it builds the view).
pub fn map_rect(g: &Shared, f: &Fonts, room: f32, cards: &[Card]) -> Rect {
    map_at(body_from(room, cards_h(g, f, cards), results_min()))
}

/// Each card's top and height, stacked under the screen.
fn card_tops(g: &Shared, f: &Fonts, v: &View) -> Vec<(f32, f32)> {
    let mut y = body_h(g, f, v);
    v.cards
        .iter()
        .map(|c| {
            let h = card_h(g, f, c);
            let r = (y, h);
            y += h + 8.0;
            r
        })
        .collect()
}

const SHEET_W: f32 = 420.0;

fn sheet_row_h(sub: &str) -> f32 {
    if sub.is_empty() {
        40.0
    } else {
        48.0
    }
}

fn group_h(gr: &ConfirmGroup) -> f32 {
    16.0 + 16.0
        + if gr.check_first { 6.0 + 16.0 } else { 0.0 }
        + 8.0
        + 8.0
        + gr.items.iter().map(|i| sheet_row_h(&i.1)).sum::<f32>()
        + if gr.more.is_some() { 28.0 } else { 0.0 }
}

fn sheet_h(g: &Shared, f: &Fonts, c: &ConfirmView) -> f32 {
    20.0 + 32.0
        + c.groups.iter().map(group_h).sum::<f32>()
        + 14.0
        + wrap_h(g, c.note, &f.small_wrap, SHEET_W - 40.0)
        + 18.0
        + 16.0
        + 32.0
        + 20.0
}

/// The area under the title band: the screen, its cards, room for the confirm sheet.
fn body_size(g: &Shared, f: &Fonts, v: &View) -> (f32, f32) {
    let mut h = body_h(g, f, v) + cards_h(g, f, &v.cards);
    if let Some(c) = &v.confirm {
        h = h.max(sheet_h(g, f, c) + 32.0);
    }
    (W, h.ceil())
}

/// The window's logical size: `room` tall, taller only when a screen and its cards need it.
pub fn size(g: &Shared, f: &Fonts, v: &View) -> (f32, f32) {
    let (w, h) = body_size(g, f, v);
    (w, TOP + h)
}

// ---- layout (hit rects) ----

/// A quiet 28 px button: label 12/500, optional 14 px icon, 10 px padding.
fn quiet_w(g: &Shared, f: &Fonts, label: &str, icon: bool) -> f32 {
    g.text_width(label, &f.small_medium) + 20.0 + if icon { 20.0 } else { 0.0 }
}

/// A 32 px button: label 13/500, 14 px padding, optional icon.
fn tall_w(g: &Shared, f: &Fonts, label: &str, icon: bool) -> f32 {
    g.text_width(label, &f.title) + 28.0 + if icon { 20.0 } else { 0.0 }
}

fn start_rects(g: &Shared, f: &Fonts, n: usize) -> Vec<(Hit, Rect)> {
    let mut out = Vec::new();
    for i in 0..n {
        let t = 52.0 + i as f32 * 60.0;
        out.push((Hit::ScanDrive(i), (X0, t, X1, t + 56.0)));
    }
    let y = start_buttons_y(n);
    let w1 = quiet_w(g, f, "Scan a folder", true);
    out.insert(0, (Hit::ScanFolder, (X0, y, X0 + w1, y + 28.0)));
    let w2 = quiet_w(g, f, "Safe to clean", true);
    out.insert(
        0,
        (
            Hit::OpenClean,
            (X0 + w1 + 8.0, y, X0 + w1 + 8.0 + w2, y + 28.0),
        ),
    );
    let o = option_top(n);
    out.push((Hit::DupesOption, (X0 - 8.0, o, X1 + 8.0, o + OPTION_H)));
    out
}

/// The drive chip at the top left of Results.
fn chip_rect(g: &Shared, f: &Fonts, r: &ResultsView) -> Rect {
    let w =
        36.0 + g.text_width(&r.chip, &f.title) + 6.0 + g.text_width(&r.chip_sub, &f.body) + 12.0;
    (X0, 16.0, X0 + w, 48.0)
}

/// Scan again and Safe to clean (or Stop while scanning), right aligned in the header row.
fn header_buttons(g: &Shared, f: &Fonts, live: bool) -> Vec<(Hit, Rect)> {
    if live {
        let w = quiet_w(g, f, "Stop", true);
        return vec![(Hit::Stop, (X1 - w, 18.0, X1, 46.0))];
    }
    let w1 = quiet_w(g, f, "Scan again", true);
    let w2 = quiet_w(g, f, "Safe to clean", true);
    let w3 = quiet_w(g, f, "Duplicates", true);
    let x2 = X1 - w1 - 4.0;
    let x3 = x2 - w2 - 4.0;
    vec![
        (Hit::ScanAgain, (X1 - w1, 18.0, X1, 46.0)),
        (Hit::OpenClean, (x2 - w2, 18.0, x2, 46.0)),
        (Hit::OpenDupes, (x3 - w3, 18.0, x3, 46.0)),
    ]
}

/// The segmented control, right aligned on the toolbar.
fn tabs_rects(g: &Shared, f: &Fonts) -> [(Tab, Rect); 3] {
    let ws: Vec<f32> = ["Folders", "Files", "Types"]
        .iter()
        .map(|l| g.text_width(l, &f.small_medium) + 24.0)
        .collect();
    let total: f32 = ws.iter().sum::<f32>() + 4.0 + 4.0;
    let mut x = X1 - total + 2.0;
    let mut out = [(Tab::Folders, (0.0, 0.0, 0.0, 0.0)); 3];
    for (i, tab) in [Tab::Folders, Tab::Files, Tab::Types]
        .into_iter()
        .enumerate()
    {
        out[i] = (tab, (x, TOOL_Y + 2.0, x + ws[i], TOOL_Y + 30.0));
        x += ws[i] + 2.0;
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
    let mut from = 0;
    loop {
        let mut w: f32 = (from..n).map(|i| wid(i) + sep).sum::<f32>();
        if from > 0 {
            w += g.text_width("…", &f.body) + sep;
        }
        if X0 + w <= right || from + 1 >= n {
            break;
        }
        from += 1;
    }
    let mut x = X0;
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

/// The capacity bar's segments: (index, left, right). Each is at least 4 px, 2 px apart.
fn seg_layout(segs: &[Seg]) -> Vec<(usize, f32, f32)> {
    let n = segs.len();
    if n == 0 {
        return Vec::new();
    }
    let gaps = 2.0 * (n - 1) as f32;
    let room = (X1 - X0 - gaps).max(1.0);
    let total: f64 = segs.iter().map(|s| s.bytes as f64).sum::<f64>().max(1.0);
    let mut ws: Vec<f32> = segs
        .iter()
        .map(|s| (room as f64 * s.bytes as f64 / total) as f32)
        .map(|w| w.max(4.0))
        .collect();
    // Give the room the minimums took back from the biggest segment.
    let over: f32 = ws.iter().sum::<f32>() - room;
    if over > 0.0 {
        if let Some(big) = ws.iter_mut().max_by(|a, b| a.total_cmp(b)) {
            *big = (*big - over).max(4.0);
        }
    }
    let mut x = X0;
    ws.iter()
        .enumerate()
        .map(|(i, w)| {
            let r = (i, x, x + w);
            x += w + 2.0;
            r
        })
        .collect()
}

/// A folder row's columns from the right: (mini bar left, percent right, size right).
fn row_cols() -> (f32, f32, f32) {
    let size_r = LIST_R - 6.0;
    let pct_r = size_r - SIZE_W - 10.0;
    let bar_l = pct_r - PCT_W - 10.0 - MINI_W;
    (bar_l, pct_r, size_r)
}

/// The bottom-right buttons of a selection bar: (open?, clear, primary).
fn sel_bar_rects(g: &Shared, f: &Fonts, b: f32, open: bool, primary_w: f32) -> Vec<(Hit, Rect)> {
    let t = bar_top(b) + 12.0;
    let r = W - 12.0;
    let mut out = vec![(Hit::SelRecycle, (r - primary_w, t, r, t + 32.0))];
    let cw = tall_w(g, f, "Clear", false);
    let cx = r - primary_w - 8.0;
    out.push((Hit::SelClear, (cx - cw, t, cx, t + 32.0)));
    if open {
        let ow = tall_w(g, f, "Open in Explorer", true);
        let ox = cx - cw - 8.0;
        out.push((Hit::SelOpen, (ox - ow, t, ox, t + 32.0)));
    }
    out
}

const PRIMARY: &str = "Move to Recycle Bin";

/// The primary button's width, springing between its plain and its suffixed label.
fn primary_w(g: &Shared, f: &Fonts, m: &Motion) -> f32 {
    let w0 = tall_w(g, f, PRIMARY, true);
    if m.suffix_text.is_empty() {
        return w0;
    }
    let w1 = w0 + g.text_width(&m.suffix_text, &f.title);
    w0 + (w1 - w0) * m.suffix.clamp(0.0, 1.2)
}

fn strip_open_rect(g: &Shared, f: &Fonts, b: f32) -> Rect {
    let w = quiet_w(g, f, "Open in Explorer", true);
    let t = b - STRIP_H / 2.0 - 14.0;
    (W - 8.0 - w, t, W - 8.0, t + 28.0)
}

fn results_rects(g: &Shared, f: &Fonts, r: &ResultsView, v: &View, b: f32) -> Vec<(Hit, Rect)> {
    let live = r.scanning;
    let mut out: Vec<(Hit, Rect)> = header_buttons(g, f, live);
    if live || v.motion.veil > 0.0 {
        // Only Stop answers while the veil is up; the bar and the list fill in under it.
        return out;
    }
    out.push((Hit::Drive, chip_rect(g, f, r)));
    for (i, l, rr) in seg_layout(&r.segs) {
        out.push((Hit::Seg(i), (l, CBAR_Y - 4.0, rr, CBAR_Y + 16.0)));
    }
    let tabs = tabs_rects(g, f);
    for (i, x, w) in crumb_layout(g, f, r, tabs[0].1 .0 - 12.0) {
        if let Some(i) = i {
            out.push((
                Hit::Crumb(i),
                (x - 2.0, TOOL_Y + 4.0, x + w + 2.0, TOOL_Y + 28.0),
            ));
        }
    }
    for (tab, rect) in tabs {
        out.push((Hit::Tab(tab), rect));
    }
    match r.tab {
        Tab::Folders => {
            for t in &r.tiles {
                if let Some(row) = t.row {
                    out.push((Hit::Tile(row), t.rect));
                }
            }
            for (k, i) in (r.first..r.rows.len().min(r.first + folder_rows(b))).enumerate() {
                let top = list_top() + k as f32 * ROW;
                out.push((Hit::RowCheck(i), (LIST_L, top, LIST_L + 30.0, top + ROW)));
                out.push((Hit::Row(i), (LIST_L, top, LIST_R, top + ROW)));
            }
            if r.selected.is_some() {
                out.extend(sel_bar_rects(g, f, b, false, primary_w(g, f, &v.motion)));
            } else if r.detail.is_some() {
                out.push((Hit::StripOpen, strip_open_rect(g, f, b)));
            }
        }
        Tab::Files => {
            let n = files_rows(b);
            for (k, i) in (r.files_first..r.files.len().min(r.files_first + n)).enumerate() {
                let top = BODY_Y + k as f32 * FILE_ROW;
                out.push((Hit::File(i), (X0 - 8.0, top, X1 + 8.0, top + ROW)));
            }
            out.extend(sel_bar_rects(g, f, b, true, primary_w(g, f, &v.motion)));
        }
        Tab::Types => {}
    }
    out
}

/// The filter chips: All, Safe, Check first with their counts.
fn filter_rects(g: &Shared, f: &Fonts, c: &CleanView) -> [(Filter, Rect); 3] {
    let mut x = X0;
    let mut out = [(Filter::All, (0.0, 0.0, 0.0, 0.0)); 3];
    for (i, (fl, label)) in [
        (Filter::All, "All"),
        (Filter::Safe, "Safe"),
        (Filter::Check, "Check first"),
    ]
    .into_iter()
    .enumerate()
    {
        let w = g.text_width(label, &f.small_medium)
            + 6.0
            + g.text_width(&c.counts[i].to_string(), &f.small)
            + 24.0;
        out[i] = (fl, (x, 56.0, x + w, 84.0 - 0.0));
        x += w + 6.0;
    }
    out
}

fn clean_header_total_x(g: &Shared, f: &Fonts, c: &CleanView) -> f32 {
    X1 - g.text_width(" to review", &f.body) - g.text_width(&c.total, &f.heading)
}

fn extra_button(g: &Shared, f: &Fonts, label: &str, top: f32) -> Rect {
    let w = quiet_w(g, f, label, false);
    (X1 - w, top + 12.0, X1, top + 40.0)
}

fn clean_rects(g: &Shared, f: &Fonts, c: &CleanView, v: &View, b: f32) -> Vec<(Hit, Rect)> {
    let mut out = vec![(Hit::Back, (X0, 14.0, X0 + 28.0, 42.0))];
    for (fl, r) in filter_rects(g, f, c) {
        out.push((Hit::Filter(fl), r));
    }
    for (e, top) in clean_visible(c, b) {
        match e {
            Entry::Row(gi, k) => {
                let row = &c.groups[gi].rows[k];
                out.push((
                    Hit::CleanCheck(row.index),
                    (X0 - 8.0, top, X0 + 24.0, top + CLEAN_ROW),
                ));
                if row.can_review {
                    out.push((
                        Hit::Clean(row.index),
                        (X0 + 24.0, top, X1 + 8.0, top + CLEAN_ROW),
                    ));
                }
            }
            Entry::Bin => out.push((Hit::BinOpen, extra_button(g, f, "Open", top))),
            Entry::System => out.push((Hit::Settings, extra_button(g, f, "Settings", top))),
            _ => {}
        }
    }
    out.extend(sel_bar_rects(g, f, b, false, primary_w(g, f, &v.motion)));
    out
}

fn review_rects(r: &ReviewView, g: &Shared, f: &Fonts, v: &View, b: f32) -> Vec<(Hit, Rect)> {
    let mut out = vec![
        (Hit::Back, (X0, 16.0, X0 + 28.0, 44.0)),
        (Hit::ReviewFolder, (X1 - 28.0, 16.0, X1, 44.0)),
    ];
    let aw = g.text_width(if r.all { "Select none" } else { "Select all" }, &f.small) + 12.0;
    out.push((Hit::SelectAll, (X1 - aw, 50.0, X1, 70.0)));
    for (k, i) in (r.first..r.len.min(r.first + review_rows(b))).enumerate() {
        let top = REVIEW_TOP + k as f32 * FILE_ROW;
        out.push((Hit::File(i), (X0 - 8.0, top, X1 + 8.0, top + ROW)));
    }
    out.extend(sel_bar_rects(g, f, b, true, primary_w(g, f, &v.motion)));
    out
}

/// "Select all extra copies" in the filter-chip style, right aligned under the header.
fn dupes_all_rect(g: &Shared, f: &Fonts, d: &DupesView) -> Rect {
    let w = g.text_width(dupes_all_label(d), &f.small_medium) + 24.0;
    (X1 - w, 56.0, X1, 84.0)
}

fn dupes_all_label(d: &DupesView) -> &'static str {
    if d.all {
        "Select none"
    } else {
        "Select all extra copies"
    }
}

/// The card that holds the estimate, the running check or the empty result: its rect, the
/// text's left edge and width.
fn dupes_block(g: &Shared, f: &Fonts, d: &DupesView) -> Option<(Rect, f32, f32)> {
    let tx = X0 + 56.0;
    let tw = X1 - 16.0 - tx;
    let y1 = DUPES_TOP + 16.0 + 24.0;
    let bottom = match &d.body {
        DupesBody::Offer {
            body, cost, check, ..
        } => {
            let y2 = y1 + wrap_h(g, body, &f.small_wrap, tw) + 6.0;
            let y3 = y2 + wrap_h(g, cost, &f.small_wrap, tw);
            if *check {
                y3 + 14.0 + 28.0 + 16.0
            } else {
                y3 + 16.0
            }
        }
        DupesBody::Running { .. } => y1 + 16.0 + 16.0 + 16.0,
        DupesBody::Empty { body, .. } => y1 + wrap_h(g, body, &f.small_wrap, tw) + 16.0,
        DupesBody::Groups(_) => return None,
    };
    Some(((X0, DUPES_TOP, X1, bottom), tx, tw))
}

/// Check now (under the estimate) or Stop (top right of the running card).
fn dupes_block_button(g: &Shared, f: &Fonts, d: &DupesView) -> Option<(Hit, Rect)> {
    let (r, tx, _) = dupes_block(g, f, d)?;
    match &d.body {
        DupesBody::Offer { check: true, .. } => {
            let w = quiet_w(g, f, "Check now", true);
            Some((Hit::DupesCheck, (tx, r.3 - 16.0 - 28.0, tx + w, r.3 - 16.0)))
        }
        DupesBody::Running { .. } => {
            let w = quiet_w(g, f, "Stop", true);
            Some((
                Hit::DupesStop,
                (r.2 - 16.0 - w, r.1 + 12.0, r.2 - 16.0, r.1 + 40.0),
            ))
        }
        _ => None,
    }
}

/// A copy's keep column: "Keep" on the keeper, "Keep this" on the others.
fn keep_rect(top: f32) -> Rect {
    let r = X1 - DATE_W - 12.0;
    let cy = top + ROW / 2.0;
    (r - KEEP_W, cy - 11.0, r, cy + 11.0)
}

fn dupes_rects(g: &Shared, f: &Fonts, d: &DupesView, v: &View, b: f32) -> Vec<(Hit, Rect)> {
    let mut out = vec![(Hit::Back, (X0, 14.0, X0 + 28.0, 42.0))];
    out.extend(dupes_block_button(g, f, d));
    if let DupesBody::Groups(groups) = &d.body {
        out.push((Hit::DupesAll, dupes_all_rect(g, f, d)));
        for (e, top) in dupe_visible(d, b) {
            if let DupeEntry::Row(gi, k) = e {
                let Some(row) = groups[gi].rows.get(k) else {
                    continue;
                };
                if !row.keep {
                    out.push((Hit::DupeCheck(gi, k), (X0 - 8.0, top, X0 + 24.0, top + ROW)));
                }
                out.push((Hit::DupeKeep(gi, k), keep_rect(top)));
                out.push((Hit::DupeRow(gi, k), (X0 - 8.0, top, X1 + 8.0, top + ROW)));
            }
        }
    }
    out.extend(sel_bar_rects(g, f, b, true, primary_w(g, f, &v.motion)));
    out
}

/// The confirm sheet, centred in the area under the title band.
fn sheet_rect(g: &Shared, f: &Fonts, v: &View, c: &ConfirmView) -> Rect {
    let (w, h) = body_size(g, f, v);
    let sh = sheet_h(g, f, c);
    let l = (w - SHEET_W) / 2.0;
    let t = ((h - sh) / 2.0).max(16.0);
    (l, t, l + SHEET_W, t + sh)
}

fn sheet_buttons(g: &Shared, f: &Fonts, s: Rect, c: &ConfirmView) -> [(Hit, Rect); 2] {
    let mw = tall_w(g, f, &c.action, false);
    let cw = tall_w(g, f, "Cancel", false);
    let (b, r) = (s.3 - 20.0, s.2 - 20.0);
    [
        (Hit::Cancel, (r - mw - 8.0 - cw, b - 32.0, r - mw - 8.0, b)),
        (Hit::Move, (r - mw, b - 32.0, r, b)),
    ]
}

/// The hit rects under the title band, in its coordinates (y from the band's bottom edge).
fn layout(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    if let Some(c) = &v.confirm {
        let s = sheet_rect(g, f, v, c);
        let mut out: Vec<(Hit, Rect)> = sheet_buttons(g, f, s, c).to_vec();
        out.push((Hit::Sheet, (0.0, 0.0, 10_000.0, 10_000.0)));
        return out;
    }
    let b = body_h(g, f, v);
    let mut out = match &v.screen {
        Screen::Start(d, _) => start_rects(g, f, d.len()),
        Screen::Results(r) => results_rects(g, f, r, v, b),
        Screen::Clean(c) => clean_rects(g, f, c, v, b),
        Screen::Review(r) => review_rects(r, g, f, v, b),
        Screen::Dupes(d) => dupes_rects(g, f, d, v, b),
    };
    for (i, ((top, h), c)) in card_tops(g, f, v).into_iter().zip(&v.cards).enumerate() {
        if let Some(label) = &c.button {
            let bw = quiet_w(g, f, label, false);
            let x = X0 + 48.0;
            out.push((
                Hit::Card(i),
                (x, top + h - 12.0 - 28.0, x + bw, top + h - 12.0),
            ));
        }
    }
    out
}

/// The title band's close button, in window coordinates.
fn close_rect() -> Rect {
    (W - 8.0 - 28.0, 8.0, W - 8.0, 36.0)
}

/// What is under (x, y) in window coordinates. The band's empty part is None (it drags the
/// window); while the confirm sheet is up, only its buttons answer.
pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    if v.confirm.is_none() && inside(&close_rect(), x, y) {
        return Some(Hit::Close);
    }
    if y < TOP {
        return None;
    }
    layout(g, f, v)
        .into_iter()
        .find(|(_, r)| inside(r, x, y - TOP))
        .map(|(h, _)| h)
}

// ---- drawing helpers ----

/// Hover is half of active, so the selected row always reads stronger.
fn hover_fill(t: &Theme) -> D2D1_COLOR_F {
    if t.dark {
        rgba(0xFFFFFF, 0.04)
    } else {
        rgba(0x000000, 0.025)
    }
}

/// Free space and empty wells: a light wash, edged with a hairline.
fn wash(t: &Theme) -> (D2D1_COLOR_F, D2D1_COLOR_F) {
    if t.dark {
        (rgba(0xFFFFFF, 0.05), rgba(0xFFFFFF, 0.06))
    } else {
        (rgba(0x000000, 0.05), rgba(0x000000, 0.05))
    }
}

/// The six folder greys, biggest first; in dark the biggest is the lightest.
pub fn folder_grey(dark: bool, i: usize) -> u32 {
    const LIGHT: [u32; 6] = [0x171717, 0x525252, 0x8A8A8A, 0xB5B5B5, 0xD4D4D4, 0xE5E5E5];
    const DARK: [u32; 6] = [0xFAFAFA, 0xC2C2C2, 0x8F8F8F, 0x666666, 0x4A4A4A, 0x303030];
    if dark {
        DARK[i.min(5)]
    } else {
        LIGHT[i.min(5)]
    }
}

fn grey(t: &Theme, i: usize) -> D2D1_COLOR_F {
    rgba(folder_grey(t.dark, i), 1.0)
}

/// The palest grey needs a hairline to hold its edge.
fn grey_edge(t: &Theme) -> D2D1_COLOR_F {
    if t.dark {
        rgba(0xFFFFFF, 0.1)
    } else {
        rgba(0x000000, 0.08)
    }
}

fn fill_rr(dc: &ID2D1DeviceContext, r: Rect, rad: f32, c: D2D1_COLOR_F) {
    unsafe { dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, rad), &brush(dc, c)) };
}

fn edge_rr(dc: &ID2D1DeviceContext, r: Rect, rad: f32, c: D2D1_COLOR_F, w: f32) {
    let i = w / 2.0;
    unsafe {
        dc.DrawRoundedRectangle(
            &rrect(r.0 + i, r.1 + i, r.2 - i, r.3 - i, (rad - i).max(0.0)),
            &brush(dc, c),
            w,
            None,
        )
    };
}

/// A rounded bar piece with its own left and right radius (the capacity bar's ends are round,
/// the joins between segments nearly square).
fn seg_shape(g: &Shared, l: f32, r: f32, y: f32, h: f32, rl: f32, rr: f32) -> ID2D1PathGeometry {
    let half = (r - l) / 2.0;
    let (rl, rr) = (rl.min(half).min(h / 2.0), rr.min(half).min(h / 2.0));
    path(g, pt(l + rl, y), true, |k| {
        line_to(k, pt(r - rr, y));
        if rr > 0.0 {
            arc_to(k, pt(r, y + rr), rr, false, true);
        }
        line_to(k, pt(r, y + h - rr));
        if rr > 0.0 {
            arc_to(k, pt(r - rr, y + h), rr, false, true);
        }
        line_to(k, pt(l + rl, y + h));
        if rl > 0.0 {
            arc_to(k, pt(l, y + h - rl), rl, false, true);
        }
        line_to(k, pt(l, y + rl));
        if rl > 0.0 {
            arc_to(k, pt(l + rl, y), rl, false, true);
        }
    })
}

/// A quiet button (28 px): no fill until hovered, icon in muted, label 12/500 in ink.
fn quiet_button(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: Rect,
    label: &str,
    icon: Option<Icon>,
    hover: bool,
    filled: bool,
) {
    let h = r.3 - r.1;
    if hover || filled {
        fill_rr(dc, r, h / 2.0, if hover { t.button_hover } else { t.chip });
    }
    let mut x = r.0 + 10.0;
    if let Some(i) = icon {
        draw_icon(
            g,
            dc,
            i,
            x + 7.0,
            (r.1 + r.3) / 2.0,
            14.0,
            &brush(dc, t.muted),
        );
        x += 20.0;
    }
    text(
        dc,
        label,
        &f.small_medium,
        x,
        r.1,
        r.2 - x + 2.0,
        h,
        &brush(dc, t.text),
    );
}

/// A 32 px button, 13/500: primary (ink fill) or quiet (active fill on hover only).
fn tall_button(
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
        Some(with_alpha(t.text, if hover && enabled { 0.85 } else { a }))
    } else if hover && enabled {
        Some(t.chip)
    } else {
        None
    };
    if let Some(c) = fill {
        fill_rr(dc, r, 16.0, c);
    }
    let ink = if primary {
        t.panel
    } else {
        with_alpha(t.text, a)
    };
    let b = brush(dc, ink);
    let mut x = r.0 + 14.0;
    if let Some(i) = icon {
        draw_icon(g, dc, i, x + 7.0, (r.1 + r.3) / 2.0, 14.0, &b);
        x += 20.0;
    }
    text(dc, label, &f.title, x, r.1, r.2 - x + 2.0, r.3 - r.1, &b);
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
            &brush(dc, t.text),
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
    unsafe {
        dc.FillEllipse(&ellipse(cx, cy, r), &brush(dc, fill));
        dc.DrawEllipse(&ellipse(cx, cy, r - 0.5), &brush(dc, t.hair), 1.0, None);
    }
    draw_icon(g, dc, icon, cx, cy, (r * 0.9).min(16.0), &brush(dc, ink));
}

/// The 24 px rounded-square chip of a list row.
fn square_chip(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    x: f32,
    cy: f32,
    icon: Icon,
    alpha: f32,
) {
    let r = (x, cy - 12.0, x + 24.0, cy + 12.0);
    fill_rr(dc, r, 7.0, with_alpha(t.chip, alpha));
    edge_rr(dc, r, 7.0, with_alpha(t.hair, alpha), 1.0);
    draw_icon(
        g,
        dc,
        icon,
        x + 12.0,
        cy,
        14.0,
        &brush(dc, with_alpha(t.text, alpha)),
    );
}

fn checkbox(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, x: f32, cy: f32, on: bool, alpha: f32) {
    let r = (x, cy - 8.0, x + 16.0, cy + 8.0);
    if on {
        fill_rr(dc, r, 5.0, with_alpha(t.text, alpha));
        draw_icon(
            g,
            dc,
            Icon::Tick,
            x + 8.0,
            cy,
            12.0,
            &brush(dc, with_alpha(t.panel, alpha)),
        );
    } else {
        fill_rr(dc, r, 5.0, with_alpha(t.panel, alpha));
        let edge = if t.dark {
            rgba(0xFFFFFF, 0.25)
        } else {
            rgba(0x000000, 0.22)
        };
        edge_rr(dc, r, 5.0, with_alpha(edge, alpha), 1.0);
    }
}

/// The 4 px bar of a row against its biggest sibling.
fn mini_bar(dc: &ID2D1DeviceContext, t: &Theme, x: f32, cy: f32, w: f32, frac: f32, alpha: f32) {
    let track = if t.dark {
        rgba(0xFFFFFF, 0.08)
    } else {
        rgba(0x000000, 0.07)
    };
    fill_rr(
        dc,
        (x, cy - 2.0, x + w, cy + 2.0),
        2.0,
        with_alpha(track, alpha),
    );
    let fw = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { 4.0 } else { 0.0 });
    if fw > 0.0 {
        fill_rr(
            dc,
            (x, cy - 2.0, x + fw, cy + 2.0),
            2.0,
            with_alpha(t.text, alpha),
        );
    }
}

fn track_bar(dc: &ID2D1DeviceContext, t: &Theme, x: f32, y: f32, w: f32, h: f32, frac: f32) {
    fill_rr(dc, (x, y, x + w, y + h), h / 2.0, t.track);
    let fw = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
    if fw > 0.0 {
        fill_rr(dc, (x, y, x + fw, y + h), h / 2.0, t.text);
    }
}

/// The window's card: radius 16, like Settings (the window region matches it).
fn window_bg(dc: &ID2D1DeviceContext, t: &Theme, w: f32, h: f32) {
    fill_rr(dc, (0.0, 0.0, w, h), 16.0, t.panel);
    edge_rr(dc, (0.0, 0.0, w, h), 16.0, t.hair, 1.0);
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

/// The "Check first" eye: an almond and its pupil, 12 px.
fn eye(g: &Shared, dc: &ID2D1DeviceContext, cx: f32, cy: f32, b: &ID2D1SolidColorBrush) {
    let s = 0.5;
    let p = |x: f32, y: f32| pt(cx + (x - 12.0) * s, cy + (y - 12.0) * s);
    let geo = path(g, p(2.5, 12.0), false, |k| {
        quad_to(k, p(12.0, 2.0), p(21.5, 12.0));
        quad_to(k, p(12.0, 22.0), p(2.5, 12.0));
    });
    unsafe {
        dc.DrawGeometry(&geo, b, 1.1, &round_stroke(g));
        dc.DrawEllipse(&ellipse(cx, cy, 1.5), b, 1.1, None);
    }
}

/// The confirm sheet's warning mark: a 16 px outline circle with "!" in ink.
fn bang(dc: &ID2D1DeviceContext, t: &Theme, cx: f32, cy: f32) {
    let b = brush(dc, t.text);
    unsafe {
        dc.DrawEllipse(&ellipse(cx, cy, 6.25), &b, 1.25, None);
        dc.DrawLine(pt(cx, cy - 3.25), pt(cx, cy + 0.75), &b, 1.4, None);
        dc.FillEllipse(&ellipse(cx, cy + 3.0, 0.8), &b);
    }
}

/// A safety tag, 20 px tall: "Check first" (outline at ink 40%, eye, ink text) or "Safe to
/// remove" (active fill, muted text). Right aligned at `right`. Returns its width.
fn tag(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    check_first: bool,
    right: f32,
    cy: f32,
) -> f32 {
    let tf = tag_font(g, f);
    let label = if check_first {
        "Check first"
    } else {
        "Safe to remove"
    };
    let tw = g.text_width(label, &tf);
    let w = tw + 16.0 + if check_first { 16.0 } else { 0.0 };
    let r = (right - w, cy - 10.0, right, cy + 10.0);
    if check_first {
        edge_rr(dc, r, 10.0, with_alpha(t.text, 0.4), 1.0);
        let ink = brush(dc, t.text);
        eye(g, dc, r.0 + 8.0 + 6.0, cy, &ink);
        text(dc, label, &tf, r.0 + 8.0 + 16.0, r.1, tw + 2.0, 20.0, &ink);
    } else {
        fill_rr(dc, r, 10.0, t.chip);
        text(
            dc,
            label,
            &tf,
            r.0 + 8.0,
            r.1,
            tw + 2.0,
            20.0,
            &brush(dc, t.muted),
        );
    }
    w
}

/// The selection bar: "3 selected · 11.3 GB" and the buttons, under a hairline.
fn draw_sel_bar(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    b: f32,
    selected: &Option<(String, String)>,
    can_recycle: bool,
    open: bool,
    hover: Option<Hit>,
) {
    let top = bar_top(b);
    hairline(dc, t, 0.0, W, top);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let cy = top + BAR_H / 2.0;
    match selected {
        Some((n, size)) => {
            let nw = g.text_width(n, &f.title);
            text(dc, n, &f.title, X0, cy - 10.0, nw + 2.0, 20.0, &ink);
            text(
                dc,
                &format!(" · {size}"),
                &f.body,
                X0 + nw,
                cy - 10.0,
                160.0,
                20.0,
                &muted,
            );
        }
        None => text(
            dc,
            "Nothing selected",
            &f.body,
            X0,
            cy - 10.0,
            200.0,
            20.0,
            &muted,
        ),
    }
    let pw = primary_w(g, f, &v.motion);
    let any = selected.is_some();
    for (h, r) in sel_bar_rects(g, f, b, open, pw) {
        match h {
            Hit::SelRecycle => {
                let enabled = any && can_recycle;
                tall_button(
                    g,
                    dc,
                    t,
                    f,
                    r,
                    PRIMARY,
                    Some(Icon::Trash),
                    true,
                    hover == Some(h),
                    enabled,
                );
                let m = &v.motion;
                if !m.suffix_text.is_empty() && m.suffix_alpha > 0.0 {
                    let x = r.0 + 14.0 + 20.0 + g.text_width(PRIMARY, &f.title);
                    unsafe {
                        dc.PushAxisAlignedClip(
                            &D2D_RECT_F {
                                left: r.0,
                                top: r.1,
                                right: r.2 - 10.0,
                                bottom: r.3,
                            },
                            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        )
                    };
                    text(
                        dc,
                        &m.suffix_text,
                        &f.title,
                        x,
                        r.1,
                        200.0,
                        32.0,
                        &brush(dc, with_alpha(t.panel, m.suffix_alpha)),
                    );
                    unsafe { dc.PopAxisAlignedClip() };
                }
            }
            Hit::SelClear => {
                tall_button(g, dc, t, f, r, "Clear", None, false, hover == Some(h), any)
            }
            _ => tall_button(
                g,
                dc,
                t,
                f,
                r,
                "Open in Explorer",
                Some(Icon::External),
                false,
                hover == Some(h),
                any,
            ),
        }
    }
}

// ---- screens ----

fn draw_start(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    drives: &[DriveRow],
    dupes: bool,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    text(
        dc,
        "Pick a drive",
        &f.panel_title,
        X0,
        16.0,
        200.0,
        18.0,
        &ink,
    );
    right_text(
        g,
        dc,
        "What is taking the room",
        &f.small,
        X1,
        17.0,
        16.0,
        &muted,
    );
    for (i, d) in drives.iter().enumerate() {
        let top = 52.0 + i as f32 * 60.0;
        let on = hover == Some(Hit::ScanDrive(i));
        if on {
            fill_rr(
                dc,
                (X0 - 8.0, top, X1 + 8.0, top + 56.0),
                10.0,
                hover_fill(t),
            );
        }
        if v.kb && v.cursor == Some(i) {
            focus_ring(dc, t, (X0 - 8.0, top, X1 + 8.0, top + 56.0), 10.0);
        }
        chip(
            g,
            dc,
            t,
            X0 + 16.0,
            top + 22.0,
            16.0,
            Icon::HardDrive,
            false,
        );
        let bw = quiet_w(g, f, "Scan", false);
        clipped_text(
            g,
            dc,
            &d.name,
            &f.title,
            X0 + 44.0,
            top + 6.0,
            X1 - X0 - 56.0 - bw,
            18.0,
            &ink,
        );
        text(
            dc,
            &d.free,
            &f.small,
            X0 + 44.0,
            top + 24.0,
            260.0,
            16.0,
            &muted,
        );
        track_bar(
            dc,
            t,
            X0 + 44.0,
            top + 45.0,
            X1 - X0 - 56.0 - bw - 12.0,
            4.0,
            d.used,
        );
        quiet_button(
            g,
            dc,
            t,
            f,
            (X1 - bw, top + 14.0, X1, top + 42.0),
            "Scan",
            None,
            on,
            true,
        );
    }
    // "Also find duplicate files", off by default, with what it costs under it.
    let n = drives.len();
    let o = option_top(n);
    let row = (X0 - 8.0, o, X1 + 8.0, o + OPTION_H);
    if hover == Some(Hit::DupesOption) {
        fill_rr(dc, row, 10.0, hover_fill(t));
    }
    if v.kb && v.cursor == Some(n) {
        focus_ring(dc, t, row, 10.0);
    }
    checkbox(g, dc, t, X0, o + 13.0, dupes, 1.0);
    text(
        dc,
        "Also find duplicate files",
        &f.title,
        X0 + 28.0,
        o + 4.0,
        400.0,
        18.0,
        &ink,
    );
    clipped_text(
        g,
        dc,
        crate::disk_model::dupe_cost_words(None),
        &f.small,
        X0 + 28.0,
        o + 22.0,
        X1 - X0 - 28.0,
        16.0,
        &muted,
    );
    for (h, r) in start_rects(g, f, drives.len()).into_iter().take(2) {
        let (label, icon) = if h == Hit::ScanFolder {
            ("Scan a folder", Icon::Folder)
        } else {
            ("Safe to clean", Icon::Shield)
        };
        quiet_button(g, dc, t, f, r, label, Some(icon), hover == Some(h), true);
    }
}

/// The header row, the headline and the capacity bar with its legend.
fn draw_head(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    // Drive chip: a 32 px pill with a 24 px round icon chip, the name and the scan stamp.
    let cr = chip_rect(g, f, r);
    fill_rr(
        dc,
        cr,
        16.0,
        if hover == Some(Hit::Drive) {
            t.button_hover
        } else {
            t.chip
        },
    );
    edge_rr(dc, cr, 16.0, t.hair, 1.0);
    unsafe {
        dc.FillEllipse(&ellipse(X0 + 16.0, 32.0, 12.0), &brush(dc, t.panel));
        dc.DrawEllipse(
            &ellipse(X0 + 16.0, 32.0, 12.0),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    draw_icon(
        g,
        dc,
        if r.folder {
            Icon::Folder
        } else {
            Icon::HardDrive
        },
        X0 + 16.0,
        32.0,
        14.0,
        &ink,
    );
    let nw = g.text_width(&r.chip, &f.title);
    text(dc, &r.chip, &f.title, X0 + 36.0, 16.0, nw + 2.0, 32.0, &ink);
    text(
        dc,
        &r.chip_sub,
        &f.body,
        X0 + 36.0 + nw + 6.0,
        16.0,
        200.0,
        32.0,
        &muted,
    );
    for (h, rect) in header_buttons(g, f, r.scanning) {
        let (label, icon) = match h {
            Hit::Stop => ("Stop", Icon::Stop),
            Hit::ScanAgain => ("Scan again", Icon::Refresh),
            Hit::OpenDupes => ("Duplicates", Icon::Layers),
            _ => ("Safe to clean", Icon::Shield),
        };
        quiet_button(
            g,
            dc,
            t,
            f,
            rect,
            label,
            Some(icon),
            hover == Some(h),
            false,
        );
    }
    // Headline: "438 GB used" with "of 476 GB" on its baseline, "38 GB free" on the right.
    let bw = big_text(g, dc, f, &r.big, X0, HEAD_Y, &ink);
    let base = HEAD_Y + baseline_in(g, &r.big, &f.big, 30.0);
    let sy = base - baseline_in(g, &r.of, &f.body, 18.0);
    text(dc, &r.of, &f.body, X0 + bw + 8.0, sy, 300.0, 18.0, &muted);
    right_text(g, dc, &r.right, &f.body, X1, sy, 18.0, &muted);
    // Capacity bar, by top-level folder.
    let lit = |i: usize| -> f32 {
        let on = match v.motion.focus {
            Some(Focus::Row(row)) => r.segs[i].row == Some(row),
            Some(Focus::Seg(s)) => s == i,
            None => true,
        };
        if on || v.motion.focus.is_none() {
            1.0
        } else {
            1.0 - 0.7 * v.motion.dim
        }
    };
    let pieces = seg_layout(&r.segs);
    let n = pieces.len();
    for (k, &(i, l, rr)) in pieces.iter().enumerate() {
        let rl = if k == 0 { 6.0 } else { 2.0 };
        let rrad = if k + 1 == n { 6.0 } else { 2.0 };
        let geo = seg_shape(g, l, rr, CBAR_Y, 12.0, rl, rrad);
        let a = lit(i);
        unsafe {
            match r.segs[i].kind {
                SegKind::Grey(gi) => {
                    dc.FillGeometry(&geo, &brush(dc, with_alpha(grey(t, gi), a)), None);
                    if gi == 5 {
                        dc.DrawGeometry(&geo, &brush(dc, with_alpha(grey_edge(t), a)), 1.0, None);
                    }
                }
                SegKind::Free => {
                    let (fill, edge) = wash(t);
                    dc.FillGeometry(&geo, &brush(dc, with_alpha(fill, a)), None);
                    dc.DrawGeometry(&geo, &brush(dc, with_alpha(edge, a)), 1.0, None);
                }
                SegKind::Dotted => {
                    let (_, edge) = wash(t);
                    dc.DrawGeometry(&geo, &brush(dc, with_alpha(edge, a)), 1.0, None);
                    let dots = if t.dark {
                        rgba(0x666666, a)
                    } else {
                        rgba(0xB5B5B5, a)
                    };
                    if rr - l > 8.0 {
                        dc.DrawLine(
                            pt(l + 4.0, CBAR_Y + 6.0),
                            pt(rr - 4.0, CBAR_Y + 6.0),
                            &brush(dc, dots),
                            2.0,
                            &dotted_stroke(g),
                        );
                    }
                }
            }
        }
    }
    // Legend: an 8 px swatch and the name, 14 px apart; what does not fit is left out.
    let mut x = X0;
    for (i, s) in r.segs.iter().enumerate() {
        let w = 14.0 + g.text_width(&s.label, &f.small);
        if x + w > X1 {
            break;
        }
        let sw = (x, LEG_Y + 4.0, x + 8.0, LEG_Y + 12.0);
        let a = lit(i);
        match s.kind {
            SegKind::Grey(gi) => {
                fill_rr(dc, sw, 2.0, with_alpha(grey(t, gi), a));
                if gi == 5 {
                    edge_rr(dc, sw, 2.0, with_alpha(grey_edge(t), a), 1.0);
                }
            }
            SegKind::Free => {
                let (fill, _) = wash(t);
                fill_rr(dc, sw, 2.0, with_alpha(fill, a));
                edge_rr(dc, sw, 2.0, with_alpha(grey_edge(t), a), 1.0);
            }
            SegKind::Dotted => {
                edge_rr(dc, sw, 2.0, with_alpha(grey_edge(t), a), 1.0);
                unsafe {
                    dc.FillEllipse(
                        &ellipse(x + 4.0, LEG_Y + 8.0, 1.0),
                        &brush(dc, with_alpha(t.passed, a)),
                    )
                };
            }
        }
        text(
            dc,
            &s.label,
            &f.small,
            x + 14.0,
            LEG_Y,
            w,
            16.0,
            &brush(dc, with_alpha(t.muted, a)),
        );
        x += w + 14.0;
    }
}

/// The toolbar: the breadcrumb and the Folders / Files / Types control.
fn draw_toolbar(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let tabs = tabs_rects(g, f);
    let n = r.crumbs.len();
    let pieces = crumb_layout(g, f, r, tabs[0].1 .0 - 12.0);
    for (k, (i, x, w)) in pieces.iter().enumerate() {
        let last = *i == Some(n - 1);
        let label = i.map_or("…", |i| r.crumbs[i].as_str());
        let fnt = if last { &f.title } else { &f.body };
        let c = if last || matches!(hover, Some(Hit::Crumb(j)) if Some(j) == *i) {
            &ink
        } else {
            &muted
        };
        text(dc, label, fnt, *x, TOOL_Y, w + 2.0, 32.0, c);
        if k + 1 < pieces.len() {
            draw_icon(
                g,
                dc,
                Icon::ChevronRight,
                x + w + 10.0,
                TOOL_Y + 16.0,
                12.0,
                &brush(dc, t.passed),
            );
        }
    }
    let live = r.scanning;
    let seg = (
        tabs[0].1 .0 - 2.0,
        TOOL_Y,
        tabs[2].1 .2 + 2.0,
        TOOL_Y + 32.0,
    );
    fill_rr(dc, seg, 16.0, t.raised);
    edge_rr(dc, seg, 16.0, t.hair, 1.0);
    for (tab, rc) in tabs {
        let on = tab == r.tab;
        if on {
            if t.dark {
                fill_rr(dc, rc, 14.0, rgba(0xFFFFFF, 0.1));
            } else {
                fill_rr(
                    dc,
                    (rc.0, rc.1 + 1.0, rc.2, rc.3 + 1.0),
                    14.0,
                    rgba(0x000000, 0.06),
                );
                fill_rr(dc, rc, 14.0, t.panel);
                edge_rr(dc, rc, 14.0, t.hair, 1.0);
            }
        }
        let label = match tab {
            Tab::Folders => "Folders",
            Tab::Files => "Files",
            Tab::Types => "Types",
        };
        let c = if on || (!live && hover == Some(Hit::Tab(tab))) {
            &ink
        } else {
            &muted
        };
        center_text(
            g,
            dc,
            label,
            &f.small_medium,
            (rc.0 + rc.2) / 2.0,
            rc.1,
            28.0,
            c,
        );
    }
}

/// The rows' entrance (Foundation's ripple): y 6 → 0 and opacity 0 → 1, 200 ms each on
/// `settle`, 30 ms apart. Returns (dy, alpha).
fn ripple(m: &Motion, k: usize) -> (f32, f32) {
    match m.ripple {
        None => (0.0, 1.0),
        Some(ms) => {
            let fx = motion::ripple_at(ms, k);
            (fx.dy, fx.alpha)
        }
    }
}

fn draw_folder_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    i: usize,
    k: usize,
    hover: Option<Hit>,
) {
    let row = &r.rows[i];
    let (dy, a) = ripple(&v.motion, k);
    let top = list_top() + k as f32 * ROW + dy;
    let ink = brush(dc, with_alpha(t.text, a));
    let muted = brush(dc, with_alpha(t.muted, a));
    let rect = (LIST_L, top, LIST_R, top + ROW);
    let hovered = matches!(hover, Some(Hit::Row(j) | Hit::RowCheck(j)) if j == i)
        || v.motion.focus == Some(Focus::Row(i));
    let selected = r.sel == Some(i);
    if selected || row.checked {
        fill_rr(dc, rect, 8.0, with_alpha(t.chip, a));
    } else if hovered {
        fill_rr(dc, rect, 8.0, with_alpha(hover_fill(t), a));
    }
    if selected && v.kb {
        focus_ring(dc, t, rect, 8.0);
    }
    let cy = top + ROW / 2.0;
    checkbox(g, dc, t, LIST_L + 6.0, cy, row.checked, a);
    square_chip(
        g,
        dc,
        t,
        LIST_L + 32.0,
        cy,
        if row.dir { Icon::Folder } else { Icon::File },
        a,
    );
    let (bar_l, pct_r, size_r) = row_cols();
    right_text(g, dc, &row.pct, &f.small, pct_r, top, ROW, &muted);
    right_text(g, dc, &row.size, &f.title, size_r, top, ROW, &ink);
    mini_bar(dc, t, bar_l, cy, MINI_W, row.bar, a);
    let nx = LIST_L + 66.0;
    let room = bar_l - 10.0 - nx;
    let nw = g.text_width(&row.name, &f.title);
    let extra = if row.counting {
        Some("counting")
    } else {
        row.tag
    };
    match extra {
        Some(tag) if nw + 6.0 + g.text_width(tag, &f.body) < room => {
            text(dc, &row.name, &f.title, nx, top, nw + 2.0, ROW, &ink);
            text(dc, tag, &f.body, nx + nw + 4.0, top, 80.0, ROW, &muted);
        }
        _ => clipped_text(g, dc, &row.name, &f.title, nx, top, room, ROW, &ink),
    }
}

/// The tail row: "6 more items", muted, no checkbox.
fn draw_tail_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    shown: usize,
    k: usize,
) {
    let rest = &r.rows[shown..];
    if rest.is_empty() {
        return;
    }
    let (dy, a) = ripple(&v.motion, k);
    let top = list_top() + k as f32 * ROW + dy;
    let bytes: u64 = rest.iter().map(|x| x.bytes).sum();
    let pct: f64 = rest.iter().map(|x| x.pct_v).sum();
    let (bar_l, pct_r, size_r) = row_cols();
    let cy = top + ROW / 2.0;
    let muted = brush(dc, with_alpha(t.muted, a));
    square_chip(g, dc, t, LIST_L + 32.0, cy, Icon::More, a);
    let more = if rest.len() == 1 {
        "1 more item".to_string()
    } else {
        format!("{} more items", rest.len())
    };
    text(dc, &more, &f.body, LIST_L + 66.0, top, 200.0, ROW, &muted);
    right_text(g, dc, &pct_label(pct), &f.small, pct_r, top, ROW, &muted);
    right_text(
        g,
        dc,
        &size_label(bytes),
        &f.title,
        size_r,
        top,
        ROW,
        &brush(dc, with_alpha(t.text, a)),
    );
    let top_b = r.rows.first().map_or(1, |x| x.bytes.max(1));
    mini_bar(dc, t, bar_l, cy, MINI_W, bytes as f32 / top_b as f32, a);
}

/// A skeleton row while the scan has not reached it: bones with a shimmer passing over them.
fn draw_skeleton_row(dc: &ID2D1DeviceContext, t: &Theme, v: &View, k: usize, seed: usize) {
    let top = list_top() + k as f32 * ROW;
    let cy = top + ROW / 2.0;
    let bone = |x0: f32, x1: f32| {
        let mid = (x0 + x1) / 2.0;
        let lift = v.motion.shimmer.map_or(0.0, |s| {
            let sx = LIST_L - 120.0 + s * (LIST_R - LIST_L + 240.0);
            (1.0 - ((mid - sx).abs() / 120.0)).max(0.0)
        });
        fill_rr(
            dc,
            (x0, cy - 4.0, x1, cy + 4.0),
            4.0,
            with_alpha(t.text, 0.05 + 0.04 * lift),
        );
    };
    fill_rr(
        dc,
        (LIST_L + 6.0, cy - 8.0, LIST_L + 22.0, cy + 8.0),
        5.0,
        t.chip,
    );
    fill_rr(
        dc,
        (LIST_L + 32.0, cy - 12.0, LIST_L + 56.0, cy + 12.0),
        7.0,
        t.chip,
    );
    let (bar_l, pct_r, size_r) = row_cols();
    let nx = LIST_L + 66.0;
    let name_w = (bar_l - 10.0 - nx) * [0.62, 0.48, 0.7, 0.4, 0.55, 0.34][seed % 6];
    bone(nx, nx + name_w);
    bone(bar_l, bar_l + MINI_W);
    bone(pct_r - 22.0, pct_r);
    bone(size_r - 40.0, size_r);
}

/// Where a tile is drawn this frame: on its way from its old place during a reflow.
fn tile_at(v: &View, i: usize, rect: Rect) -> Rect {
    match v.motion.from.get(i).copied().flatten() {
        Some(a) if v.motion.reflow < 1.0 => {
            let e = v.motion.reflow;
            (
                a.0 + (rect.0 - a.0) * e,
                a.1 + (rect.1 - a.1) * e,
                a.2 + (rect.2 - a.2) * e,
                a.3 + (rect.3 - a.3) * e,
            )
        }
        _ => rect,
    }
}

fn draw_treemap(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    map: Rect,
) {
    let biggest = r.rows.first().map_or(0, |x| x.bytes);
    if r.tiles.is_empty() {
        fill_rr(dc, map, 6.0, t.raised);
        center_text(
            g,
            dc,
            if r.scanning {
                "Counting…"
            } else {
                "Nothing in here takes room"
            },
            &f.small,
            (map.0 + map.2) / 2.0,
            (map.1 + map.3) / 2.0 - 8.0,
            16.0,
            &brush(dc, t.muted),
        );
        return;
    }
    let lit = |row: Option<usize>| -> f32 {
        match v.motion.focus {
            Some(Focus::Row(j)) if row != Some(j) => 1.0 - 0.7 * v.motion.dim,
            Some(Focus::Seg(_)) => 1.0 - 0.7 * v.motion.dim,
            _ => 1.0,
        }
    };
    for (ti, tile) in r.tiles.iter().enumerate() {
        let at = tile_at(v, ti, tile.rect);
        let rc = (at.0 + 1.0, at.1 + 1.0, at.2 - 1.0, at.3 - 1.0);
        let (w, h) = (rc.2 - rc.0, rc.3 - rc.1);
        if w < 1.0 || h < 1.0 {
            continue;
        }
        let rad = 6.0f32.min(w / 2.0).min(h / 2.0);
        if tile.skeleton {
            // Space not counted yet: a dotted field, edged with a hairline.
            let (_, edge) = wash(t);
            edge_rr(dc, rc, rad, edge, 1.0);
            unsafe {
                dc.PushAxisAlignedClip(
                    &D2D_RECT_F {
                        left: rc.0 + 2.0,
                        top: rc.1 + 2.0,
                        right: rc.2 - 2.0,
                        bottom: rc.3 - 2.0,
                    },
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
                let b = brush(dc, t.track);
                let st = dotted_stroke(g);
                let mut y = rc.1 + 4.0;
                while y < rc.3 {
                    dc.DrawLine(pt(rc.0 + 3.0, y), pt(rc.2, y), &b, 2.0, &st);
                    y += 7.5;
                }
                dc.PopAxisAlignedClip();
            }
            continue;
        }
        let bytes = match tile.row {
            Some(i) => r.rows[i].bytes,
            None => tile.more_bytes,
        };
        let a = lit(tile.row);
        let fill_a = crate::disk_model::tile_alpha(bytes, biggest) * if t.dark { 1.4 } else { 1.0 };
        fill_rr(dc, rc, rad, with_alpha(t.text, fill_a * a));
        let ink = brush(dc, with_alpha(t.text, a));
        for (ir, name, ib) in &tile.inner {
            let ir = tile_at_inner(at, tile.rect, *ir);
            let ir = (ir.0 + 1.0, ir.1 + 1.0, ir.2 - 1.0, ir.3 - 1.0);
            if ir.2 - ir.0 < 2.0 || ir.3 - ir.1 < 2.0 {
                continue;
            }
            let inner_fill = if t.dark {
                rgba(0xFFFFFF, 0.05)
            } else {
                rgba(0xFFFFFF, 0.45)
            };
            fill_rr(dc, ir, 4.0, with_alpha(inner_fill, a));
            edge_rr(dc, ir, 4.0, with_alpha(t.hair, a), 1.0);
            label_tile(g, dc, f, ir, name, &size_label(*ib), &ink);
        }
        let (name, size) = match tile.row {
            Some(i) => (r.rows[i].name.clone(), r.rows[i].size.clone()),
            None => (
                format!("{} smaller", tile.more),
                size_label(tile.more_bytes),
            ),
        };
        label_tile(g, dc, f, rc, &name, &size, &ink);
        let ring = tile.row.is_some()
            && (tile.row == r.sel
                || v.motion
                    .focus
                    .is_some_and(|fo| fo == Focus::Row(tile.row.unwrap_or(usize::MAX))));
        if ring {
            edge_rr(dc, rc, rad, t.text, 2.0);
        }
    }
}

/// Inner tiles move with their parent during a reflow.
fn tile_at_inner(now: Rect, target: Rect, inner: Rect) -> Rect {
    let sx = (now.2 - now.0) / (target.2 - target.0).max(1.0);
    let sy = (now.3 - now.1) / (target.3 - target.1).max(1.0);
    (
        now.0 + (inner.0 - target.0) * sx,
        now.1 + (inner.1 - target.1) * sy,
        now.0 + (inner.2 - target.0) * sx,
        now.1 + (inner.3 - target.1) * sy,
    )
}

/// Name and size at ≥ 60 × 32, the name alone at ≥ 48 × 18, nothing smaller.
pub fn tile_labels(w: f32, h: f32) -> (bool, bool) {
    if w >= 60.0 && h >= 32.0 {
        (true, true)
    } else if w >= 48.0 && h >= 18.0 {
        (true, false)
    } else {
        (false, false)
    }
}

fn label_tile(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    f: &Fonts,
    rc: Rect,
    name: &str,
    size: &str,
    ink: &ID2D1SolidColorBrush,
) {
    let (w, h) = (rc.2 - rc.0, rc.3 - rc.1);
    let (show_name, show_size) = tile_labels(w, h);
    let ty = if h >= 32.0 {
        rc.1 + 8.0
    } else {
        rc.1 + (h - 16.0) / 2.0
    };
    if show_name {
        clipped_text(
            g,
            dc,
            name,
            &f.small_medium,
            rc.0 + 8.0,
            ty,
            w - 14.0,
            16.0,
            ink,
        );
    }
    if show_size && !size.is_empty() {
        clipped_text(
            g,
            dc,
            size,
            &f.small,
            rc.0 + 8.0,
            ty + 16.0,
            w - 14.0,
            16.0,
            ink,
        );
    }
}

/// The label card beside the pointer on a tile: radius 8, hairline, raised fill, 12 px to the
/// right of the pointer, kept inside the map.
fn draw_label_card(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    map: Rect,
    hover: Option<Hit>,
) {
    let (Some(Hit::Tile(i)), Some((mx, my))) = (hover, v.motion.mouse) else {
        return;
    };
    let Some(row) = r.rows.get(i) else { return };
    let my = my - TOP;
    let nw = g.text_width(&row.name, &f.small_medium).min(200.0);
    let sw = g.text_width(&row.size, &f.small);
    let (w, h) = (nw + 8.0 + sw + 20.0, 28.0);
    let x = (mx + 12.0).min(map.2 - w).max(map.0);
    let y = (my - h / 2.0).min(map.3 - h).max(map.1);
    let rc = (x, y, x + w, y + h);
    fill_rr(dc, rc, 8.0, t.panel);
    fill_rr(dc, rc, 8.0, t.raised);
    edge_rr(dc, rc, 8.0, t.hair, 1.0);
    clipped_text(
        g,
        dc,
        &row.name,
        &f.small_medium,
        x + 10.0,
        y,
        nw,
        h,
        &brush(dc, t.text),
    );
    text(
        dc,
        &row.size,
        &f.small,
        x + 10.0 + nw + 8.0,
        y,
        sw + 2.0,
        h,
        &brush(dc, t.muted),
    );
}

/// The 44 px strip under the map: name and path on the left, share and size with Open in
/// Explorer on the right.
fn draw_strip(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    d: &Detail,
    b: f32,
    hover: Option<Hit>,
) {
    let top = b - STRIP_H;
    hairline(dc, t, 0.0, W, top);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let open = strip_open_rect(g, f, b);
    quiet_button(
        g,
        dc,
        t,
        f,
        open,
        "Open in Explorer",
        Some(Icon::External),
        hover == Some(Hit::StripOpen),
        true,
    );
    let sw = g.text_width(&d.size, &f.title);
    let size_r = open.0 - 12.0;
    right_text(g, dc, &d.size, &f.title, size_r, top, STRIP_H, &ink);
    let share = format!("{} · ", d.pct);
    let pw = g.text_width(&share, &f.body);
    text(
        dc,
        &share,
        &f.body,
        size_r - sw - pw,
        top,
        pw + 2.0,
        STRIP_H,
        &muted,
    );
    let nw = g.text_width(&d.name, &f.title).min(220.0);
    clipped_text(g, dc, &d.name, &f.title, X0, top, nw, STRIP_H, &ink);
    let px = X0 + nw + 10.0;
    let room = size_r - sw - pw - 16.0 - px;
    let path = cut_path(g, &d.path, room, &f.small);
    text(
        dc,
        &path,
        &f.small,
        px,
        top,
        room.max(0.0) + 2.0,
        STRIP_H,
        &muted,
    );
}

/// The veil over a scan in progress, with the centred pill naming where it is.
fn draw_veil(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    live: &LiveView,
    v: &View,
    b: f32,
) {
    let a = v.motion.veil.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let (top, bottom) = (BODY_Y - 6.0, b - STRIP_H);
    fill_rr(dc, (0.0, top, W, bottom), 0.0, with_alpha(t.panel, 0.6 * a));
    let path_room = 360.0;
    let path = middle_cut(&live.path, path_room, |s| g.text_width(s, &f.body));
    let lw = g.text_width(&live.label, &f.title);
    let pw = g.text_width(&path, &f.body);
    let w = 6.0 + 28.0 + 10.0 + lw + 10.0 + pw + 16.0;
    let cy = (top + bottom) / 2.0;
    let x = (W - w) / 2.0;
    let rc = (x, cy - 20.0, x + w, cy + 20.0);
    unsafe {
        // A soft shadow tucked under the pill.
        for (grow, al) in [(6.0, 0.025), (3.0, 0.035)] {
            dc.FillRoundedRectangle(
                &rrect(
                    rc.0 - grow,
                    rc.1 - grow + 3.0,
                    rc.2 + grow,
                    rc.3 + grow + 3.0,
                    20.0 + grow,
                ),
                &brush(dc, rgba(0x000000, al * a * if t.dark { 3.0 } else { 1.0 })),
            );
        }
    }
    fill_rr(dc, rc, 20.0, with_alpha(t.panel, a));
    edge_rr(dc, rc, 20.0, with_alpha(t.hair, a), 1.0);
    let (ccx, ccy) = (x + 6.0 + 14.0, cy);
    unsafe {
        dc.FillEllipse(&ellipse(ccx, ccy, 14.0), &brush(dc, with_alpha(t.chip, a)));
        dc.DrawEllipse(
            &ellipse(ccx, ccy, 12.5),
            &brush(dc, with_alpha(t.text, 0.1 * a)),
            1.5,
            None,
        );
    }
    let ink = brush(dc, with_alpha(t.text, a));
    let (from, to) = match (live.frac, v.motion.shimmer) {
        (Some(p), _) => (0.0, 360.0 * p.clamp(0.0, 1.0)),
        (None, Some(s)) => (s * 360.0, s * 360.0 + 100.0),
        (None, None) => (0.0, 100.0),
    };
    draw_arc(g, dc, ccx, ccy, 12.5, from, to, &ink, 2.0, &round_stroke(g));
    draw_icon(g, dc, Icon::Search, ccx, ccy, 13.0, &ink);
    let tx = x + 6.0 + 28.0 + 10.0;
    text(dc, &live.label, &f.title, tx, rc.1, lw + 2.0, 40.0, &ink);
    text(
        dc,
        &path,
        &f.body,
        tx + lw + 10.0,
        rc.1,
        pw + 2.0,
        40.0,
        &brush(dc, with_alpha(t.muted, a)),
    );
}

fn draw_file_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    row: &FileRow,
    top: f32,
    hovered: bool,
    focused: bool,
    a: f32,
) {
    let (x0, x1) = (X0 - 8.0, X1 + 8.0);
    let ink = brush(dc, with_alpha(t.text, a));
    let muted = brush(dc, with_alpha(t.muted, a));
    if row.checked {
        fill_rr(dc, (x0, top, x1, top + ROW), 8.0, with_alpha(t.chip, a));
    } else if hovered {
        fill_rr(
            dc,
            (x0, top, x1, top + ROW),
            8.0,
            with_alpha(hover_fill(t), a),
        );
    }
    if focused {
        focus_ring(dc, t, (x0, top, x1, top + ROW), 8.0);
    }
    let cy = top + ROW / 2.0;
    checkbox(g, dc, t, x0 + 8.0, cy, row.checked, a);
    square_chip(g, dc, t, x0 + 34.0, cy, row.icon, a);
    right_text(g, dc, &row.size, &f.title, x1 - 8.0, top, ROW, &ink);
    let month_r = x1 - 8.0 - SIZE_W - 10.0;
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
    let nx = x0 + 68.0;
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

fn draw_results(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    r: &ResultsView,
    v: &View,
    hover: Option<Hit>,
) {
    let muted = brush(dc, t.muted);
    let b = body_h(g, f, v);
    draw_head(g, dc, t, f, r, v, hover);
    draw_toolbar(g, dc, t, f, r, hover);
    match r.tab {
        Tab::Folders => {
            let map = map_at(b);
            draw_treemap(g, dc, t, f, r, v, map);
            // Column header.
            let (_, pct_r, size_r) = row_cols();
            text(
                dc,
                "Name",
                &f.small,
                LIST_L + 66.0,
                BODY_Y,
                80.0,
                16.0,
                &muted,
            );
            right_text(g, dc, "%", &f.small, pct_r, BODY_Y, 16.0, &muted);
            right_text(g, dc, "Size", &f.small, size_r, BODY_Y, 16.0, &muted);
            let n = folder_rows(b);
            let shown = r.rows.len().min(r.first + n);
            for (k, i) in (r.first..shown).enumerate() {
                draw_folder_row(g, dc, t, f, r, v, i, k, hover);
            }
            if r.scanning && r.rows.len() <= shown {
                // Skeleton rows hold the places the scan has not filled yet.
                for k in (shown - r.first)..=n {
                    draw_skeleton_row(dc, t, v, k, k);
                }
            } else {
                if r.rows.is_empty() {
                    text(
                        dc,
                        "This folder is empty",
                        &f.body,
                        LIST_L + 8.0,
                        list_top(),
                        260.0,
                        ROW,
                        &muted,
                    );
                }
                draw_tail_row(g, dc, t, f, r, v, shown, shown - r.first);
            }
            match (&r.live, r.scanning) {
                (Some(live), true) => {
                    let top = b - STRIP_H;
                    hairline(dc, t, 0.0, W, top);
                    text(dc, &live.foot, &f.body, X0, top, 400.0, STRIP_H, &muted);
                    right_text(
                        g,
                        dc,
                        "Results fill in as each folder finishes",
                        &f.body,
                        X1,
                        top,
                        STRIP_H,
                        &muted,
                    );
                }
                _ if r.selected.is_some() => {
                    draw_sel_bar(g, dc, t, f, v, b, &r.selected, r.can_recycle, false, hover)
                }
                _ => {
                    if let Some(d) = &r.detail {
                        draw_strip(g, dc, t, f, d, b, hover);
                    }
                }
            }
            match &r.live {
                // The veil stays over the results until it has faded.
                Some(live) => draw_veil(g, dc, t, f, live, v, b),
                None => draw_label_card(g, dc, t, f, r, v, map, hover),
            }
        }
        Tab::Files => {
            text(
                dc,
                &r.files_count,
                &f.small,
                X0,
                TOOL_Y + 32.0 + 2.0,
                300.0,
                12.0,
                &muted,
            );
            if r.files.is_empty() {
                text(
                    dc,
                    "No files in here",
                    &f.body,
                    X0,
                    BODY_Y,
                    260.0,
                    ROW,
                    &muted,
                );
            }
            let n = files_rows(b);
            let shown = r.files.len().min(r.files_first + n);
            for (k, i) in (r.files_first..shown).enumerate() {
                let (dy, a) = ripple(&v.motion, k);
                let top = BODY_Y + k as f32 * FILE_ROW + dy;
                draw_file_row(
                    g,
                    dc,
                    t,
                    f,
                    &r.files[i],
                    top,
                    hover == Some(Hit::File(i)),
                    v.kb && v.cursor == Some(i),
                    a,
                );
            }
            draw_scroll_thumb(
                dc,
                t,
                W - 4.0,
                BODY_Y,
                n as f32 * FILE_ROW,
                r.files.len(),
                n,
                r.files_first,
            );
            draw_sel_bar(g, dc, t, f, v, b, &r.selected, r.can_recycle, true, hover);
        }
        Tab::Types => {
            draw_type_bar(dc, t, r, X0, BODY_Y + 4.0, X1);
            let top_b = r.types.iter().map(|x| x.bytes).max().unwrap_or(1).max(1);
            for (i, ty) in r.types.iter().enumerate() {
                let (dy, a) = ripple(&v.motion, i);
                let top = BODY_Y + 28.0 + i as f32 * FILE_ROW + dy;
                let sw = (X0 + 8.0, top + 13.0, X0 + 18.0, top + 23.0);
                fill_rr(dc, sw, 3.0, with_alpha(grey(t, ty.grey), a));
                if ty.grey == 5 {
                    edge_rr(dc, sw, 3.0, with_alpha(grey_edge(t), a), 1.0);
                }
                let ink_a = brush(dc, with_alpha(t.text, a));
                text(dc, ty.label, &f.title, X0 + 30.0, top, 200.0, ROW, &ink_a);
                text(
                    dc,
                    &ty.files,
                    &f.body,
                    X0 + 200.0,
                    top,
                    160.0,
                    ROW,
                    &brush(dc, with_alpha(t.muted, a)),
                );
                mini_bar(
                    dc,
                    t,
                    X0 + 380.0,
                    top + ROW / 2.0,
                    200.0,
                    ty.bytes as f32 / top_b as f32,
                    a,
                );
                right_text(g, dc, &ty.size, &f.title, X1 - 8.0, top, ROW, &ink_a);
            }
        }
    }
}

fn draw_type_bar(dc: &ID2D1DeviceContext, t: &Theme, r: &ResultsView, x0: f32, y: f32, x1: f32) {
    let total: u64 = r.types.iter().map(|x| x.bytes).sum();
    fill_rr(dc, (x0, y, x1, y + 10.0), 3.0, t.track);
    if total == 0 {
        return;
    }
    let mut x = x0;
    for ty in &r.types {
        let w = (x1 - x0) * ty.bytes as f32 / total as f32;
        if w >= 1.0 {
            let rc = (x, y, (x + w - 2.0).max(x + 1.0), y + 10.0);
            fill_rr(dc, rc, 2.0, grey(t, ty.grey));
            if ty.grey == 5 {
                edge_rr(dc, rc, 2.0, grey_edge(t), 1.0);
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
    fill_rr(dc, (x - 4.0, ty, x, ty + th), 2.0, with_alpha(t.text, 0.18));
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
    let b = body_h(g, f, v);
    // Back sits on the left: the band's close button is the only cross.
    let back = (X0, 14.0, X0 + 28.0, 42.0);
    fill_rr(
        dc,
        back,
        14.0,
        if hover == Some(Hit::Back) {
            t.button_hover
        } else {
            t.chip
        },
    );
    draw_icon(g, dc, Icon::ArrowLeft, X0 + 14.0, 28.0, 14.0, &ink);
    let tw = g.text_width("Safe to clean", &f.panel_title);
    text(
        dc,
        "Safe to clean",
        &f.panel_title,
        X0 + 38.0,
        14.0,
        tw + 2.0,
        28.0,
        &ink,
    );
    let base = 14.0 + baseline_in(g, "Safe to clean", &f.panel_title, 28.0);
    let tx = clean_header_total_x(g, f, c);
    clipped_text(
        g,
        dc,
        &c.place,
        &f.body,
        X0 + 38.0 + tw + 8.0,
        base - baseline_in(g, &c.place, &f.body, 18.0),
        tx - 16.0 - (X0 + 38.0 + tw + 8.0),
        18.0,
        &muted,
    );
    let totw = g.text_width(&c.total, &f.heading);
    let ty = base - baseline_in(g, &c.total, &f.heading, 24.0);
    text(dc, &c.total, &f.heading, tx, ty, totw + 2.0, 24.0, &ink);
    text(
        dc,
        " to review",
        &f.body,
        tx + totw,
        base - baseline_in(g, " to review", &f.body, 18.0),
        100.0,
        18.0,
        &muted,
    );
    // Filter chips.
    for (i, (fl, r)) in filter_rects(g, f, c).into_iter().enumerate() {
        let on = fl == c.filter;
        let label = ["All", "Safe", "Check first"][i];
        let lw = g.text_width(label, &f.small_medium);
        if on {
            fill_rr(dc, r, 14.0, t.text);
        } else {
            if hover == Some(Hit::Filter(fl)) {
                fill_rr(dc, r, 14.0, hover_fill(t));
            }
            edge_rr(dc, r, 14.0, t.hair, 1.0);
        }
        let (lc, cc) = if on {
            (
                t.panel,
                if t.dark {
                    rgba(0x6B6B6B, 1.0)
                } else {
                    rgba(0xA3A3A3, 1.0)
                },
            )
        } else {
            (t.text, t.muted)
        };
        text(
            dc,
            label,
            &f.small_medium,
            r.0 + 12.0,
            r.1,
            lw + 2.0,
            28.0,
            &brush(dc, lc),
        );
        text(
            dc,
            &c.counts[i].to_string(),
            &f.small,
            r.0 + 12.0 + lw + 6.0,
            r.1,
            40.0,
            28.0,
            &brush(dc, cc),
        );
    }
    // The list: groups, rows, then what is not counted above.
    let mut row_pos = 0usize;
    for e in clean_entries(c).into_iter().take(c.first) {
        if matches!(e, Entry::Row(..)) {
            row_pos += 1;
        }
    }
    for (k, (e, top)) in clean_visible(c, b).into_iter().enumerate() {
        let (dy, a) = ripple(&v.motion, k);
        let top = top + dy;
        let ink_a = brush(dc, with_alpha(t.text, a));
        let muted_a = brush(dc, with_alpha(t.muted, a));
        match e {
            Entry::Group(gi) => {
                let gr = &c.groups[gi];
                let y = top + entry_h(e) - 20.0;
                text(dc, &gr.title, &f.small, X0, y, 300.0, 16.0, &muted_a);
                right_text(g, dc, &gr.sub, &f.small, X1, y, 16.0, &muted_a);
            }
            Entry::Row(gi, k2) => {
                let row = &c.groups[gi].rows[k2];
                let rect = (X0 - 8.0, top, X1 + 8.0, top + CLEAN_ROW);
                let hovered =
                    matches!(hover, Some(Hit::Clean(j) | Hit::CleanCheck(j)) if j == row.index);
                if row.checked {
                    fill_rr(dc, rect, 8.0, with_alpha(t.chip, a));
                } else if hovered {
                    fill_rr(dc, rect, 8.0, with_alpha(hover_fill(t), a));
                }
                if v.kb && v.cursor == Some(row_pos) {
                    focus_ring(dc, t, rect, 8.0);
                }
                row_pos += 1;
                let cy = top + CLEAN_ROW / 2.0;
                checkbox(g, dc, t, X0, cy, row.checked, a);
                chip(g, dc, t, X0 + 28.0 + 16.0, cy, 16.0, row.icon, false);
                right_text(g, dc, &row.size, &f.title, X1, top, CLEAN_ROW, &ink_a);
                let tag_r = X1 - SIZE_W - 12.0;
                tag(g, dc, t, f, row.check_first, tag_r, cy);
                let tx = X0 + 72.0;
                let room = tag_r - 112.0 - 12.0 - tx;
                clipped_text(
                    g,
                    dc,
                    &row.title,
                    &f.title,
                    tx,
                    top + 3.0,
                    room,
                    18.0,
                    &ink_a,
                );
                clipped_text(
                    g,
                    dc,
                    &row.body,
                    &f.small,
                    tx,
                    top + 21.0,
                    room,
                    16.0,
                    &muted_a,
                );
            }
            Entry::Nothing => {
                let rc = (X0, top + 4.0, X1, top + 72.0);
                fill_rr(dc, rc, 12.0, t.raised);
                chip(g, dc, t, X0 + 26.0, top + 28.0, 13.0, Icon::Tick, false);
                text(
                    dc,
                    c.nothing.as_deref().unwrap_or_default(),
                    &f.title,
                    X0 + 48.0,
                    top + 14.0,
                    400.0,
                    18.0,
                    &ink_a,
                );
                text(
                    dc,
                    "Temp files and caches are already small. Big folders are still in Results.",
                    &f.small,
                    X0 + 48.0,
                    top + 34.0,
                    X1 - X0 - 60.0,
                    16.0,
                    &muted_a,
                );
            }
            Entry::Extras => {
                hairline(dc, t, X0, X1, top + 6.0);
                text(
                    dc,
                    "Not counted above",
                    &f.small,
                    X0,
                    top + 12.0,
                    300.0,
                    16.0,
                    &muted_a,
                );
            }
            Entry::Bin | Entry::System => {
                let bin = e == Entry::Bin;
                let cy = top + 26.0;
                chip(
                    g,
                    dc,
                    t,
                    X0 + 28.0 + 16.0,
                    cy,
                    16.0,
                    if bin { Icon::Trash } else { Icon::Sliders },
                    false,
                );
                let (title, body, label, hit) = if bin {
                    (
                        "Recycle Bin",
                        "Already deleted. Emptying it is permanent, so that stays your call.",
                        "Open",
                        Hit::BinOpen,
                    )
                } else {
                    (
                        "System files",
                        "Update leftovers and restore points. Windows manages these itself.",
                        "Settings",
                        Hit::Settings,
                    )
                };
                let br = extra_button(g, f, label, top);
                quiet_button(g, dc, t, f, br, label, None, hover == Some(hit), true);
                let tx = X0 + 72.0;
                let mut room = br.0 - 12.0 - tx;
                if bin && !c.bin.is_empty() {
                    let sw = g.text_width(&c.bin, &f.title);
                    right_text(
                        g,
                        dc,
                        &c.bin,
                        &f.title,
                        br.0 - 12.0,
                        top + 8.0,
                        18.0,
                        &ink_a,
                    );
                    room -= sw + 12.0;
                }
                clipped_text(g, dc, title, &f.title, tx, top + 8.0, room, 18.0, &ink_a);
                clipped_text(
                    g,
                    dc,
                    body,
                    &f.small,
                    tx,
                    top + 26.0,
                    br.0 - 12.0 - tx,
                    16.0,
                    &muted_a,
                );
            }
        }
    }
    let n_entries = clean_entries(c).len();
    let shown = clean_visible(c, b).len();
    draw_scroll_thumb(
        dc,
        t,
        W - 4.0,
        CLEAN_TOP,
        bar_top(b) - 8.0 - CLEAN_TOP,
        n_entries,
        shown,
        c.first,
    );
    draw_sel_bar(g, dc, t, f, v, b, &c.selected, true, false, hover);
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
    let b = body_h(g, f, v);
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
        X1 - X0 - 76.0,
        28.0,
        &ink,
    );
    icon_button(
        g,
        dc,
        t,
        (X1 - 28.0, 16.0, X1, 44.0),
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
        X1,
        52.0,
        16.0,
        if hover == Some(Hit::SelectAll) {
            &ink
        } else {
            &muted
        },
    );
    if r.len == 0 {
        text(
            dc,
            "Nothing left to review here",
            &f.body,
            X0,
            REVIEW_TOP,
            260.0,
            ROW,
            &muted,
        );
    }
    let n = review_rows(b);
    let shown = r.len.min(r.first + n);
    for (k, i) in (r.first..shown).enumerate() {
        let (dy, a) = ripple(&v.motion, k);
        let top = REVIEW_TOP + k as f32 * FILE_ROW + dy;
        let Some(row) = i.checked_sub(r.rows_from).and_then(|j| r.rows.get(j)) else {
            continue;
        };
        draw_file_row(
            g,
            dc,
            t,
            f,
            row,
            top,
            hover == Some(Hit::File(i)),
            v.kb && v.cursor == Some(i),
            a,
        );
    }
    draw_scroll_thumb(
        dc,
        t,
        W - 4.0,
        REVIEW_TOP,
        n as f32 * FILE_ROW,
        r.len,
        n,
        r.first,
    );
    draw_sel_bar(g, dc, t, f, v, b, &r.selected, r.can_recycle, true, hover);
}

fn draw_dupes(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    d: &DupesView,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let b = body_h(g, f, v);
    // The header, as on Safe to clean: back on the left, the total on the right.
    let back = (X0, 14.0, X0 + 28.0, 42.0);
    fill_rr(
        dc,
        back,
        14.0,
        if hover == Some(Hit::Back) {
            t.button_hover
        } else {
            t.chip
        },
    );
    draw_icon(g, dc, Icon::ArrowLeft, X0 + 14.0, 28.0, 14.0, &ink);
    let title = "Duplicates";
    let tw = g.text_width(title, &f.panel_title);
    text(
        dc,
        title,
        &f.panel_title,
        X0 + 38.0,
        14.0,
        tw + 2.0,
        28.0,
        &ink,
    );
    let base = 14.0 + baseline_in(g, title, &f.panel_title, 28.0);
    let lw = g.text_width(d.total_label, &f.body);
    let totw = g.text_width(&d.total, &f.heading);
    let tx = X1 - lw - totw;
    clipped_text(
        g,
        dc,
        &d.place,
        &f.body,
        X0 + 38.0 + tw + 8.0,
        base - baseline_in(g, &d.place, &f.body, 18.0),
        tx - 16.0 - (X0 + 38.0 + tw + 8.0),
        18.0,
        &muted,
    );
    let ty = base - baseline_in(g, &d.total, &f.heading, 24.0);
    text(dc, &d.total, &f.heading, tx, ty, totw + 2.0, 24.0, &ink);
    text(
        dc,
        d.total_label,
        &f.body,
        tx + totw,
        base - baseline_in(g, d.total_label, &f.body, 18.0),
        lw + 2.0,
        18.0,
        &muted,
    );
    // The line under it: what the check found, and select all in the filter-chip style.
    text(dc, &d.sub, &f.small, X0, 62.0, 420.0, 16.0, &muted);
    if let DupesBody::Groups(_) = &d.body {
        let r = dupes_all_rect(g, f, d);
        if hover == Some(Hit::DupesAll) {
            fill_rr(dc, r, 14.0, hover_fill(t));
        }
        edge_rr(dc, r, 14.0, t.hair, 1.0);
        center_text(
            g,
            dc,
            dupes_all_label(d),
            &f.small_medium,
            (r.0 + r.2) / 2.0,
            r.1,
            28.0,
            &ink,
        );
    }
    // Before, during and after a check with nothing to list: one card.
    if let Some((r, tx, tw)) = dupes_block(g, f, d) {
        let (dy, a) = ripple(&v.motion, 0);
        let r = (r.0, r.1 + dy, r.2, r.3 + dy);
        fill_rr(dc, r, 12.0, with_alpha(t.raised, a));
        let ink_a = brush(dc, with_alpha(t.text, a));
        let muted_a = brush(dc, with_alpha(t.muted, a));
        let y0 = r.1 + 16.0;
        let y1 = y0 + 24.0;
        let (icon, title) = match &d.body {
            DupesBody::Offer { title, .. } => (Icon::Layers, title.as_str()),
            DupesBody::Running { stage, .. } => (Icon::Search, *stage),
            DupesBody::Empty { title, .. } => (Icon::Tick, title.as_str()),
            DupesBody::Groups(_) => (Icon::Layers, ""),
        };
        chip(g, dc, t, X0 + 28.0, y0 + 9.0, 14.0, icon, false);
        let button = dupes_block_button(g, f, d);
        let title_w = match &button {
            Some((Hit::DupesStop, br)) => br.0 - 12.0 - tx,
            _ => tw,
        };
        clipped_text(g, dc, title, &f.title, tx, y0, title_w, 18.0, &ink_a);
        match &d.body {
            DupesBody::Offer { body, cost, .. } => {
                let hb = wrap_h(g, body, &f.small_wrap, tw);
                text(dc, body, &f.small_wrap, tx, y1, tw, hb + 2.0, &muted_a);
                let y2 = y1 + hb + 6.0;
                let hc = wrap_h(g, cost, &f.small_wrap, tw);
                text(dc, cost, &f.small_wrap, tx, y2, tw, hc + 2.0, &muted_a);
            }
            DupesBody::Running { line, frac, .. } => {
                track_bar(dc, t, tx, y1 + 4.0, tw, 4.0, *frac);
                text(dc, line, &f.small, tx, y1 + 16.0, tw, 16.0, &muted_a);
            }
            DupesBody::Empty { body, .. } => {
                let hb = wrap_h(g, body, &f.small_wrap, tw);
                text(dc, body, &f.small_wrap, tx, y1, tw, hb + 2.0, &muted_a);
            }
            DupesBody::Groups(_) => {}
        }
        if let Some((h, br)) = button {
            let (label, icon) = if h == Hit::DupesStop {
                ("Stop", Icon::Stop)
            } else {
                ("Check now", Icon::Search)
            };
            let br = (br.0, br.1 + dy, br.2, br.3 + dy);
            quiet_button(g, dc, t, f, br, label, Some(icon), hover == Some(h), true);
        }
    }
    // The groups: a heading per group, then each copy with its keep column and date.
    if let DupesBody::Groups(groups) = &d.body {
        // The keyboard's row number of the first copy shown: copies before `first`.
        let mut row_pos = d.first;
        let mut seen = 0;
        for gr in groups {
            if seen >= d.first {
                break;
            }
            row_pos -= 1;
            seen += 1 + gr.len;
        }
        let visible = dupe_visible(d, b);
        for (k, &(e, top)) in visible.iter().enumerate() {
            let (dy, a) = ripple(&v.motion, k);
            let top = top + dy;
            // At rest (no ripple) the rows share the screen's brushes.
            let (ink_a, muted_a) = if a >= 1.0 {
                (ink.clone(), muted.clone())
            } else {
                (
                    brush(dc, with_alpha(t.text, a)),
                    brush(dc, with_alpha(t.muted, a)),
                )
            };
            match e {
                DupeEntry::Group(gi) => {
                    let gr = &groups[gi];
                    let y = top + DUPE_HEAD - 22.0;
                    let sw = g.text_width(&gr.sub, &f.small);
                    clipped_text(
                        g,
                        dc,
                        &gr.name,
                        &f.title,
                        X0,
                        y,
                        X1 - X0 - sw - 16.0,
                        18.0,
                        &ink_a,
                    );
                    right_text(g, dc, &gr.sub, &f.small, X1, y + 1.0, 16.0, &muted_a);
                }
                DupeEntry::Row(gi, ki) => {
                    let Some(row) = groups[gi].rows.get(ki) else {
                        row_pos += 1;
                        continue;
                    };
                    let rect = (X0 - 8.0, top, X1 + 8.0, top + ROW);
                    let hovered = matches!(hover, Some(Hit::DupeRow(x, y) | Hit::DupeCheck(x, y) | Hit::DupeKeep(x, y)) if (x, y) == (gi, ki));
                    let focused = v.kb && v.cursor == Some(row_pos);
                    row_pos += 1;
                    if row.checked {
                        fill_rr(dc, rect, 8.0, with_alpha(t.chip, a));
                    } else if hovered {
                        fill_rr(dc, rect, 8.0, with_alpha(hover_fill(t), a));
                    }
                    if focused {
                        focus_ring(dc, t, rect, 8.0);
                    }
                    let cy = top + ROW / 2.0;
                    if !row.keep {
                        checkbox(g, dc, t, X0, cy, row.checked, a);
                    }
                    right_text(g, dc, &row.date, &f.small, X1, top, ROW, &muted_a);
                    let kr = keep_rect(top);
                    let tf = tag_font(g, f);
                    if row.keep {
                        edge_rr(dc, kr, 11.0, with_alpha(t.text, 0.4 * a), 1.0);
                        center_text(g, dc, "Keep", &tf, (kr.0 + kr.2) / 2.0, kr.1, 22.0, &ink_a);
                    } else if hovered || focused {
                        let on = hover == Some(Hit::DupeKeep(gi, ki));
                        fill_rr(dc, kr, 11.0, if on { t.button_hover } else { t.chip });
                        center_text(
                            g,
                            dc,
                            "Keep this",
                            &tf,
                            (kr.0 + kr.2) / 2.0,
                            kr.1,
                            22.0,
                            if on { &ink_a } else { &muted_a },
                        );
                    }
                    let px = X0 + 28.0;
                    let room = kr.0 - 12.0 - px;
                    let cut = cut_path(g, &row.path, room, &f.body);
                    text(dc, &cut, &f.body, px, top, room + 2.0, ROW, &ink_a);
                }
            }
        }
        let n_entries = dupe_entry_count(d);
        let shown = visible.len();
        draw_scroll_thumb(
            dc,
            t,
            W - 4.0,
            DUPES_TOP,
            bar_top(b) - 8.0 - DUPES_TOP,
            n_entries,
            shown,
            d.first,
        );
    }
    draw_sel_bar(g, dc, t, f, v, b, &d.selected, d.can_recycle, true, hover);
}

fn draw_cards(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    for (i, ((top, h), c)) in card_tops(g, f, v).into_iter().zip(&v.cards).enumerate() {
        let rc = (X0, top, X1, top + h);
        fill_rr(dc, rc, 12.0, t.panel);
        edge_rr(dc, rc, 12.0, t.hair, 1.0);
        let (cx, cy) = (X0 + 24.0, top + 24.0);
        fill_rr(
            dc,
            (cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0),
            7.0,
            if c.ember {
                with_alpha(t.ember, 0.14)
            } else {
                t.chip
            },
        );
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
            X1 - X0 - 60.0,
            18.0,
            &ink,
        );
        text(
            dc,
            &c.body,
            &f.small_wrap,
            X0 + 48.0,
            top + 34.0,
            X1 - X0 - 64.0,
            h - 34.0,
            &muted,
        );
        if let Some(label) = &c.button {
            let bw = quiet_w(g, f, label, false);
            let x = X0 + 48.0;
            quiet_button(
                g,
                dc,
                t,
                f,
                (x, top + h - 40.0, x + bw, top + h - 12.0),
                label,
                None,
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
    let s = sheet_rect(g, f, v, c);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    unsafe {
        for (grow, a) in [(16.0, 0.05), (8.0, 0.06), (2.0, 0.06)] {
            dc.FillRoundedRectangle(
                &rrect(
                    s.0 - grow * 0.5,
                    s.1 + grow * 0.6,
                    s.2 + grow * 0.5,
                    s.3 + grow,
                    20.0 + grow * 0.5,
                ),
                &brush(dc, rgba(0x000000, a)),
            );
        }
    }
    fill_rr(dc, s, 20.0, t.panel);
    edge_rr(dc, s, 20.0, t.hair, 1.0);
    let (x0, x1) = (s.0 + 20.0, s.2 - 20.0);
    let mut y = s.1 + 20.0;
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
    y += 32.0;
    for gr in &c.groups {
        y += 16.0;
        text(
            dc,
            if gr.check_first {
                "Check first"
            } else {
                "Safe to remove"
            },
            &f.small,
            x0,
            y,
            200.0,
            16.0,
            &muted,
        );
        right_text(g, dc, &gr.count, &f.small, x1, y, 16.0, &muted);
        y += 16.0;
        if gr.check_first {
            y += 6.0;
            bang(dc, t, x0 + 8.0, y + 8.0);
            text(
                dc,
                "If you're not sure about one, cancel and untick it.",
                &f.small,
                x0 + 22.0,
                y,
                x1 - x0 - 22.0,
                16.0,
                &ink,
            );
            y += 16.0;
        }
        y += 8.0;
        let wh = 8.0
            + gr.items.iter().map(|i| sheet_row_h(&i.1)).sum::<f32>()
            + if gr.more.is_some() { 28.0 } else { 0.0 };
        fill_rr(dc, (x0, y, x1, y + wh), 12.0, t.raised);
        let mut ry = y + 4.0;
        for (name, sub, size) in &gr.items {
            let rh = sheet_row_h(sub);
            let sw = g.text_width(size, &f.title);
            right_text(g, dc, size, &f.title, x1 - 14.0, ry, rh, &ink);
            let room = x1 - x0 - 28.0 - sw - 12.0;
            if sub.is_empty() {
                clipped_text(g, dc, name, &f.title, x0 + 14.0, ry, room, rh, &ink);
            } else {
                clipped_text(g, dc, name, &f.title, x0 + 14.0, ry + 6.0, room, 18.0, &ink);
                let p = cut_path(g, sub, room, &f.small);
                text(
                    dc,
                    &p,
                    &f.small,
                    x0 + 14.0,
                    ry + 24.0,
                    room + 2.0,
                    16.0,
                    &muted,
                );
            }
            ry += rh;
        }
        if let Some(more) = &gr.more {
            text(dc, more, &f.small, x0 + 14.0, ry, 200.0, 24.0, &muted);
        }
        y += wh;
    }
    y += 14.0;
    let nh = wrap_h(g, c.note, &f.small_wrap, x1 - x0);
    text(dc, c.note, &f.small_wrap, x0, y, x1 - x0, nh + 2.0, &muted);
    y += nh + 18.0;
    hairline(dc, t, s.0 + 1.0, s.2 - 1.0, y);
    let [(_, cancel), (_, mv)] = sheet_buttons(g, f, s, c);
    let fw = cancel.0 - 12.0 - x0;
    let fh = wrap_h(g, &c.footer, &f.small_wrap, fw);
    text(
        dc,
        &c.footer,
        &f.small_wrap,
        x0,
        (cancel.1 + cancel.3) / 2.0 - fh / 2.0,
        fw,
        fh + 2.0,
        &muted,
    );
    fill_rr(
        dc,
        cancel,
        16.0,
        if hover == Some(Hit::Cancel) {
            t.button_hover
        } else {
            t.chip
        },
    );
    center_text(
        g,
        dc,
        "Cancel",
        &f.title,
        (cancel.0 + cancel.2) / 2.0,
        cancel.1,
        32.0,
        &ink,
    );
    tall_button(
        g,
        dc,
        t,
        f,
        mv,
        &c.action,
        None,
        true,
        hover == Some(Hit::Move),
        true,
    );
    focus_ring(dc, t, if c.on_move { mv } else { cancel }, 16.0);
}

/// The title band: "Disk", the close button, a hairline under them.
fn draw_band(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, hover: Option<Hit>) {
    text(
        dc,
        "Disk",
        &f.title,
        16.0,
        0.0,
        200.0,
        TOP,
        &brush(dc, t.text),
    );
    let cr = close_rect();
    if hover == Some(Hit::Close) {
        fill_rr(dc, cr, 14.0, t.button_hover);
    }
    draw_icon(
        g,
        dc,
        Icon::Close,
        (cr.0 + cr.2) / 2.0,
        (cr.1 + cr.3) / 2.0,
        14.0,
        &brush(dc, t.muted),
    );
    hairline(dc, t, 1.0, W - 1.0, TOP - 1.0);
}

fn shift(y: f32) -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: 0.0,
        M32: y,
    }
}

/// The whole window: its card, the title band, the screen and its cards under the band, then
/// the confirm sheet over a scrim that covers the band too.
pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let (w, h) = size(g, f, v);
    window_bg(dc, t, w, h);
    let under = if v.confirm.is_some() { None } else { hover };
    draw_band(g, dc, t, f, under);
    unsafe { dc.SetTransform(&shift(TOP)) };
    match &v.screen {
        Screen::Start(d, dupes) => draw_start(g, dc, t, f, d, *dupes, v, under),
        Screen::Results(r) => draw_results(g, dc, t, f, r, v, under),
        Screen::Clean(c) => draw_clean(g, dc, t, f, c, v, under),
        Screen::Review(r) => draw_review(g, dc, t, f, r, v, under),
        Screen::Dupes(d) => draw_dupes(g, dc, t, f, d, v, under),
    }
    draw_cards(g, dc, t, f, v, under);
    if let Some(c) = &v.confirm {
        unsafe {
            dc.SetTransform(&shift(0.0));
            dc.FillRoundedRectangle(
                &rrect(0.0, 0.0, w, h, 16.0),
                &brush(dc, rgba(0x000000, if t.dark { 0.5 } else { 0.32 })),
            );
            dc.SetTransform(&shift(TOP));
        }
        draw_confirm(g, dc, t, f, v, c, hover);
    }
    // The device context is shared with every other window: leave it untransformed.
    unsafe { dc.SetTransform(&shift(0.0)) };
}

/// Which list a wheel over (x, y) scrolls.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scroll {
    Folders,
    Files,
    Review,
    Clean,
    Dupes,
}

/// (x, y) in window coordinates.
pub fn scroll_target(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Scroll> {
    let b = body_h(g, f, v);
    let y = y - TOP;
    match &v.screen {
        Screen::Results(r) if !r.scanning => match r.tab {
            Tab::Folders if inside(&list_at(b), x, y) || inside(&map_at(b), x, y) => {
                Some(Scroll::Folders)
            }
            Tab::Files if (BODY_Y - 6.0..bar_top(b)).contains(&y) => Some(Scroll::Files),
            _ => None,
        },
        Screen::Review(_) => Some(Scroll::Review),
        Screen::Clean(_) if (CLEAN_TOP..bar_top(b)).contains(&y) => Some(Scroll::Clean),
        Screen::Dupes(_) if (DUPES_TOP..bar_top(b)).contains(&y) => Some(Scroll::Dupes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_and_treemap_sit_side_by_side_at_any_height() {
        for b in [results_min(), H - TOP - 120.0, H - TOP, 1000.0] {
            let (map, list) = (map_at(b), list_at(b));
            assert!(map.2 < list.0, "the map stays left of the list");
            assert!(list.2 <= W - X0);
            assert_eq!(
                list.3 - list.1,
                (folder_rows(b) + 1) as f32 * ROW + 4.0,
                "the rows and the tail fill the map's height"
            );
            assert!(map.3 <= bar_top(b), "the map ends above the selection bar");
            assert!(map.3 <= b - STRIP_H, "and above the detail strip");
            assert!(
                BODY_Y + files_rows(b) as f32 * FILE_ROW <= bar_top(b),
                "file rows end above the bar"
            );
            assert!(
                REVIEW_TOP + review_rows(b) as f32 * FILE_ROW + 8.0 <= bar_top(b),
                "review rows too"
            );
        }
        assert_eq!(folder_rows(results_min()), MIN_ROWS);
    }

    #[test]
    fn the_full_window_matches_the_frame() {
        let b = H - TOP;
        assert_eq!(
            folder_rows(b),
            9,
            "nine folders and the tail row, as in frame 02"
        );
        let m = map_at(b);
        assert_eq!(
            (m.2 - m.0, m.3 - m.1),
            (300.0, 384.0),
            "the treemap is 300 × 384"
        );
        assert!(files_rows(b) > 7);
        assert!(review_rows(b) > 9);
        // disk.rs puts words only in these windows; they hold every row the lists can show.
        assert!(FOLDER_WINDOW > folder_rows(b));
        assert!(REVIEW_WINDOW >= review_rows(b));
        assert!(DUPES_WINDOW as f32 * DUPE_HEAD >= bar_top(b) - DUPES_TOP);
    }

    #[test]
    fn cards_take_room_from_the_lists_but_never_below_the_minimum() {
        let full = body_from(H, 0.0, results_min());
        let with_card = body_from(H, 90.0, results_min());
        assert_eq!(full, H - TOP);
        assert!(files_rows(with_card) < files_rows(full));
        assert_eq!(body_from(500.0, 300.0, results_min()), results_min());
    }

    #[test]
    fn number_columns_are_44_and_64_px_and_clear_the_name() {
        let (bar_l, pct_r, size_r) = row_cols();
        assert_eq!(size_r, LIST_R - 6.0);
        assert_eq!(size_r - SIZE_W - 10.0, pct_r);
        assert_eq!(pct_r - PCT_W - 10.0 - MINI_W, bar_l);
        assert!(
            bar_l - 10.0 - (LIST_L + 66.0) > 120.0,
            "the name keeps its room"
        );
    }

    #[test]
    fn sizes_drop_a_trailing_zero_and_keep_base_1024() {
        assert_eq!(size_label(182 << 30), "182 GB");
        assert_eq!(size_label((9.4 * (1u64 << 30) as f64) as u64), "9.4 GB");
        assert_eq!(size_label(640 << 20), "640 MB");
        assert_eq!(pct_label(41.0), "41%");
        assert_eq!(pct_label(30.62), "30.6%");
        assert_eq!(pct_label(0.04), "0%");
    }

    fn segs(bytes: &[u64]) -> Vec<Seg> {
        bytes
            .iter()
            .enumerate()
            .map(|(i, b)| Seg {
                label: format!("s{i}"),
                bytes: *b,
                kind: SegKind::Grey(i),
                row: None,
            })
            .collect()
    }

    #[test]
    fn capacity_segments_fill_the_bar_with_2px_gaps_and_a_4px_floor() {
        let s = segs(&[182, 97, 52, 38, 31, 38, 38, 0]);
        let p = seg_layout(&s);
        assert_eq!(p.first().unwrap().1, X0);
        assert!((p.last().unwrap().2 - X1).abs() < 0.5, "{:?}", p.last());
        for w in p.windows(2) {
            assert!(
                (w[1].1 - w[0].2 - 2.0).abs() < 0.01,
                "2 px between segments"
            );
        }
        assert!(
            p.iter().all(|(_, l, r)| r - l >= 3.99),
            "every segment at least 4 px"
        );
    }

    #[test]
    fn tile_labels_follow_the_size_rules() {
        assert_eq!(tile_labels(60.0, 32.0), (true, true));
        assert_eq!(tile_labels(59.0, 40.0), (true, false));
        assert_eq!(tile_labels(48.0, 18.0), (true, false));
        assert_eq!(tile_labels(47.0, 30.0), (false, false));
    }

    #[test]
    fn paths_are_cut_in_the_middle() {
        let p = r"C:\Users\dhio\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data";
        let cut = middle_cut(p, 40.0, |s| s.chars().count() as f32);
        assert!(cut.starts_with(r"C:\Users"), "{cut}");
        assert!(cut.ends_with("Cache_Data"), "{cut}");
        assert!(cut.contains('…'));
        assert!(cut.chars().count() <= 40);
        assert_eq!(middle_cut("C:\\x", 40.0, |s| s.len() as f32), "C:\\x");
    }

    #[test]
    fn rows_enter_with_the_ripple_and_rest_after_it() {
        let m = Motion {
            ripple: Some(0.0),
            ..Motion::default()
        };
        assert_eq!(
            ripple(&m, 0),
            (6.0, 0.0),
            "the first row starts 6 px low and clear"
        );
        let m = Motion {
            ripple: Some(1000.0),
            ..Motion::default()
        };
        assert_eq!(ripple(&m, 8), (0.0, 1.0));
        assert_eq!(
            ripple(&Motion::default(), 3),
            (0.0, 1.0),
            "no ripple: at rest"
        );
        // The reflow curve: slow start, slow end.
        assert!(motion::cubic_bezier(0.7, 0.0, 0.3, 1.0, 0.2) < 0.1);
        assert!((motion::cubic_bezier(0.7, 0.0, 0.3, 1.0, 0.5) - 0.5).abs() < 0.02);
    }

    #[test]
    fn the_close_button_sits_in_the_band() {
        let c = close_rect();
        assert!(c.3 <= TOP && c.2 <= W && c.1 > 0.0);
    }

    #[test]
    fn the_duplicates_option_sits_between_the_drives_and_the_buttons() {
        for n in 0..5 {
            let o = option_top(n);
            assert!(o >= 52.0 + n as f32 * 60.0, "under the last drive row");
            assert!(start_buttons_y(n) >= o + OPTION_H, "above the buttons");
            assert_eq!(start_h(n), start_buttons_y(n) + 28.0 + 16.0);
        }
    }

    #[test]
    fn a_copy_row_keeps_room_for_its_path_keep_column_and_date() {
        let k = keep_rect(100.0);
        assert!(
            k.2 + 12.0 <= X1 - DATE_W + 0.01,
            "the keep column clears the date"
        );
        assert!(k.0 - (X0 + 28.0) >= 400.0, "the path keeps most of the row");
        assert!(k.1 >= 100.0 && k.3 <= 100.0 + ROW);
    }

    #[test]
    fn duplicate_entries_list_each_heading_then_its_copies() {
        let row = || DupeRowView {
            path: r"C:\a\b.mp4".into(),
            date: "6 Nov 2025".into(),
            keep: false,
            checked: false,
        };
        let group = |n: usize| DupeGroupView {
            len: n,
            name: "b.mp4".into(),
            sub: String::new(),
            rows: (0..n).map(|_| row()).collect(),
        };
        let mut d = DupesView {
            place: String::new(),
            total: String::new(),
            total_label: " to reclaim",
            sub: String::new(),
            body: DupesBody::Groups(vec![group(2), group(3)]),
            first: 0,
            selected: None,
            all: false,
            can_recycle: true,
        };
        use DupeEntry::{Group, Row};
        assert_eq!(
            dupe_entries(&d),
            [
                Group(0),
                Row(0, 0),
                Row(0, 1),
                Group(1),
                Row(1, 0),
                Row(1, 1),
                Row(1, 2)
            ]
        );
        assert_eq!(dupe_entry_count(&d), 7);
        for first in 0..=8 {
            assert_eq!(
                dupe_entries_from(&d, first).collect::<Vec<_>>(),
                dupe_entries(&d).into_iter().skip(first).collect::<Vec<_>>(),
                "skipping whole groups lands on the same entry ({first})"
            );
        }
        let b = H - TOP;
        let shown = dupe_visible(&d, b);
        assert!(
            shown.len() <= DUPES_WINDOW,
            "the window covers every shown entry"
        );
        assert!(shown
            .iter()
            .all(|(e, top)| top + dupe_entry_h(*e) <= bar_top(b)));
        d.body = DupesBody::Running {
            stage: "Comparing whole files…",
            line: String::new(),
            frac: 0.5,
        };
        assert!(dupe_entries(&d).is_empty(), "no list while a check runs");
    }

    #[test]
    fn clean_entries_end_with_what_is_not_counted() {
        let c = CleanView {
            place: String::new(),
            total: String::new(),
            filter: Filter::All,
            counts: [1, 1, 0],
            groups: vec![CleanGroup {
                title: "Suggested".into(),
                sub: String::new(),
                rows: vec![CleanRow {
                    index: 0,
                    icon: Icon::Layers,
                    title: "Windows temp files".into(),
                    body: String::new(),
                    check_first: false,
                    size: String::new(),
                    checked: false,
                    can_review: true,
                }],
            }],
            first: 0,
            bin: String::new(),
            nothing: None,
            selected: None,
        };
        assert_eq!(
            clean_entries(&c),
            [
                Entry::Group(0),
                Entry::Row(0, 0),
                Entry::Extras,
                Entry::Bin,
                Entry::System
            ]
        );
        assert_eq!(clean_visible(&c, H - TOP).len(), 5);
    }
}
