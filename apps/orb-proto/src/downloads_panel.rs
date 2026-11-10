#![allow(clippy::too_many_arguments)] // drawing helpers take a canvas plus a full set of geometry
//! The Downloads panel (frames "v6.2 · 11" and "v6.2 · 12"): the title with its count, the
//! first-run card, the paste field, the rows by section, and the folder footer. Also the small
//! format menu that opens from the offer pill or a waiting row's format chip.

use app_downloads::{ErrorKind, Format, MediaKind, Tone};

use crate::canvas::{clipped_text, ellipse, pt, rgba, rrect, text_width, Canvas, Color, Stroke};
use crate::downloads_model::{menu_rows, Btn, Chip, Field, Row, Section};
use crate::ui::{lip, paint_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 400.0;
pub const MAX_H: f32 = 620.0;
const X0: f32 = 16.0;
const X1: f32 = W - 16.0;
const ROW_H: f32 = 48.0;
const HEAD_H: f32 = 28.0;
/// The hairline and the folder row.
const FOOT: f32 = 8.0 + 36.0 + 4.0;
const EMPTY_H: f32 = 188.0;

/// The engine card above everything while yt-dlp is not there yet.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineCard {
    None,
    Missing {
        waiting: bool,
    },
    Fetching {
        done: u64,
        total: Option<u64>,
        waiting: bool,
    },
    Failed(String),
}

pub struct View {
    pub count: String,
    pub engine: EngineCard,
    pub field: Field,
    pub rows: Vec<Row>,
    pub folder: String,
    pub free: String,
    pub scroll: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Paste,
    Fetch,
    ClearList,
    Row(usize),
    Btn(usize, Btn),
    Folder,
}

type Rect = (f32, f32, f32, f32);

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Head(Section),
    Row(usize),
}

impl Item {
    fn h(self) -> f32 {
        match self {
            Item::Head(_) => HEAD_H,
            Item::Row(_) => ROW_H,
        }
    }
}

fn items(v: &View) -> Vec<Item> {
    let mut out = Vec::new();
    let mut last = None;
    for (i, r) in v.rows.iter().enumerate() {
        if last != Some(r.section) {
            out.push(Item::Head(r.section));
            last = Some(r.section);
        }
        out.push(Item::Row(i));
    }
    out
}

fn card_h(c: &EngineCard) -> f32 {
    match c {
        EngineCard::None => 0.0,
        EngineCard::Fetching { .. } => 12.0 + 18.0 + 10.0 + 36.0 + 12.0,
        EngineCard::Missing { .. } | EngineCard::Failed(_) => {
            12.0 + 18.0 + 6.0 + 32.0 + 10.0 + 28.0 + 12.0
        }
    }
}

fn field_msg_h(f: &Field) -> f32 {
    match f {
        Field::Empty => 0.0,
        Field::Unsupported(_) => 6.0 + 32.0,
        Field::NoLink => 6.0 + 16.0,
    }
}

struct Geo {
    card: f32,
    /// The paste field, or the empty block when there are no rows.
    field: f32,
    list_top: f32,
    list_bottom: f32,
    h: f32,
}

/// No rows: the empty block, unless a refused link needs the field to show it in ember.
fn empty_block(v: &View) -> bool {
    v.rows.is_empty() && !matches!(v.field, Field::Unsupported(_))
}

fn content_h(v: &View) -> f32 {
    items(v).iter().map(|i| i.h()).sum()
}

fn geo(v: &View) -> Geo {
    let card = 46.0;
    let mut y = card;
    if v.engine != EngineCard::None {
        y += card_h(&v.engine) + 10.0;
    }
    let field = y;
    if empty_block(v) {
        let h = (field + EMPTY_H + field_msg_h(&v.field) + 8.0).ceil();
        return Geo {
            card,
            field,
            list_top: h,
            list_bottom: h,
            h,
        };
    }
    y += 40.0 + field_msg_h(&v.field) + 6.0;
    let list_top = y;
    let h = (list_top + content_h(v) + FOOT).min(MAX_H).ceil();
    Geo {
        card,
        field,
        list_top,
        list_bottom: h - FOOT,
        h,
    }
}

pub fn height(v: &View) -> f32 {
    geo(v).h
}

