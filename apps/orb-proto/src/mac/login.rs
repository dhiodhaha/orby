//! Start at login through `SMAppService.mainAppService` (macOS 13+): the app itself as a login
//! item, listed in System Settings, General, Login Items, where you can switch it off again.
//! Off until you turn it on from the menu bar.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {}

/// SMAppServiceStatus.
const ENABLED: isize = 1;
const REQUIRES_APPROVAL: isize = 2;

fn service() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    // SAFETY: +mainAppService returns the app's own service object (autoreleased).
    unsafe { msg_send![cls, mainAppService] }
}

fn status() -> isize {
    service().map_or(0, |s| unsafe { msg_send![&*s, status] })
}

pub fn enabled() -> bool {
    status() == ENABLED
}

/// Registered, but waiting for you to allow it in Login Items.
pub fn needs_approval() -> bool {
    status() == REQUIRES_APPROVAL
}

/// Registers or unregisters the app as a login item.
pub fn set(on: bool) -> Result<(), String> {
    let s = service().ok_or("ServiceManagement is not available")?;
    let mut err: *mut AnyObject = std::ptr::null_mut();
    // SAFETY: both methods take an NSError out pointer and return BOOL.
    let ok: bool = unsafe {
        if on {
            msg_send![&*s, registerAndReturnError: &mut err]
        } else {
            msg_send![&*s, unregisterAndReturnError: &mut err]
        }
    };
    if ok {
        Ok(())
    } else if err.is_null() {
        Err("refused".into())
    } else {
        // SAFETY: on failure the out pointer is an autoreleased NSError.
        let d: Option<Retained<AnyObject>> = unsafe { msg_send![&*err, localizedDescription] };
        Err(d.map_or_else(|| "refused".into(), |d| format!("{d:?}")))
    }
}
