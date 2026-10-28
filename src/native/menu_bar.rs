//! The menu bar's native side (sprint 2 W7b, NTF-4, NTF-5): the status
//! item, the panel's `NSWindow` (shown, hidden, placed and sized here, as
//! gpui has no per-window hide), click-away, and global hot keys through
//! Carbon's `RegisterEventHotKey` (no Accessibility permission needed).
//!
//! The panel opens with a short fade and a few points of slide down from the
//! menu bar (`NSAnimationContext`), and closes at once. It can become key
//! (the keys in it work) but never the main window, so a Pigeon window that
//! was main keeps its active look while the panel is up.
//!
//! Main thread only. Showing, hiding, focusing and resizing the panel make
//! AppKit call back into gpui at once (`windowDidBecomeKey:`, a resize),
//! which borrows the `App`: never call those from inside a gpui update.
//! `app::menu_bar` runs them from a foreground task.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    ptr::NonNull,
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, NSObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSAnimatablePropertyContainer as _, NSAnimationContext, NSApplication, NSBezierPath,
    NSCellImagePosition, NSColor, NSEvent, NSEventMask, NSFont, NSFontWeightRegular, NSImage,
    NSScreen, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSView, NSWindow,
    NSWindowAnimationBehavior,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// What the system tells the menu bar.
pub enum Signal {
    /// The status item was pressed, in the menu bar of this screen.
    Clicked(Anchor),
    /// A click in another app or on the desktop.
    ClickAway,
    /// A registered hot key, by its id.
    HotKey(u32),
}

/// The status-bar window that was clicked: the item has a copy in each
/// screen's menu bar.
#[derive(Clone)]
pub struct Anchor(Option<Retained<NSWindow>>);

/// Between the menu bar and the panel's top edge, in points.
const MENU_BAR_GAP: f64 = 6.;
/// The panel keeps this far from the screen's edges.
const SCREEN_MARGIN: f64 = 8.;
/// The panel's opacity while it's drawn at launch: enough for macOS to
/// count it as visible (at 0 it's occluded and gpui doesn't draw), too
/// little to see.
const WARM_UP_ALPHA: f64 = 0.01;
/// How far the panel slides down into place as it opens, in points.
const OPEN_SLIDE: f64 = 6.;
/// The new-mail dot, in points.
const DOT: f64 = 5.;

struct State {
    item: Option<Retained<NSStatusItem>>,
    target: Retained<ClickTarget>,
    panel: Option<Retained<NSWindow>>,
    /// Drawn at launch, not shown yet.
    warming: bool,
    /// Until when the panel is opening, and the frame it opens to.
    opening: Option<(Instant, NSRect)>,
    glyph: Option<Retained<NSImage>>,
    glyph_dot: Option<Retained<NSImage>>,
    dot: bool,
    title: Option<String>,
    hot_keys: HashMap<u32, usize>,
    hot_key_handler: bool,
    _monitor: Option<Retained<AnyObject>>,
}

type Handler = Box<dyn Fn(Signal)>;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

fn signal(signal: Signal) {
    HANDLER.with_borrow(|handler| {
        if let Some(handler) = handler {
            handler(signal);
        }
    });
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with_borrow_mut(|state| state.as_mut().map(f))
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and `ClickTarget`
    // doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PigeonStatusItemTarget"]
    struct ClickTarget;

    impl ClickTarget {
        #[unsafe(method(statusItemClicked:))]
        fn status_item_clicked(&self, _sender: Option<&AnyObject>) {
            let mtm = self.mtm();
            let clicked = NSApplication::sharedApplication(mtm)
                .currentEvent()
                .and_then(|event| event.window(mtm));
            signal(Signal::Clicked(Anchor(clicked)));
        }
    }
);

