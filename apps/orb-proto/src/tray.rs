//! The tray icon, its menu, and the menu commands (also used to drive the exe in tests).

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::history_model::Tab;
use crate::panels::{self, Kind};
use crate::settings_model::Section;
use crate::today_model::Cue;
use crate::{
    calendar, disk, downloads, habits, history, notices, orb, prayer, settings, tasks, with_app,
    WM_APP_TRAY,
};

pub const MENU_PRAYER: usize = 1;
pub const MENU_TOGGLE: usize = 2;
pub const MENU_QUIT: usize = 3;
pub const MENU_NOTICE: usize = 4;
pub const MENU_TEST_10: usize = 5;
pub const MENU_TEST_AT: usize = 6;
pub const MENU_PANEL: usize = 7;
pub const MENU_FOCUS: usize = 8;
pub const MENU_DRINK: usize = 9;
pub const MENU_DRINK_DUE: usize = 10;
pub const MENU_FOCUS_1: usize = 11;
pub const MENU_TASKS: usize = 12;
pub const MENU_TASK_SAMPLES: usize = 13;
pub const MENU_CAL_LINK: usize = 14;
pub const MENU_TODAY: usize = 15;
pub const MENU_CAL_SOON: usize = 16;
pub const MENU_CAL_START: usize = 17;
pub const MENU_CAL_READ: usize = 18;
pub const MENU_CAL_REMOVE: usize = 19;
pub const MENU_DOWNLOADS: usize = 20;
pub const MENU_DL_PASTE: usize = 21;
pub const MENU_DISK: usize = 40;
pub const MENU_DISK_SCAN_CLIP: usize = 41;
pub const MENU_DISK_LOW_8: usize = 42;
pub const MENU_DISK_LOW_2: usize = 43;
pub const MENU_DISK_CHECK: usize = 44;
pub const MENU_DISK_FIND_C: usize = 45;
pub const MENU_HISTORY: usize = 50;
pub const MENU_HISTORY_DRINK: usize = 51;
pub const MENU_HISTORY_FOCUS: usize = 52;
pub const MENU_HISTORY_LOADING: usize = 53;
pub const MENU_PAUSE_30: usize = 60;
pub const MENU_PAUSE_60: usize = 61;
pub const MENU_PAUSE_TOMORROW: usize = 62;
pub const MENU_PAUSE_OFF: usize = 63;
pub const MENU_NOTICES: usize = 64;
pub const MENU_TEST_FULL_SCREEN: usize = 65;
pub const MENU_TEST_NOTICES: usize = 66;
pub const MENU_TEST_QUIET: usize = 67;
pub const MENU_UNMUTE_ALL: usize = 68;
pub const MENU_SETTINGS: usize = 70;
pub const MENU_FIRST_RUN: usize = 71;
pub const MENU_SETTINGS_PRAYER: usize = 72;
pub const MENU_SETTINGS_NOTICES: usize = 73;
pub const MENU_SETTINGS_ACCOUNT: usize = 74;
pub const MENU_SETTINGS_ABOUT: usize = 75;

pub fn add_tray(hwnd: HWND) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
            ..Default::default()
        };
        let tip: Vec<u16> = "Orby".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