pub fn max_scroll(v: &View) -> f32 {
    let g = geo(v);
    (content_h(v) - (g.list_bottom - g.list_top)).max(0.0)
}

fn item_tops(v: &View) -> Vec<(Item, f32)> {
    let mut y = geo(v).list_top - v.scroll;
    items(v)
        .into_iter()
        .map(|i| {
            let r = (i, y);
            y += i.h();
            r
        })
        .collect()
}

fn btn_w(f: &Fonts, b: Btn, row: &Row) -> f32 {
    match (b, b.label()) {
        (_, Some(l)) => text_width(l, &f.small_medium) + 24.0,
        (Btn::Format, _) => 10.0 + text_width(&row.format_chip, &f.small_medium) + 4.0 + 14.0 + 8.0,
        _ => 28.0,
    }
}

/// The row's buttons, right-aligned, left to right.
fn buttons(f: &Fonts, row: &Row, top: f32) -> Vec<(Btn, Rect)> {
    let cy = top + ROW_H / 2.0;
    let mut x = X1 - 8.0;
    let mut out: Vec<(Btn, Rect)> = row
        .buttons
        .iter()
        .rev()
        .map(|b| {
            let w = btn_w(f, *b, row);
            let r = (x - w, cy - 14.0, x, cy + 14.0);
            x -= w + 4.0;
            (*b, r)
        })
        .collect();
    out.reverse();
    out
}

fn paste_rect(f: &Fonts, v: &View) -> Rect {
    let geo = geo(v);
    if empty_block(v) {
        // No "Ctrl V" hint: the panel never takes the keyboard.
        let w = text_width("Paste link", &f.title) + 28.0;
        let l = W / 2.0 - w / 2.0;
        let top = geo.field + 140.0;
        (l, top, l + w, top + 32.0)
    } else {
        let w = text_width("Paste", &f.small_medium) + 24.0;
        (X1 - 6.0 - w, geo.field + 6.0, X1 - 6.0, geo.field + 34.0)
    }
}

fn card_button(f: &Fonts, v: &View) -> Option<Rect> {
    let label = match &v.engine {
        EngineCard::Missing { .. } => "Fetch yt-dlp",
        EngineCard::Failed(_) => "Try again",
        _ => return None,
    };
    let top = geo(v).card + card_h(&v.engine) - 12.0 - 28.0;
    let w = text_width(label, &f.small_medium) + 28.0;
    Some((X0 + 14.0, top, X0 + 14.0 + w, top + 28.0))
}

fn clear_rect(f: &Fonts, top: f32) -> Rect {
    let w = text_width("Clear list", &f.small);
    (X1 - 8.0 - w - 6.0, top + 4.0, X1, top + HEAD_H)
}

fn layout(f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let geo = geo(v);
    let mut out: Vec<(Hit, Rect)> = Vec::new();
    if let Some(r) = card_button(f, v) {
        out.push((Hit::Fetch, r));
    }
    out.push((Hit::Paste, paste_rect(f, v)));
    if !empty_block(v) {
        // A click on the field itself pastes too: the panel never takes the keyboard.
        out.push((Hit::Paste, (X0, geo.field, X1, geo.field + 40.0)));
        out.push((Hit::Folder, (X0, geo.h - FOOT + 8.0, X1, geo.h - 4.0)));
    }
    for (item, top) in item_tops(v) {
        if top + item.h() <= geo.list_top || top >= geo.list_bottom {
            continue;
        }
        match item {
            Item::Head(Section::Done) => out.push((Hit::ClearList, clear_rect(f, top))),
            Item::Head(_) => {}
            Item::Row(i) => {
                for (b, r) in buttons(f, &v.rows[i], top) {
                    out.push((Hit::Btn(i, b), r));
                }
                out.push((
                    Hit::Row(i),
                    (
                        X0,
                        top.max(geo.list_top),
                        X1,
                        (top + ROW_H).min(geo.list_bottom),
                    ),
                ));
            }
        }
    }
    out
}

pub fn hit(f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    layout(f, v)
        .into_iter()
        .find(|(_, r)| inside(r, x, y))
        .map(|(h, _)| h)
}

