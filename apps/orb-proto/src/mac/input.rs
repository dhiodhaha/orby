//! Typing on the Mac: an invisible first-responder view that speaks NSTextInputClient, so key
//! presses go through the input system (dead keys, Option-letter accents, the emoji and
//! character viewer, input methods) and come out as committed text or as editing commands.
//!
//! The shared units edit with Windows semantics (`textfield::TextField`, Windows virtual-key
//! numbers, Ctrl for words, Shift to extend), so this file translates: `moveWordLeft:` is
//! Ctrl+Left, `moveToBeginningOfLine:` (Cmd+Left) is Home, `deleteWordBackward:` (Option+Delete)
//! is Ctrl+Backspace, Cmd+A/C/V/X/Z arrive as Ctrl+A/C/V/X/Z. While an event is delivered,
//! `mods()` answers what the unit would read from GetKeyState on Windows.
//!
//! A window that types adds one `InputView` and makes it first responder; the events go to the
//! sink set with `set_sink` (the app routes them to whatever has the keyboard).

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSTextInputClient, NSView};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint, NSRange,
    NSRangePointer, NSRect, NSSize, NSString, NSUInteger,
};

// Windows virtual-key numbers, as the shared units use them.
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_DELETE: u16 = 0x2E;

/// The modifiers a Windows unit would read, for the event being delivered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// Ctrl on Windows: Cmd on the Mac for shortcuts, Option for word moves.
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Mac's Command key itself (for the few places that care).
    pub cmd: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// Committed text (one key, an accent, a paste from the character viewer, an IME commit).
    Text(String),
    /// Text being composed (an input method's marked text); empty when the composition ends.
    Marked(String),
    /// An editing or navigation key, Windows style: `vk` with `mods` (Ctrl = word / shortcut).
    Key { vk: u16, mods: Mods },
}

thread_local! {
    static SINK: Cell<Option<fn(Input)>> = const { Cell::new(None) };
    static MODS: Cell<Mods> = const { Cell::new(Mods { ctrl: false, shift: false, alt: false, cmd: false }) };
    static MARKED: RefCell<String> = const { RefCell::new(String::new()) };
    /// Where the caret is on screen (y up), for the input method's candidate window.
    static CARET: Cell<NSRect> = const {
        Cell::new(NSRect { origin: NSPoint { x: 0.0, y: 0.0 }, size: NSSize { width: 1.0, height: 16.0 } })
    };
}

/// Where typed input goes (the app's router).
pub fn set_sink(f: fn(Input)) {
    SINK.with(|s| s.set(Some(f)));
}

/// The modifiers of the event being delivered (GetKeyState's answer on Windows).
pub fn mods() -> Mods {
    MODS.with(|m| m.get())
}

/// The caret's rect in screen coordinates, so a candidate window opens next to it.
pub fn set_caret_rect(r: NSRect) {
    CARET.with(|c| c.set(r));
}

fn deliver(i: Input, m: Mods) {
    let Some(f) = SINK.with(|s| s.get()) else {
        return;
    };
    MODS.with(|c| c.set(m));
    f(i);
    MODS.with(|c| c.set(Mods::default()));
}

fn key(vk: u16, m: Mods) {
    deliver(Input::Key { vk, mods: m }, m);
}

fn flags_mods(f: NSEventModifierFlags) -> Mods {
    Mods {
        ctrl: f.contains(NSEventModifierFlags::Command),
        shift: f.contains(NSEventModifierFlags::Shift),
        alt: f.contains(NSEventModifierFlags::Option),
        cmd: f.contains(NSEventModifierFlags::Command),
    }
}

/// The shortcut letter of a Cmd chord as its Windows virtual key ('A'..'Z' are 0x41..).
fn chord_vk(chars: &str) -> Option<u16> {
    let c = chars.chars().next()?.to_ascii_uppercase();
    c.is_ascii_alphanumeric().then_some(c as u16)
}