impl ClickTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `NSObject`'s `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Call `handler` for clicks and hot keys, and watch for clicks away.
/// Once, at launch.
pub fn listen(handler: impl Fn(Signal) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    HANDLER.with_borrow_mut(|h| *h = Some(Box::new(handler)));
    // Clicks in other apps never reach Pigeon; a global monitor sees them
    // without Accessibility permission (only key events need it).
    let block = RcBlock::new(|_: NonNull<NSEvent>| signal(Signal::ClickAway));
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
        &block,
    );
    let (glyph, glyph_dot) = glyphs();
    STATE.with_borrow_mut(|state| {
        *state = Some(State {
            item: None,
            target: ClickTarget::new(mtm),
            panel: None,
            warming: false,
            opening: None,
            glyph,
            glyph_dot,
            dot: false,
            title: None,
            hot_keys: HashMap::new(),
            hot_key_handler: false,
            _monitor: monitor,
        })
    });
}

/// The bird, as a template image so it follows the menu bar's look, and
/// the same with the new-mail dot.
fn glyphs() -> (Option<Retained<NSImage>>, Option<Retained<NSImage>>) {
    let Some(bird) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("bird"),
        Some(&NSString::from_str("Pigeon")),
    ) else {
        return (None, None);
    };
    bird.setTemplate(true);
    let size = bird.size();
    let canvas = NSSize::new(size.width + DOT - 1., size.height);
    let inner = bird.clone();
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        inner.drawInRect(NSRect::new(NSPoint::new(0., 0.), size));
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
            NSPoint::new(canvas.width - DOT, canvas.height - DOT),
            NSSize::new(DOT, DOT),
        ))
        .fill();
        Bool::YES
    });
    let dotted = NSImage::imageWithSize_flipped_drawingHandler(canvas, false, &draw);
    dotted.setTemplate(true);
    dotted.setAccessibilityDescription(Some(&NSString::from_str("Pigeon, new mail")));
    (Some(bird), Some(dotted))
}

/// Show the status item, or take it out of the menu bar.
pub fn set_status_item(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    with_state(|state| match (&state.item, visible) {
        (None, true) => {
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            if let Some(button) = item.button(mtm) {
                // SAFETY: the target lives in `STATE` as long as the item,
                // and answers `statusItemClicked:`.
                unsafe {
                    button.setTarget(Some(&state.target));
                    button.setAction(Some(sel!(statusItemClicked:)));
                }
                // Open on mouse down, as the system's menu extras do.
                button.sendActionOn(NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown);
                let size = NSFont::menuBarFontOfSize(0.).pointSize();
                button.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
                    size,
                    // SAFETY: a constant AppKit exports.
                    unsafe { NSFontWeightRegular },
                )));
            }
            state.item = Some(item);
            apply_status(state, mtm);
        }
        (Some(item), false) => {
            NSStatusBar::systemStatusBar().removeStatusItem(item);
            state.item = None;
        }
        _ => {}
    });
}

/// The text beside the icon (none: the icon alone), and the new-mail dot.
pub fn set_status(title: Option<&str>, dot: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    with_state(|state| {
        let title = title.map(str::to_string);
        if state.title == title && state.dot == dot {
            return;
        }
        state.title = title;
        state.dot = dot;
        apply_status(state, mtm);
    });
}

fn apply_status(state: &State, mtm: MainThreadMarker) {
    let Some(button) = state.item.as_ref().and_then(|item| item.button(mtm)) else {
        return;
    };
    let image = if state.dot {
        &state.glyph_dot
    } else {
        &state.glyph
    };
    match image {
        Some(image) => button.setImage(Some(image)),
        None => button.setImage(None),
    }
    let title = state.title.as_deref().unwrap_or_default();
    // No image (the symbol is missing): a letter stands in.
    let title = match (image, title) {
        (None, "") => "P".to_string(),
        (_, "") => String::new(),
        (_, title) => format!(" {title}"),
    };
    button.setTitle(&NSString::from_str(&title));
    button.setImagePosition(if title.is_empty() {
        NSCellImagePosition::ImageOnly
    } else {
        NSCellImagePosition::ImageLeading
    });
    let tip = match (&state.title, state.dot) {
        (Some(title), true) => format!("Pigeon: new mail · {}", title.trim()),
        (Some(title), false) => format!("Pigeon · {}", title.trim()),
        (None, true) => "Pigeon: new mail".into(),
        (None, false) => "Pigeon".into(),
    };
    button.setToolTip(Some(&NSString::from_str(&tip)));
}