/// The waiting row's format chip in panel coordinates, for anchoring the format menu.
pub fn btn_rect(f: &Fonts, v: &View, row: usize, which: Btn) -> Option<(f32, f32, f32, f32)> {
    item_tops(v)
        .into_iter()
        .find(|(i, _)| *i == Item::Row(row))
        .and_then(|(_, top)| {
            buttons(f, &v.rows[row], top)
                .into_iter()
                .find(|(b, _)| *b == which)
                .map(|(_, r)| r)
        })
}

fn track(t: &Theme) -> Color {
    if t.dark {
        rgba(0xFFFFFF, 0.12)
    } else {
        rgba(0x000000, 0.10)
    }
}

fn tone_color(t: &Theme, tone: Tone) -> Color {
    match tone {
        Tone::Ink => t.text,
        Tone::Grey => t.passed,
        Tone::Ember => t.ember,
    }
}

/// A 28 px status chip: r 12 hairline track, a 2 px arc from 12 o'clock, a 12 px icon inside.
fn ring_chip(
    cv: &mut dyn Canvas,
    t: &Theme,
    cx: f32,
    cy: f32,
    from: f32,
    to: f32,
    arc: Color,
    icon: Option<(Icon, Color)>,
) {
    cv.stroke_ellipse(&ellipse(cx, cy, 12.0), track(t), 1.5, Stroke::Plain);
    cv.arc(cx, cy, 12.0, from, to, arc, 2.0, Stroke::Round);
    if let Some((i, c)) = icon {
        paint_icon(cv, i, cx, cy, 12.0, c);
    }
}

fn kind_icon(audio: bool) -> Icon {
    if audio {
        Icon::Music
    } else {
        Icon::Film
    }
}

fn draw_chip(cv: &mut dyn Canvas, t: &Theme, row: &Row, cx: f32, cy: f32) {
    let kind = kind_icon(row.audio);
    match row.chip {
        Chip::Reading => ring_chip(cv, t, cx, cy, 0.0, 90.0, t.text, None),
        Chip::Ring { fraction, tone } => {
            let icon = if tone == Tone::Ember {
                Icon::Drive
            } else {
                kind
            };
            ring_chip(
                cv,
                t,
                cx,
                cy,
                0.0,
                360.0 * fraction,
                tone_color(t, tone),
                Some((icon, if tone == Tone::Ember { t.ember } else { t.text })),
            );
        }
        Chip::Paused { fraction } => ring_chip(
            cv,
            t,
            cx,
            cy,
            0.0,
            360.0 * fraction,
            t.passed,
            Some((Icon::Pause, t.muted)),
        ),
        Chip::Waiting => {
            cv.stroke_ellipse(&ellipse(cx, cy, 12.0), t.passed, 1.5, Stroke::Dotted);
            paint_icon(cv, kind, cx, cy, 12.0, t.muted);
        }
        Chip::Done => {
            cv.fill_ellipse(&ellipse(cx, cy, 14.0), t.chip);
            paint_icon(cv, Icon::Tick, cx, cy, 14.0, t.text);
        }
        Chip::Failed(kind) => {
            cv.fill_ellipse(&ellipse(cx, cy, 14.0), with_alpha(t.ember, 0.14));
            let icon = if kind == ErrorKind::Private {
                Icon::Private
            } else {
                Icon::Alert
            };
            paint_icon(cv, icon, cx, cy, 13.0, t.ember);
        }
    }
}

fn button_icon(b: Btn) -> Icon {
    match b {
        Btn::Pause => Icon::Pause,
        Btn::Resume => Icon::Play,
        Btn::Folder => Icon::Folder,
        _ => Icon::Close,
    }
}