pub fn remove_tray(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

pub fn tray_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu().unwrap();
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Settings"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_FIRST_RUN,
            w!("Test: show the first run again"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_NOTICE, w!("Test notice: drink"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_PANEL, w!("Open the Prayer panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_FOCUS, w!("Open the Focus panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_DRINK, w!("Open the Drink panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_TASKS, w!("Open the Tasks panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_TODAY, w!("Open the Today panel"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DOWNLOADS,
            w!("Open the Downloads panel"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DL_PASTE,
            w!("Paste a link into Downloads"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_DISK, w!("Open the Disk panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_HISTORY, w!("Open History"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_NOTICES, w!("Open Notices"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_PAUSE_30,
            w!("Pause notices for 30 min"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_PAUSE_60,
            w!("Pause notices for 1 hour"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_PAUSE_TOMORROW,
            w!("Pause notices until tomorrow"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_PAUSE_OFF, w!("Resume notices"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_UNMUTE_ALL,
            w!("Unmute every notice source"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DISK_SCAN_CLIP,
            w!("Scan the folder on the clipboard (in the background)"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_DISK_CHECK, w!("Check free space now"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_CAL_LINK,
            w!("Add calendar link (from the clipboard)"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_CAL_READ, w!("Read the calendar now"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_CAL_REMOVE,
            w!("Remove the calendar link"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DRINK_DUE,
            w!("Test: drink reminder due now"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_FOCUS_1,
            w!("Test: start a 1 min focus"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TASK_SAMPLES,
            w!("Test: add sample tasks (empty store only)"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_CAL_SOON,
            w!("Test: event pill, 10 min before"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_CAL_START,
            w!("Test: event pill, starting now"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DISK_LOW_8,
            w!("Test: low space on C:, 8% free (fresh)"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_DISK_LOW_2,
            w!("Test: low space on C:, 2% free"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_HISTORY_LOADING,
            w!("Test: History while days are loading"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TEST_NOTICES,
            w!("Test: sample notices from three sources"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_TEST_QUIET, w!("Test: a quiet notice"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TEST_FULL_SCREEN,
            w!("Test: act as full screen (on or off)"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TEST_10,
            w!("Test reminder: 10 menit lagi"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TEST_AT,
            w!("Test reminder: Memasuki waktu"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_PRAYER, w!("Play the prayer moment"));
        let label = with_app(|app| settings::shortcut_label(app)).unwrap_or_default();
        let toggle = HSTRING::from(format!("Show or hide\t{label}"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_TOGGLE, &toggle);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}

/// WM_COMMAND from the tray menu (or posted by a test driver).
pub fn on_command(hwnd: HWND, id: usize) {
    match id {
        MENU_NOTICE => {
            with_app(habits::drink_notice);
        }
        MENU_PRAYER => {
            with_app(prayer::play_prayer);
        }
        MENU_PANEL => {
            with_app(prayer::open_panel);
        }
        MENU_FOCUS => {
            with_app(|app| panels::open_panel(app, Kind::Focus));
        }
        MENU_DRINK => {
            with_app(|app| panels::open_panel(app, Kind::Drink));
        }
        MENU_TASKS => {
            with_app(|app| tasks::open(app, None));
        }
        MENU_TASK_SAMPLES => {
            with_app(tasks::add_samples);
        }
        MENU_TODAY => {
            with_app(calendar::open);
        }
        MENU_DOWNLOADS => {
            with_app(downloads::open);
        }
        MENU_DL_PASTE => {
            with_app(downloads::paste);
        }
        MENU_DISK => {
            with_app(disk::open);
        }
        MENU_HISTORY => {
            with_app(|app| history::open(app, None));
        }
        MENU_HISTORY_DRINK | MENU_HISTORY_FOCUS => {
            let tab = if id == MENU_HISTORY_DRINK {
                Tab::Drink
            } else {
                Tab::Focus
            };
            with_app(|app| history::open(app, Some(tab)));
        }
        MENU_HISTORY_LOADING => {
            with_app(history::test_loading);
        }
        MENU_NOTICES => {
            with_app(notices::open_panel);
        }
        MENU_PAUSE_30 | MENU_PAUSE_60 => {
            let mins = if id == MENU_PAUSE_30 { 30 } else { 60 };
            with_app(|app| {
                notices::pause(app, Some(std::time::Duration::from_secs(mins * 60)), false)
            });
        }
        MENU_PAUSE_TOMORROW => {
            with_app(|app| notices::pause(app, None, true));
        }
        MENU_PAUSE_OFF => {
            with_app(|app| notices::pause(app, None, false));
        }
        MENU_UNMUTE_ALL => {
            with_app(notices::unmute_all);
        }
        MENU_SETTINGS
        | MENU_SETTINGS_PRAYER
        | MENU_SETTINGS_NOTICES
        | MENU_SETTINGS_ACCOUNT
        | MENU_SETTINGS_ABOUT => {
            let section = match id {
                MENU_SETTINGS_PRAYER => Section::Prayer,
                MENU_SETTINGS_NOTICES => Section::Notices,
                MENU_SETTINGS_ACCOUNT => Section::Account,
                MENU_SETTINGS_ABOUT => Section::About,
                _ => Section::General,
            };
            with_app(|app| settings::open(app, section));
        }
        MENU_FIRST_RUN => {
            with_app(settings::open_first_run);
        }
        MENU_TEST_NOTICES => {
            with_app(notices::test_samples);
        }
        MENU_TEST_QUIET => {
            with_app(notices::test_quiet);
        }
        MENU_TEST_FULL_SCREEN => {
            with_app(notices::test_full_screen);
        }
        MENU_DISK_SCAN_CLIP => {
            with_app(disk::scan_clipboard);
        }
        MENU_DISK_LOW_8 | MENU_DISK_LOW_2 => {
            let free = if id == MENU_DISK_LOW_8 { 8.0 } else { 2.0 };
            with_app(|app| disk::test_low_space(app, free, id == MENU_DISK_LOW_8));
        }
        MENU_DISK_CHECK => {
            with_app(disk::check_space);
        }
        MENU_DISK_FIND_C => {
            with_app(|app| disk::find_big_folders(app, 'C'));
        }
        MENU_CAL_LINK => {
            with_app(calendar::add_link_from_clipboard);
        }
        MENU_CAL_READ => {
            with_app(calendar::read_now);
        }
        MENU_CAL_REMOVE => {
            with_app(calendar::remove_link);
        }
        MENU_CAL_SOON | MENU_CAL_START => {
            let cue = if id == MENU_CAL_SOON {
                Cue::Soon
            } else {
                Cue::Start
            };
            with_app(|app| calendar::test_cue(app, cue));
        }
        MENU_DRINK_DUE => {
            with_app(habits::test_drink_due);
        }
        MENU_FOCUS_1 => {
            with_app(|app| habits::start_focus(app, 1));
        }
        MENU_TEST_10 | MENU_TEST_AT => {
            let off = if id == MENU_TEST_10 { -10 } else { 0 };
            with_app(|app| prayer::test_reminder(app, off));
        }
        MENU_TOGGLE => {
            with_app(|app| {
                let v = !app.visible;
                orb::set_visible(app, v);
                notices::on_user_visibility(app);
            });
        }
        MENU_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}