/// The item drawn pressed while the panel shows.
pub fn set_highlight(on: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    with_state(|state| {
        if let Some(button) = state.item.as_ref().and_then(|item| item.button(mtm)) {
            button.highlight(on);
        }
    });
}

/// Take over the panel's `NSWindow`, from inside its gpui window's
/// construction. False when there's none.
pub fn adopt_panel(window: &gpui::Window) -> bool {
    let Some(ns_window) = ns_window_of(window) else {
        return false;
    };
    // gpui gives pop-ups the utility "zoom in"; the panel animates itself.
    ns_window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    never_main(&ns_window);
    with_state(|state| state.panel = Some(ns_window)).is_some()
}

extern "C" fn cannot_become_main(_: &AnyObject, _: Sel) -> Bool {
    Bool::NO
}

/// gpui's panels can become the main window; this one can only be key, so
/// the main window under it keeps its active look (traffic lights) while
/// the panel takes the keys. A subclass of the panel's class, swapped in.
fn never_main(window: &NSWindow) {
    let object: &AnyObject = window;
    let name = c"PigeonMenuBarPanel";
    let class = AnyClass::get(name).or_else(|| {
        let mut builder = ClassBuilder::new(name, object.class())?;
        // SAFETY: the signature matches `-[NSWindow canBecomeMainWindow]`.
        unsafe {
            builder.add_method(
                sel!(canBecomeMainWindow),
                cannot_become_main as extern "C" fn(_, _) -> _,
            );
        }
        Some(builder.register())
    });
    if let Some(class) = class {
        // SAFETY: a subclass with no new ivars, overriding one method.
        unsafe { AnyObject::set_class(object, class) };
    }
}

fn panel() -> Option<Retained<NSWindow>> {
    with_state(|state| state.panel.clone()).flatten()
}

/// Showing, and not just being drawn at launch.
pub fn panel_visible() -> bool {
    panel().is_some_and(|p| p.isVisible()) && with_state(|state| !state.warming) == Some(true)
}

/// Put the panel under the item clicked, or for a hot key under the item
/// when it's on the screen with the pointer, else at the top of that
/// screen, right of centre. Then show it and make it key: fading in and
/// sliding down over `animate`, or at once with `None`.
pub fn show_panel(anchor: Option<&Anchor>, animate: Option<Duration>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(panel) = panel() else {
        return;
    };
    place(&panel, anchor, mtm);
    let rest = panel.frame();
    let opening = animate.map(|duration| (Instant::now() + duration, rest));
    with_state(|state| {
        state.warming = false;
        state.opening = opening;
    });
    panel.setIgnoresMouseEvents(false);
    match animate {
        Some(duration) => {
            // Almost see-through and a little high when it's ordered
            // front (gpui draws it then), so no frame of it shows at rest.
            panel.setAlphaValue(WARM_UP_ALPHA);
            let start = NSRect::new(
                NSPoint::new(rest.origin.x, rest.origin.y + OPEN_SLIDE),
                rest.size,
            );
            panel.setFrame_display(start, false);
            panel.makeKeyAndOrderFront(None);
            animate_to(&panel, rest, duration.as_secs_f64());
        }
        None => {
            panel.setAlphaValue(1.);
            panel.makeKeyAndOrderFront(None);
        }
    }
    panel.invalidateShadow();
    set_highlight(true);
}

/// Animate the panel's opacity and frame, with an ease-out.
fn animate_to(panel: &NSWindow, frame: NSRect, seconds: f64) {
    NSAnimationContext::beginGrouping();
    let context = NSAnimationContext::currentContext();
    context.setDuration(seconds);
    if let Some(class) = AnyClass::get(c"CAMediaTimingFunction") {
        let name = NSString::from_str("easeOut");
        // SAFETY: `+functionWithName:` takes a name and returns the
        // function (or nil); the context takes it as `timingFunction`.
        unsafe {
            let function: Option<Retained<AnyObject>> = msg_send![class, functionWithName: &*name];
            if let Some(function) = function {
                let _: () = msg_send![&*context, setTimingFunction: &*function];
            }
        }
    }
    let animator = panel.animator();
    animator.setAlphaValue(1.);
    animator.setFrame_display(frame, true);
    NSAnimationContext::endGrouping();
}