fn draw_row(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    i: usize,
    row: &Row,
    top: f32,
    hover: Option<Hit>,
) {
    let row_hover = matches!(hover, Some(Hit::Row(j) | Hit::Btn(j, _)) if j == i);
    if row_hover {
        cv.fill_rrect(&rrect(X0, top, X1, top + ROW_H, 8.0), t.raised);
    }
    let cy = top + ROW_H / 2.0;
    draw_chip(cv, t, row, X0 + 22.0, cy);
    let btns = buttons(f, row, top);
    for (b, r) in &btns {
        let on = hover == Some(Hit::Btn(i, *b));
        let fill = if on { t.button_hover } else { t.button };
        match (b, b.label()) {
            (_, Some(label)) => {
                cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), fill);
                cv.center_text(label, &f.small_medium, (r.0 + r.2) / 2.0, r.1, 28.0, t.text);
            }
            (Btn::Format, _) => {
                cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), fill);
                let ink = t.text;
                cv.text(
                    &row.format_chip,
                    &f.small_medium,
                    r.0 + 10.0,
                    r.1,
                    r.2 - r.0,
                    28.0,
                    ink,
                );
                paint_icon(cv, Icon::ChevronDown, r.2 - 15.0, cy, 12.0, ink);
            }
            _ => {
                // Quiet icons until the row is hovered, then full buttons.
                if row_hover {
                    cv.fill_ellipse(&ellipse((r.0 + r.2) / 2.0, cy, 14.0), fill);
                }
                let c = if row_hover { t.text } else { t.muted };
                paint_icon(cv, button_icon(*b), (r.0 + r.2) / 2.0, cy, 14.0, c);
            }
        }
    }
    let right = btns.first().map_or(X1 - 8.0, |(_, r)| r.0 - 10.0);
    let x = X0 + 44.0;
    clipped_text(
        cv,
        &row.title,
        &f.title,
        x,
        top + 6.0,
        right - x,
        18.0,
        t.text,
    );
    clipped_text(
        cv,
        &row.line,
        &f.small,
        x,
        top + 24.0,
        right - x,
        16.0,
        t.muted,
    );
}

fn section_name(s: Section) -> &'static str {
    match s {
        Section::NeedsYou => "Needs you",
        Section::Now => "Now",
        Section::Done => "Done today",
    }
}

fn draw_list(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let geo = geo(v);
    let muted = t.muted;
    cv.push_clip(0.0, geo.list_top, W, geo.list_bottom);
    for (item, top) in item_tops(v) {
        if top + item.h() < geo.list_top || top > geo.list_bottom {
            continue;
        }
        match item {
            Item::Head(s) => {
                cv.text(
                    section_name(s),
                    &f.small,
                    X0 + 4.0,
                    top + 4.0,
                    160.0,
                    HEAD_H - 4.0,
                    muted,
                );
                match s {
                    Section::Now => cv.right_text(
                        "2 at a time",
                        &f.small,
                        X1 - 8.0,
                        top + 4.0,
                        HEAD_H - 4.0,
                        muted,
                    ),
                    Section::Done => {
                        let c = if hover == Some(Hit::ClearList) {
                            t.text
                        } else {
                            t.muted
                        };
                        cv.right_text("Clear list", &f.small, X1 - 8.0, top + 4.0, HEAD_H - 4.0, c);
                    }
                    Section::NeedsYou => {}
                }
            }
            Item::Row(i) => draw_row(cv, t, f, i, &v.rows[i], top, hover),
        }
    }
    cv.pop_clip();
    let ms = max_scroll(v);
    if ms > 0.0 {
        let track_h = geo.list_bottom - geo.list_top;
        let th = (track_h * track_h / content_h(v)).max(24.0);
        let ty = geo.list_top + (track_h - th) * (v.scroll / ms);
        cv.fill_rrect(&rrect(W - 7.0, ty, W - 4.0, ty + th, 1.5), t.track);
    }
}