/// An editing command from the key bindings (Mac selector) as a Windows key with modifiers.
fn command_key(s: Sel) -> Option<(u16, Mods)> {
    let plain = Mods::default();
    let shift = Mods {
        shift: true,
        ..plain
    };
    let word = Mods {
        ctrl: true,
        ..plain
    };
    let word_shift = Mods {
        ctrl: true,
        shift: true,
        ..plain
    };
    let name = s.name().to_str().ok()?;
    Some(match name {
        "deleteBackward:" | "deleteBackwardByDecomposingPreviousCharacter:" => (VK_BACK, plain),
        "deleteForward:" => (VK_DELETE, plain),
        "deleteWordBackward:" => (VK_BACK, word),
        "deleteWordForward:" => (VK_DELETE, word),
        "deleteToBeginningOfLine:" => (VK_BACK, Mods { alt: true, ..plain }),
        "moveLeft:" | "moveBackward:" => (VK_LEFT, plain),
        "moveRight:" | "moveForward:" => (VK_RIGHT, plain),
        "moveLeftAndModifySelection:" | "moveBackwardAndModifySelection:" => (VK_LEFT, shift),
        "moveRightAndModifySelection:" | "moveForwardAndModifySelection:" => (VK_RIGHT, shift),
        "moveWordLeft:" | "moveWordBackward:" => (VK_LEFT, word),
        "moveWordRight:" | "moveWordForward:" => (VK_RIGHT, word),
        "moveWordLeftAndModifySelection:" | "moveWordBackwardAndModifySelection:" => {
            (VK_LEFT, word_shift)
        }
        "moveWordRightAndModifySelection:" | "moveWordForwardAndModifySelection:" => {
            (VK_RIGHT, word_shift)
        }
        "moveToBeginningOfLine:"
        | "moveToLeftEndOfLine:"
        | "moveToBeginningOfParagraph:"
        | "moveToBeginningOfDocument:" => (VK_HOME, plain),
        "moveToEndOfLine:"
        | "moveToRightEndOfLine:"
        | "moveToEndOfParagraph:"
        | "moveToEndOfDocument:" => (VK_END, plain),
        "moveToBeginningOfLineAndModifySelection:"
        | "moveToLeftEndOfLineAndModifySelection:"
        | "moveToBeginningOfDocumentAndModifySelection:"
        | "moveParagraphBackwardAndModifySelection:" => (VK_HOME, shift),
        "moveToEndOfLineAndModifySelection:"
        | "moveToRightEndOfLineAndModifySelection:"
        | "moveToEndOfDocumentAndModifySelection:"
        | "moveParagraphForwardAndModifySelection:" => (VK_END, shift),
        "moveUp:" => (VK_UP, plain),
        "moveDown:" => (VK_DOWN, plain),
        "moveUpAndModifySelection:" => (VK_UP, shift),
        "moveDownAndModifySelection:" => (VK_DOWN, shift),
        "scrollPageUp:" | "pageUp:" => (VK_PRIOR, plain),
        "scrollPageDown:" | "pageDown:" => (VK_NEXT, plain),
        "scrollToBeginningOfDocument:" => (VK_HOME, word),
        "scrollToEndOfDocument:" => (VK_END, word),
        "insertNewline:" | "insertLineBreak:" | "insertNewlineIgnoringFieldEditor:" => {
            (VK_RETURN, plain)
        }
        "insertTab:" => (VK_TAB, plain),
        "insertBacktab:" => (VK_TAB, shift),
        "cancelOperation:" => (VK_ESCAPE, plain),
        "selectAll:" => (0x41, word),
        _ => return None,
    })
}

fn ns_string_of(obj: &AnyObject) -> String {
    // NSString or NSAttributedString.
    let is_attr: bool =
        unsafe { msg_send![obj, isKindOfClass: <NSAttributedString as objc2::ClassType>::class()] };
    if is_attr {
        let s: Retained<NSString> = unsafe { msg_send![obj, string] };
        s.to_string()
    } else {
        let s: &NSString = unsafe { &*(obj as *const AnyObject as *const NSString) };
        s.to_string()
    }
}

pub struct InputIvars {
    _n: Cell<u8>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyInputView"]
    #[ivars = InputIvars]
    pub struct InputView;