pub fn hide_panel() {
    if let Some(panel) = panel().filter(|p| p.isVisible()) {
        // An opening still under way ends where it was going, so the next
        // open starts clean.
        if let Some((_, rest)) = with_state(|state| state.opening.take()).flatten() {
            animate_to(&panel, rest, 0.);
        }
        panel.orderOut(None);
        // The window under it gets the keys back.
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            if app.isActive()
                && let Some(main) = app.mainWindow()
            {
                main.makeKeyWindow();
            }
        }
    }
    set_highlight(false);
}

/// Show the panel once at launch, see-through and not taking clicks, so
/// the first real open doesn't pay for drawing it, and is a second
/// activation: gpui draws a window synchronously when it becomes key again,
/// but waits for its display link the first time. `finish_warm_up` hides
/// it again.
pub fn warm_up_panel() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(panel) = panel() {
        place(&panel, None, mtm);
        with_state(|state| state.warming = true);
        panel.setAlphaValue(WARM_UP_ALPHA);
        panel.setIgnoresMouseEvents(true);
        panel.makeKeyAndOrderFront(None);
    }
}

pub fn finish_warm_up() {
    if with_state(|state| std::mem::take(&mut state.warming)) != Some(true) {
        return;
    }
    if let Some(panel) = panel() {
        panel.orderOut(None);
        panel.setAlphaValue(1.);
        panel.setIgnoresMouseEvents(false);
    }
}

/// The panel's height in points, its top edge kept where it is.
pub fn resize_panel(height: f64) {
    let Some(panel) = panel() else {
        return;
    };
    let frame = panel.frame();
    if (frame.size.height - height).abs() < 0.5 {
        return;
    }
    // Mid-open the frame is a little high: the top that counts is where
    // it's going.
    let opening =
        with_state(|state| state.opening.filter(|(until, _)| Instant::now() < *until)).flatten();
    let top = match opening {
        Some((_, rest)) => rest.origin.y + rest.size.height,
        None => frame.origin.y + frame.size.height,
    };
    let frame = NSRect::new(
        NSPoint::new(frame.origin.x, top - height),
        NSSize::new(frame.size.width, height),
    );
    if opening.is_some() {
        // Replaces the slide under way: jumps the few points left.
        animate_to(&panel, frame, 0.);
        with_state(|state| {
            if let Some((_, rest)) = &mut state.opening {
                *rest = frame;
            }
        });
    } else {
        panel.setFrame_display(frame, true);
    }
    panel.invalidateShadow();
}

fn place(panel: &NSWindow, anchor: Option<&Anchor>, mtm: MainThreadMarker) {
    let pointer = screen_with_pointer(mtm);
    let item_window = anchor.and_then(|a| a.0.clone()).or_else(|| {
        let window = with_state(|state| {
            state
                .item
                .as_ref()
                .and_then(|item| item.button(mtm))
                .and_then(|button| button.window())
        })
        .flatten()?;
        let same = match (window.screen(), &pointer) {
            (Some(a), Some(b)) => same_rect(a.frame(), b.frame()),
            _ => false,
        };
        same.then_some(window)
    });
    let width = panel.frame().size.width;
    let (screen, centre, top) = match &item_window {
        Some(window) => {
            let anchor = window.frame();
            (
                window.screen().or(pointer),
                anchor.origin.x + anchor.size.width / 2.,
                anchor.origin.y - MENU_BAR_GAP,
            )
        }
        None => {
            let Some(screen) = pointer.or_else(|| NSScreen::mainScreen(mtm)) else {
                return;
            };
            let visible = screen.visibleFrame();
            (
                Some(screen),
                visible.origin.x + visible.size.width * 0.75,
                visible.origin.y + visible.size.height - MENU_BAR_GAP,
            )
        }
    };
    let Some(screen) = screen.or_else(|| NSScreen::mainScreen(mtm)) else {
        return;
    };
    let visible = screen.visibleFrame();
    let min_x = visible.origin.x + SCREEN_MARGIN;
    let max_x = (visible.origin.x + visible.size.width - width - SCREEN_MARGIN).max(min_x);
    let x = (centre - width / 2.).clamp(min_x, max_x);
    // Cocoa's y grows upward: the panel's top sits just below the bar.
    panel.setFrameTopLeftPoint(NSPoint::new(x, top));
}

