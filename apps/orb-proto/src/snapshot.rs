//! Snapshot mode: `ORB_SNAPSHOT=<dir>` renders the dial (ring, and a branch), two pills and the
//! Prayer panel in light and dark to PNGs at 2x, offscreen, then exits. The scenes use fixed data
//! (Yogyakarta, Kemenag, 10 Nov 2026 at 14:37:25) so two runs, or two systems, can be compared.

use std::path::Path;

use app_core::prayer::{self as pr, Location, Prayer, Times};

use crate::settings_model::PrayerMethod;
use crate::ui::{DialBranch, DialChild, DialItem, Icon, Pill, PillAction, PillPart};

pub const SCALE: f32 = 2.0;
const NOW_MIN: f64 = 14.0 * 60.0 + 37.0 + 25.0 / 60.0;

pub fn times() -> (Times, u32) {
    let loc = Location {
        lat: -7.7956,
        lng: 110.3695,
        tz: 7.0,
        elevation_m: 0.0,
    };
    let m = PrayerMethod::Kemenag.method();
    (
        pr::times(loc, m, 2026, 11, 10),
        pr::times(loc, m, 2026, 11, 11).subuh,
    )
}

pub fn dial_items(t: &Times, tomorrow: u32) -> Vec<DialItem> {
    let (np, nat, _) = pr::next(t, NOW_MIN, tomorrow);
    vec![
        DialItem {
            icon: Icon::Tasks,
            name: "Tasks",
            value: "3".into(),
        },
        DialItem {
            icon: Icon::Today,
            name: "Today",
            value: "Standup 15:30".into(),
        },
        DialItem {
            icon: Icon::Prayer,
            name: "Prayer",
            value: format!("{} {}", np.name(), pr::hhmm(nat)),
        },
        DialItem {
            icon: Icon::Focus,
            name: "Focus",
            value: String::new(),
        },
        DialItem {
            icon: Icon::Drink,
            name: "Drink",
            value: "750 ml".into(),
        },
        DialItem {
            icon: Icon::More,
            name: "More",
            value: String::new(),
        },
    ]
}

pub fn drink_children() -> Vec<DialChild> {
    [150, 250, 500]
        .iter()
        .map(|ml| DialChild {
            text: ml.to_string(),
            icon: None,
            name: "Drink".into(),
            value: format!("+{ml} ml"),
        })
        .collect()
}

pub fn drink_pill() -> Pill {
    Pill {
        icon: Icon::Drink,
        dot: false,
        title: "Minum dulu".into(),
        value: "750 / 2000 ml".into(),
        action: Some(("+250 ml".into(), PillAction::AddDrink(250))),
        closable: true,
        ttl_ms: 6000,
    }
}

pub fn prayer_pill(t: &Times) -> Pill {
    Pill {
        icon: Icon::Bell,
        dot: false,
        title: "10 menit lagi Ashar".into(),
        value: pr::hhmm(t.of(Prayer::Ashar)),
        action: None,
        closable: true,
        ttl_ms: 6000,
    }
}

pub struct PanelData {
    pub times: Times,
    pub next: Prayer,
    pub mins_left: f64,
    pub now_min: f64,
}

pub fn panel_data() -> PanelData {
    let (t, tomorrow) = times();
    let (next, _, mins_left) = pr::next(&t, NOW_MIN, tomorrow);
    PanelData {
        times: t,
        next,
        mins_left,
        now_min: NOW_MIN,
    }
}

pub fn panel_view(d: &PanelData) -> crate::panel::View<'_> {
    crate::panel::View {
        times: &d.times,
        now_min: d.now_min,
        next: d.next,
        mins_left: d.mins_left,
        place: "Yogyakarta",
        method: "Kemenag",
        qibla: "295° WNW".into(),
        reminders: [true, true, false],
        hover: None,
    }
}

pub fn write(dir: &Path, name: &str, w: u32, h: u32, mut rgba: Vec<u8>) {
    crate::png::unpremultiply(&mut rgba);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{name}.png"));
    match std::fs::write(&path, crate::png::encode(w, h, &rgba)) {
        Ok(()) => println!("wrote {}", path.display()),
        Err(e) => eprintln!("{}: {e}", path.display()),
    }
}

/// The hover state the pill scenes draw with.
pub const PILL_HOVER: PillPart = PillPart::None;