fn draw_card(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let y = geo(v).card;
    let h = card_h(&v.engine);
    let ink = t.text;
    let muted = t.muted;
    cv.fill_rrect(&rrect(X0, y, X1, y + h, 14.0), t.raised);
    cv.stroke_rrect(
        &rrect(X0 + 0.5, y + 0.5, X1 - 0.5, y + h - 0.5, 13.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    lip(cv, t, X0, y, X1, y + h, 14.0, 1.0);
    let title = if matches!(v.engine, EngineCard::Failed(_)) {
        "Couldn't fetch the engine"
    } else {
        "Getting downloads ready"
    };
    cv.text(title, &f.title, X0 + 14.0, y + 12.0, 260.0, 18.0, ink);
    cv.right_text("once only", &f.small, X1 - 14.0, y + 13.0, 16.0, muted);
    let body: [String; 2] = match &v.engine {
        EngineCard::Missing { waiting } => [
            "Saving uses yt-dlp, a free engine from GitHub (about 18 MB).".into(),
            if *waiting {
                "It comes only when you press Fetch. Your link waits."
            } else {
                "It comes only when you press Fetch."
            }
            .into(),
        ],
        EngineCard::Failed(why) => [
            format!("{why}."),
            "Check the connection, then try again.".into(),
        ],
        EngineCard::Fetching {
            done,
            total,
            waiting,
        } => {
            let cy = y + 12.0 + 18.0 + 10.0 + 18.0;
            let frac = total
                .filter(|t| *t > 0)
                .map_or(0.25, |t| (*done as f32 / t as f32).min(1.0));
            ring_chip(
                cv,
                t,
                X0 + 26.0,
                cy,
                0.0,
                360.0 * frac,
                t.text,
                Some((Icon::Download, t.text)),
            );
            cv.text(
                "Fetching yt-dlp",
                &f.body,
                X0 + 48.0,
                cy - 18.0,
                280.0,
                18.0,
                ink,
            );
            let amount = match total {
                Some(tt) => crate::downloads_model::of(*done, *tt),
                None => crate::downloads_model::size(*done),
            };
            let tail = if *waiting {
                "your link waits, then starts"
            } else {
                "then you can save links"
            };
            clipped_text(
                cv,
                &format!("{amount} · {tail}"),
                &f.small,
                X0 + 48.0,
                cy,
                X1 - X0 - 62.0,
                16.0,
                muted,
            );
            return;
        }
        EngineCard::None => return,
    };
    for (k, line) in body.iter().enumerate() {
        clipped_text(
            cv,
            line,
            &f.small,
            X0 + 14.0,
            y + 36.0 + 16.0 * k as f32,
            X1 - X0 - 28.0,
            16.0,
            muted,
        );
    }
    if let Some(r) = card_button(f, v) {
        let fill = if hover == Some(Hit::Fetch) {
            with_alpha(t.text, 0.85)
        } else {
            t.text
        };
        cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), fill);
        let label = if matches!(v.engine, EngineCard::Failed(_)) {
            "Try again"
        } else {
            "Fetch yt-dlp"
        };
        cv.center_text(
            label,
            &f.small_medium,
            (r.0 + r.2) / 2.0,
            r.1,
            28.0,
            t.panel,
        );
    }
}

fn draw_field_msg(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    field: &Field,
    top: f32,
    centred: bool,
) {
    let lines: Vec<&str> = match field {
        Field::Empty => return,
        Field::Unsupported(_) => vec![
            "This site isn't supported. Try a link from YouTube,",
            "Instagram, TikTok or X.",
        ],
        Field::NoLink => vec!["Copy a video or audio link first, then press Paste."],
    };
    let c = if matches!(field, Field::Unsupported(_)) {
        t.ember
    } else {
        t.muted
    };
    for (k, l) in lines.iter().enumerate() {
        let y = top + 16.0 * k as f32;
        if centred {
            cv.center_text(l, &f.small, W / 2.0, y, 16.0, c);
        } else {
            if k == 0 {
                paint_icon(cv, Icon::Alert, X0 + 20.0, y + 8.0, 12.0, c);
            }
            cv.text(l, &f.small, X0 + 32.0, y, X1 - X0 - 32.0, 16.0, c);
        }
    }
}

fn draw_empty(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let y = geo(v).field;
    let ink = t.text;
    let muted = t.muted;
    cv.fill_ellipse(&ellipse(W / 2.0, y + 34.0, 18.0), t.chip);
    paint_icon(cv, Icon::Download, W / 2.0, y + 34.0, 16.0, ink);
    cv.center_text(
        "No downloads yet",
        &f.panel_title,
        W / 2.0,
        y + 64.0,
        20.0,
        ink,
    );
    cv.center_text(
        "Copy a video link anywhere and the",
        &f.body,
        W / 2.0,
        y + 90.0,
        18.0,
        muted,
    );
    cv.center_text(
        "orb offers to save it. Or paste one here.",
        &f.body,
        W / 2.0,
        y + 108.0,
        18.0,
        muted,
    );
    let r = paste_rect(f, v);
    let fill = if hover == Some(Hit::Paste) {
        with_alpha(t.text, 0.85)
    } else {
        t.text
    };
    cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 16.0), fill);
    cv.center_text(
        "Paste link",
        &f.title,
        (r.0 + r.2) / 2.0,
        r.1,
        32.0,
        t.panel,
    );
    draw_field_msg(cv, t, f, &v.field, y + EMPTY_H - 6.0, true);
}