    impl InputView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// Never in the way of clicks: the window's own view under it takes them.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _p: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            let m = flags_mods(e.modifierFlags());
            let chars = e
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let has_marked = MARKED.with(|s| !s.borrow().is_empty());
            // Cmd chords are shortcuts (Cmd+A/C/V/X/Z/W/Q...), never text.
            if m.cmd && !has_marked {
                match e.keyCode() {
                    123 => key(VK_HOME, Mods { ctrl: false, ..m }),
                    124 => key(VK_END, Mods { ctrl: false, ..m }),
                    126 => key(VK_HOME, m),
                    125 => key(VK_END, m),
                    51 => key(VK_BACK, Mods { ctrl: false, alt: true, ..m }),
                    _ => {
                        if let Some(vk) = chord_vk(&chars) {
                            key(vk, m);
                        }
                    }
                }
                return;
            }
            // Esc always reaches the app, even mid-composition handling is the input method's.
            if e.keyCode() == 53 && !has_marked {
                key(VK_ESCAPE, m);
                return;
            }
            // Option+Up and Option+Down are Windows' Alt+Up and Alt+Down (the Tasks window
            // moves a task), not the text system's paragraph moves.
            if m.alt && !has_marked && matches!(e.keyCode(), 125 | 126) {
                key(if e.keyCode() == 126 { VK_UP } else { VK_DOWN }, m);
                return;
            }
            // F1 to F35 have no text: the ones the units read arrive as their keys (F2 renames
            // a task), the others are dropped instead of typing a private-use character.
            let fkey = chars
                .chars()
                .next()
                .is_some_and(|c| ('\u{F704}'..='\u{F726}').contains(&c));
            if fkey && !has_marked {
                if let Some(vk) = crate::sys::vk_from_mac(e.keyCode()) {
                    key(vk, m);
                }
                return;
            }
            let events = NSArray::from_slice(&[e]);
            self.interpretKeyEvents(&events);
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, e: &NSEvent) -> bool {
            // Cmd chords come to keyDown above while this view has the keyboard.
            let me = self as *const Self as *const AnyObject;
            let is_key = self
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|r| Retained::as_ptr(&r) as *const AnyObject == me);
            let take = is_key && flags_mods(e.modifierFlags()).cmd;
            if take {
                let _: () = unsafe { msg_send![self, keyDown: e] };
            }
            take
        }
    }

    unsafe impl NSObjectProtocol for InputView {}

    unsafe impl NSTextInputClient for InputView {
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text(&self, string: &AnyObject, _r: NSRange) {
            let s = ns_string_of(string);
            let had = MARKED.with(|m| std::mem::take(&mut *m.borrow_mut()));
            if !had.is_empty() {
                deliver(Input::Marked(String::new()), Mods::default());
            }
            if !s.is_empty() {
                deliver(Input::Text(s), Mods::default());
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        unsafe fn do_command(&self, s: Sel) {
            if let Some((vk, m)) = command_key(s) {
                key(vk, m);
            }
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked_text(
            &self,
            string: &AnyObject,
            _sel: NSRange,
            _rep: NSRange,
        ) {
            let s = ns_string_of(string);
            MARKED.with(|m| *m.borrow_mut() = s.clone());
            deliver(Input::Marked(s), Mods::default());
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let s = MARKED.with(|m| std::mem::take(&mut *m.borrow_mut()));
            deliver(Input::Marked(String::new()), Mods::default());
            if !s.is_empty() {
                deliver(Input::Text(s), Mods::default());
            }
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(0, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let n = MARKED.with(|m| m.borrow().encode_utf16().count());
            if n == 0 {
                NSRange::new(NSNotFound as NSUInteger, 0)
            } else {
                NSRange::new(0, n)
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            MARKED.with(|m| !m.borrow().is_empty())
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributed_substring(
            &self,
            _range: NSRange,
            _actual: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect(
            &self,
            _range: NSRange,
            _actual: NSRangePointer,
        ) -> NSRect {
            CARET.with(|c| c.get())
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _p: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }
);

impl InputView {
    /// A zero-size view to add to a window's content view and make first responder.
    pub fn new(mtm: MainThreadMarker) -> Retained<InputView> {
        let this = Self::alloc(mtm).set_ivars(InputIvars { _n: Cell::new(0) });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Forgets a composition in progress (the field lost the keyboard).
    pub fn discard_marked(&self) {
        MARKED.with(|m| m.borrow_mut().clear());
        if let Some(ctx) = self.inputContext() {
            ctx.discardMarkedText();
        }
    }
}

/// Text on the general pasteboard, if any.
pub fn paste_text() -> Option<String> {
    let pb = objc2_app_kit::NSPasteboard::generalPasteboard();
    let s = pb.stringForType(unsafe { objc2_app_kit::NSPasteboardTypeString })?;
    Some(s.to_string())
}

/// Puts `s` on the general pasteboard.
pub fn copy_text(s: &str) {
    let pb = objc2_app_kit::NSPasteboard::generalPasteboard();
    pb.clearContents();
    let _ = pb.setString_forType(&NSString::from_str(s), unsafe {
        objc2_app_kit::NSPasteboardTypeString
    });
}

/// Plays a named system sound ("Glass", "Ping", "Tink", ...), if it exists.
pub fn play_sound(name: &str) {
    if let Some(s) = objc2_app_kit::NSSound::soundNamed(&NSString::from_str(name)) {
        let _ = s.play();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::sel;

    #[test]
    fn word_moves_map_to_ctrl_arrows() {
        assert_eq!(
            command_key(sel!(moveWordLeft:)),
            Some((
                VK_LEFT,
                Mods {
                    ctrl: true,
                    ..Mods::default()
                }
            ))
        );
        assert_eq!(
            command_key(sel!(deleteBackward:)),
            Some((VK_BACK, Mods::default()))
        );
        assert_eq!(command_key(sel!(noSuchCommand:)), None);
    }

    #[test]
    fn chords_name_their_letter() {
        assert_eq!(chord_vk("v"), Some(0x56));
        assert_eq!(chord_vk(""), None);
    }
}