#[cfg(windows)]
pub fn run_windows(dir: &Path) -> windows::core::Result<()> {
    use crate::gfx::{render_offscreen, D2dCanvas, Shared};
    use crate::ui;

    let g = Shared::new()?;
    let fonts = ui::Fonts::new(&g);
    let (t, tomorrow) = times();
    let items = dial_items(&t, tomorrow);
    let kids = drink_children();
    let pd = panel_data();
    for dark in [false, true] {
        let th = ui::theme(dark);
        let tag = if dark { "dark" } else { "light" };
        let (w, h, px) = render_offscreen(&g, ui::DIAL_W, ui::DIAL_H, SCALE, |dc| {
            ui::draw_dial(
                &mut D2dCanvas::new(&g, dc),
                &th,
                &fonts,
                &items,
                Some(2),
                1.0,
                false,
                false,
                None,
                None,
                1.0,
            );
        })?;
        write(dir, &format!("dial-{tag}"), w, h, px);
        let branch = DialBranch {
            parent: 4,
            children: &kids,
            aimed: Some(1),
            reveal: 1.0,
        };
        let (w, h, px) = render_offscreen(&g, ui::DIAL_W, ui::DIAL_H, SCALE, |dc| {
            ui::draw_dial(
                &mut D2dCanvas::new(&g, dc),
                &th,
                &fonts,
                &items,
                Some(4),
                1.0,
                false,
                false,
                Some(&branch),
                None,
                0.0,
            );
        })?;
        write(dir, &format!("dial-branch-{tag}"), w, h, px);
        for (name, p) in [
            ("pill-drink", drink_pill()),
            ("pill-prayer", prayer_pill(&t)),
        ] {
            let l = ui::layout_pill(&fonts, &p);
            let (pw, ph) = (l.w + 2.0 * ui::PILL_PAD, ui::PILL_H + 2.0 * ui::PILL_PAD);
            let (w, h, px) = render_offscreen(&g, pw, ph, SCALE, |dc| {
                ui::draw_pill(
                    &mut D2dCanvas::new(&g, dc),
                    &th,
                    &fonts,
                    &p,
                    &l,
                    1.0,
                    true,
                    PILL_HOVER,
                    ui::PILL_PAD,
                    ui::PILL_PAD,
                );
            })?;
            write(dir, &format!("{name}-{tag}"), w, h, px);
        }
        let view = panel_view(&pd);
        let (w, h, px) = render_offscreen(&g, crate::panel::W, crate::panel::H, SCALE, |dc| {
            crate::panel::draw(&mut D2dCanvas::new(&g, dc), &th, &fonts, &view);
        })?;
        write(dir, &format!("prayer-{tag}"), w, h, px);
    }
    Ok(())
}

/// The same scenes on macOS through Core Graphics and Core Text, plus the orb on Metal.
#[cfg(target_os = "macos")]
pub fn run_mac(dir: &Path) {
    use crate::canvas::Canvas;
    use crate::mac::cg::Bitmap;
    use crate::ui;

    let fonts = ui::Fonts::new();
    let (t, tomorrow) = times();
    let items = dial_items(&t, tomorrow);
    let kids = drink_children();
    let pd = panel_data();
    let shot = |name: &str, w: f32, h: f32, draw: &dyn Fn(&mut dyn Canvas)| {
        let b = Bitmap::new(w, h, SCALE);
        draw(&mut b.canvas());
        write(dir, name, b.pw as u32, b.ph as u32, b.rgba());
    };
    for dark in [false, true] {
        let th = ui::theme(dark);
        let tag = if dark { "dark" } else { "light" };
        shot(&format!("dial-{tag}"), ui::DIAL_W, ui::DIAL_H, &|cv| {
            ui::draw_dial(
                cv,
                &th,
                &fonts,
                &items,
                Some(2),
                1.0,
                false,
                false,
                None,
                None,
                1.0,
            );
        });
        let branch = DialBranch {
            parent: 4,
            children: &kids,
            aimed: Some(1),
            reveal: 1.0,
        };
        shot(
            &format!("dial-branch-{tag}"),
            ui::DIAL_W,
            ui::DIAL_H,
            &|cv| {
                ui::draw_dial(
                    cv,
                    &th,
                    &fonts,
                    &items,
                    Some(4),
                    1.0,
                    false,
                    false,
                    Some(&branch),
                    None,
                    0.0,
                );
            },
        );
        for (name, p) in [
            ("pill-drink", drink_pill()),
            ("pill-prayer", prayer_pill(&t)),
        ] {
            let l = ui::layout_pill(&fonts, &p);
            let (pw, ph) = (l.w + 2.0 * ui::PILL_PAD, ui::PILL_H + 2.0 * ui::PILL_PAD);
            shot(&format!("{name}-{tag}"), pw, ph, &|cv| {
                ui::draw_pill(
                    cv,
                    &th,
                    &fonts,
                    &p,
                    &l,
                    1.0,
                    true,
                    PILL_HOVER,
                    ui::PILL_PAD,
                    ui::PILL_PAD,
                );
            });
        }
        let view = panel_view(&pd);
        shot(
            &format!("prayer-{tag}"),
            crate::panel::W,
            crate::panel::H,
            &|cv| {
                crate::panel::draw(cv, &th, &fonts, &view);
            },
        );
    }
    // The orb at rest: by day (Ashar light) and after Isya (dimmed).
    for (name, angle, brightness) in [("orb-day", 128.0, 1.0), ("orb-night", 90.0, 0.7)] {
        match crate::mac::metal::snapshot(angle, brightness, SCALE) {
            Some((w, h, px)) => write(dir, name, w, h, px),
            None => eprintln!("{name}: Metal is not available"),
        }
    }
}