fn draw_field(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let y = geo(v).field;
    let ember = matches!(v.field, Field::Unsupported(_));
    cv.fill_rrect(&rrect(X0, y, X1, y + 40.0, 20.0), t.panel);
    let edge = if ember { t.ember } else { t.hair };
    cv.stroke_rrect(
        &rrect(X0 + 0.5, y + 0.5, X1 - 0.5, y + 39.5, 19.5),
        edge,
        1.0,
        Stroke::Plain,
    );
    let muted = t.muted;
    paint_icon(cv, Icon::Link, X0 + 22.0, y + 20.0, 15.0, muted);
    let r = paste_rect(f, v);
    match &v.field {
        Field::Unsupported(link) => clipped_text(
            cv,
            link,
            &f.body,
            X0 + 40.0,
            y,
            r.0 - X0 - 48.0,
            40.0,
            t.text,
        ),
        _ => cv.text(
            "Paste a video or audio link",
            &f.body,
            X0 + 40.0,
            y,
            r.0 - X0 - 48.0,
            40.0,
            muted,
        ),
    }
    let fill = if hover == Some(Hit::Paste) {
        t.button_hover
    } else {
        t.button
    };
    cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), fill);
    cv.center_text(
        "Paste",
        &f.small_medium,
        (r.0 + r.2) / 2.0,
        r.1,
        28.0,
        t.text,
    );
    draw_field_msg(cv, t, f, &v.field, y + 46.0, false);
}