fn screen_with_pointer(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
    let at = NSEvent::mouseLocation();
    NSScreen::screens(mtm).iter().find(|screen| {
        let f = screen.frame();
        at.x >= f.origin.x
            && at.x <= f.origin.x + f.size.width
            && at.y >= f.origin.y
            && at.y <= f.origin.y + f.size.height
    })
}

fn same_rect(a: NSRect, b: NSRect) -> bool {
    a.origin.x == b.origin.x
        && a.origin.y == b.origin.y
        && a.size.width == b.size.width
        && a.size.height == b.size.height
}

/// The `NSWindow` behind a gpui window (gpui hands out its `NSView`).
fn ns_window_of(window: &gpui::Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: gpui's AppKit handle is its live content view, on the main
    // thread.
    let view: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

// Global hot keys, through Carbon.

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type CarbonHandler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetEventDispatcherTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: CarbonHandler,
        num_types: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
}

const SIGNATURE: u32 = u32::from_be_bytes(*b"PGN1");
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
const CMD_KEY: u32 = 1 << 8;
const OPTION_KEY: u32 = 1 << 11;
const CONTROL_KEY: u32 = 1 << 12;
/// What Carbon answers when another app has the shortcut.
pub const HOT_KEY_TAKEN: i32 = -9878;

extern "C" fn on_hot_key(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
    let mut id = EventHotKeyId::default();
    // SAFETY: a hot-key event carries its id as the direct object.
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyId>(),
            std::ptr::null_mut(),
            (&mut id as *mut EventHotKeyId).cast(),
        )
    };
    if status == 0 && id.signature == SIGNATURE {
        signal(Signal::HotKey(id.id));
    }
    0
}

/// Register ⌃⌥⌘ + `key_code` as hot key `id`, replacing what `id` had.
/// The error is Carbon's status (`HOT_KEY_TAKEN`: another app has it).
pub fn register_hot_key(id: u32, key_code: u32) -> Result<(), i32> {
    unregister_hot_key(id);
    with_state(|state| {
        // SAFETY: plain Carbon calls on the main thread; the handler is a
        // static fn, and the refs are kept until unregistered.
        unsafe {
            let target = GetEventDispatcherTarget();
            if !state.hot_key_handler {
                let spec = EventTypeSpec {
                    event_class: K_EVENT_CLASS_KEYBOARD,
                    event_kind: K_EVENT_HOT_KEY_PRESSED,
                };
                let mut handler = std::ptr::null_mut();
                let status = InstallEventHandler(
                    target,
                    on_hot_key,
                    1,
                    &spec,
                    std::ptr::null_mut(),
                    &mut handler,
                );
                if status != 0 {
                    return Err(status);
                }
                state.hot_key_handler = true;
            }
            let mut hot_key = std::ptr::null_mut();
            let status = RegisterEventHotKey(
                key_code,
                CMD_KEY | OPTION_KEY | CONTROL_KEY,
                EventHotKeyId {
                    signature: SIGNATURE,
                    id,
                },
                target,
                0,
                &mut hot_key,
            );
            if status != 0 {
                return Err(status);
            }
            state.hot_keys.insert(id, hot_key as usize);
            Ok(())
        }
    })
    .unwrap_or(Err(-1))
}

pub fn unregister_hot_key(id: u32) {
    with_state(|state| {
        if let Some(hot_key) = state.hot_keys.remove(&id) {
            // SAFETY: a ref `RegisterEventHotKey` gave, not unregistered yet.
            unsafe { UnregisterEventHotKey(hot_key as *mut c_void) };
        }
    });
}
