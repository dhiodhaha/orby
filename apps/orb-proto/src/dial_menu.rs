//! What the dial shows and what a pick does, on both systems: the six main items and their
//! value tags, the children of the three branches (Focus 15/25/50 or Pause/Skip/Stop, Drink's
//! glasses, More's Downloads, Disk, History, Notices, Settings, All tasks), and the action of
//! each pick. The windows (dial.rs on Windows, the Mac `App`'s dial) own aiming, keys, springs
//! and closing; they call `items`, `children`, `pick` and `pick_child` here.
//!
//! Units that are not shared yet are reached through `pending`: on Windows the real unit, on
//! macOS a plain child and a quiet "Coming to Mac" pill. Each later stage moves its unit out of
//! `pending` and calls it directly.

use app_core::prayer as pr;

use crate::panels::{self, Kind};
use crate::settings_model::Section;
use crate::ui::{DialChild, DialItem, Icon};
use crate::{disk, habits, history, notices, prayer, settings, App};

pub const TASKS: usize = 0;
pub const TODAY: usize = 1;
pub const PRAYER: usize = 2;
pub const FOCUS: usize = 3;
pub const DRINK: usize = 4;
pub const MORE: usize = 5;
/// The main ring's items.
pub const N: usize = 6;

/// The items' names in the log ("dial pick focus").
pub const LOG_NAMES: [&str; N] = ["tasks", "today", "prayer", "focus", "drink", "more"];

/// Focus, Drink and More fan out a branch; the others open their panel.
pub fn has_branch(i: usize) -> bool {
    i == FOCUS || i == DRINK || i == MORE
}

/// The branch's name in the log ("dial pick drink child 1").
pub fn branch_log_name(parent: usize) -> &'static str {
    match parent {
        FOCUS => "focus",
        DRINK => "drink",
        _ => "more",
    }
}

/// The main ring: icon, name and value tag of each item.
pub fn items(app: &App) -> Vec<DialItem> {
    let (np, nat, _) = prayer::next_prayer(app);
    vec![
        DialItem {
            icon: Icon::Tasks,
            name: "Tasks",
            value: pending::tasks_value(app),
        },
        DialItem {
            icon: Icon::Today,
            name: "Today",
            value: pending::today_value(app),
        },
        DialItem {
            icon: Icon::Prayer,
            name: "Prayer",
            value: format!("{} {}", np.name(), pr::hhmm(nat)),
        },
        DialItem {
            icon: Icon::Focus,
            name: "Focus",
            value: habits::dial_focus_value(app),
        },
        DialItem {
            icon: Icon::Drink,
            name: "Drink",
            value: habits::dial_drink_value(app),
        },
        DialItem {
            icon: Icon::More,
            name: "More",
            value: disk::dial_value(app).unwrap_or_else(|| pending::downloads_value(app)),
        },
    ]
}

/// The open branch's children and its default child.
pub fn children(app: &App, parent: usize) -> (Vec<DialChild>, usize) {
    match parent {
        FOCUS => habits::focus_children(app),
        DRINK => habits::drink_children(app),
        MORE => {
            let (mut kids, default) = pending::downloads_children(app);
            kids.push(disk::dial_child(app));
            kids.push(history::dial_child(app));
            kids.push(notices::dial_child(app));
            kids.push(settings::dial_child(app));
            kids.push(pending::all_tasks_child(app));
            (kids, default)
        }
        _ => (Vec::new(), 0),
    }
}

/// A main item's action: Tasks, Today, Prayer, Focus and Drink open their panels (More has no
/// panel of its own; the window fans out its children instead and never calls this for it).
pub fn pick(app: &mut App, i: usize) {
    match i {
        TASKS => pending::open_tasks(app),
        TODAY => pending::open_today(app),
        PRAYER => prayer::open_panel(app),
        FOCUS => panels::open_panel(app, Kind::Focus),
        DRINK => panels::open_panel(app, Kind::Drink),
        _ => pending::open_downloads(app),
    }
}

/// A branch child's action, at once: start a session, log a glass, or open More's child.
pub fn pick_child(app: &mut App, parent: usize, j: usize) {
    match parent {
        FOCUS => habits::pick_focus_child(app, j),
        DRINK => habits::pick_drink_child(app, j),
        _ if j == 1 => disk::open(app),
        _ if j == 2 => history::open(app, None),
        _ if j == 3 => notices::open_panel(app),
        _ if j == 4 => settings::open(app, Section::General),
        _ if j == 5 => pending::open_all_tasks(app),
        _ => pending::open_downloads(app),
    }
}

/// Units not shared yet: the real ones on Windows.
#[cfg(windows)]
mod pending {
    use crate::ui::DialChild;
    use crate::{calendar, downloads, tasks, tasks_window, App};

    pub fn tasks_value(app: &App) -> String {
        tasks::dial_value(app)
    }

    pub fn today_value(app: &App) -> String {
        tasks::dial_today_value(app)
    }

    pub fn downloads_value(app: &App) -> String {
        downloads::dial_value(app)
    }

    pub fn downloads_children(app: &App) -> (Vec<DialChild>, usize) {
        downloads::dial_children(app)
    }

    pub fn all_tasks_child(app: &App) -> DialChild {
        tasks_window::dial_child(app)
    }

    pub fn open_tasks(app: &mut App) {
        tasks::open(app, None);
    }

    pub fn open_today(app: &mut App) {
        calendar::open(app);
    }

    pub fn open_downloads(app: &mut App) {
        downloads::open(app);
    }

    pub fn open_all_tasks(app: &mut App) {
        tasks_window::show(app);
    }
}

/// Units not on the Mac yet: a plain child, no value, and "Coming to Mac" when picked.
#[cfg(not(windows))]
mod pending {
    use crate::ui::{info_pill, DialChild, Icon};
    use crate::{notices, App};

    fn plain(icon: Icon, name: &str) -> DialChild {
        DialChild {
            text: String::new(),
            icon: Some(icon),
            name: name.into(),
            value: String::new(),
        }
    }

    fn coming(app: &mut App, what: &str) {
        crate::log_line(app, &format!("dial: {what} is coming to Mac"));
        notices::reply(app, info_pill(Icon::Info, "Coming to Mac", what));
    }

    pub fn tasks_value(_app: &App) -> String {
        String::new()
    }

    pub fn today_value(_app: &App) -> String {
        String::new()
    }

    pub fn downloads_value(_app: &App) -> String {
        String::new()
    }

    /// Downloads, with Disk (the next child, the first that works on the Mac) as the default.
    pub fn downloads_children(_app: &App) -> (Vec<DialChild>, usize) {
        (vec![plain(Icon::Download, "Downloads")], 1)
    }

    pub fn all_tasks_child(_app: &App) -> DialChild {
        plain(Icon::List, "All tasks")
    }

    pub fn open_tasks(app: &mut App) {
        coming(app, "Tasks");
    }

    pub fn open_today(app: &mut App) {
        coming(app, "Today");
    }

    pub fn open_downloads(app: &mut App) {
        coming(app, "Downloads");
    }

    pub fn open_all_tasks(app: &mut App) {
        coming(app, "All tasks");
    }
}