pub fn draw(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let geo = geo(v);
    let ink = t.text;
    let muted = t.muted;
    cv.fill_rrect(&rrect(0.0, 0.0, W, geo.h, 20.0), t.panel);
    cv.stroke_rrect(
        &rrect(0.5, 0.5, W - 0.5, geo.h - 0.5, 19.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    cv.text("Downloads", &f.panel_title, X0, 16.0, 140.0, 18.0, ink);
    // "2 of 5 · 42%": the count and percent roll on the quick spring as downloads move.
    crate::motion::roll_text_right(
        cv,
        "downloads.count",
        &v.count,
        crate::motion::Roll::Quick,
        &f.small,
        X1,
        17.0,
        16.0,
        muted,
    );
    if v.engine != EngineCard::None {
        draw_card(cv, t, f, v, hover);
    }
    if empty_block(v) {
        draw_empty(cv, t, f, v, hover);
        return;
    }
    draw_field(cv, t, f, v, hover);
    draw_list(cv, t, f, v, hover);
    let fy = geo.h - FOOT + 8.0;
    cv.line(
        pt(X0, fy - 0.5),
        pt(X1, fy - 0.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    let c = if hover == Some(Hit::Folder) {
        ink
    } else {
        muted
    };
    paint_icon(cv, Icon::Folder, X0 + 8.0, fy + 18.0, 14.0, c);
    let fw = text_width(&v.free, &f.small);
    cv.right_text(&v.free, &f.small, X1 - 4.0, fy, 36.0, muted);
    clipped_text(
        cv,
        &v.folder,
        &f.small,
        X0 + 24.0,
        fy,
        X1 - X0 - 40.0 - fw,
        36.0,
        c,
    );
}

// ---- the format menu ----

pub const MENU_W: f32 = 232.0;
const MENU_ROW: f32 = 32.0;
pub const MENU_H: f32 = 4.0 + 4.0 * MENU_ROW + 4.0;

pub fn menu_hit(x: f32, y: f32) -> Option<usize> {
    if !(4.0..=MENU_W - 4.0).contains(&x) || y < 4.0 {
        return None;
    }
    let i = ((y - 4.0) / MENU_ROW) as usize;
    (i < 4).then_some(i)
}

/// `chosen` is the format Save would use (active fill and a check).
pub fn draw_menu(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, chosen: Format, hover: Option<usize>) {
    let ink = t.text;
    let muted = t.muted;
    cv.fill_rrect(&rrect(0.0, 0.0, MENU_W, MENU_H, 12.0), t.panel);
    cv.stroke_rrect(
        &rrect(0.5, 0.5, MENU_W - 0.5, MENU_H - 0.5, 11.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    for (i, (format, kind, label, key)) in menu_rows().into_iter().enumerate() {
        let top = 4.0 + MENU_ROW * i as f32;
        let on = format == chosen;
        if on || hover == Some(i) {
            cv.fill_rrect(
                &rrect(4.0, top, MENU_W - 4.0, top + MENU_ROW, 8.0),
                if on { t.chip } else { t.raised },
            );
        }
        let cy = top + MENU_ROW / 2.0;
        paint_icon(
            cv,
            kind_icon(format.kind() == MediaKind::Audio),
            4.0 + 16.0,
            cy,
            14.0,
            muted,
        );
        let kw = text_width(kind, &f.title);
        cv.text(kind, &f.title, 4.0 + 32.0, top, kw + 2.0, MENU_ROW, ink);
        cv.text(
            &label,
            &f.body,
            4.0 + 36.0 + kw,
            top,
            120.0,
            MENU_ROW,
            muted,
        );
        if on {
            paint_icon(cv, Icon::Tick, MENU_W - 4.0 - 16.0, cy, 14.0, ink);
        } else if !key.is_empty() {
            cv.right_text(key, &f.small, MENU_W - 4.0 - 12.0, top, MENU_ROW, muted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_downloads::{JobId, Queue};

    fn view(rows: Vec<Row>, engine: EngineCard, field: Field) -> View {
        View {
            count: String::new(),
            engine,
            field,
            rows,
            folder: "Downloads".into(),
            free: "1 GB free on C:".into(),
            scroll: 0.0,
        }
    }

    fn some_rows(n: usize) -> Vec<Row> {
        let mut q = Queue::new();
        let ids: Vec<JobId> = (0..n)
            .map(|i| {
                q.add(
                    app_downloads::MediaLink::parse(&format!(
                        "https://youtu.be/abcdefghij{}",
                        i % 10
                    ))
                    .unwrap(),
                    Format::default(),
                    "C:\\D".into(),
                )
            })
            .collect();
        assert_eq!(ids.len(), n);
        crate::downloads_model::rows(&q, &|_| "10:42".into())
    }

    #[test]
    fn empty_panel_is_the_empty_block_and_grows_with_the_card() {
        let plain = height(&view(Vec::new(), EngineCard::None, Field::Empty));
        let first_run = height(&view(
            Vec::new(),
            EngineCard::Missing { waiting: false },
            Field::Empty,
        ));
        assert_eq!(
            first_run - plain,
            card_h(&EngineCard::Missing { waiting: false }) + 10.0
        );
    }

    #[test]
    fn a_refused_site_on_an_empty_panel_shows_the_field_with_the_link() {
        let refused = view(
            Vec::new(),
            EngineCard::None,
            Field::Unsupported("shopee.co.id/product/1".into()),
        );
        assert!(!empty_block(&refused), "the ember field holds the link");
        assert_eq!(
            height(&refused),
            (geo(&refused).field + 40.0 + field_msg_h(&refused.field) + 6.0 + FOOT).ceil()
        );
        assert!(
            empty_block(&view(Vec::new(), EngineCard::None, Field::NoLink)),
            "no link: the hint under the empty block"
        );
    }

    #[test]
    fn rows_grow_the_panel_up_to_a_limit_then_scroll() {
        let few = view(some_rows(2), EngineCard::None, Field::Empty);
        assert!(height(&few) < MAX_H);
        assert_eq!(max_scroll(&few), 0.0);
        let many = view(some_rows(20), EngineCard::None, Field::Empty);
        assert_eq!(height(&many), MAX_H);
        assert!(max_scroll(&many) > 0.0);
    }

    #[test]
    fn the_menu_has_four_rows_and_padding_is_not_a_row() {
        assert_eq!(menu_hit(100.0, 2.0), None);
        assert_eq!(menu_hit(100.0, 4.0 + 16.0), Some(0));
        assert_eq!(menu_hit(100.0, 4.0 + 3.0 * 32.0 + 16.0), Some(3));
        assert_eq!(menu_hit(100.0, MENU_H - 2.0), None);
    }
}
