//! The few macOS calls gpui doesn't make (sprint 2, W1): the Dock badge,
//! handing over to the Pigeon already running, the login item, and the
//! display options and colors the theme follows. Everything here runs on
//! the main thread; elsewhere it does nothing.

pub mod menu_bar;

use std::cell::RefCell;

use objc2::{
    DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, NSObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationDidBecomeActiveNotification,
    NSColor, NSColorSpace, NSHapticFeedbackManager, NSHapticFeedbackPattern,
    NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer as _, NSRunningApplication,
    NSSystemColorsDidChangeNotification, NSWorkspace,
    NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_core_foundation::CFString;
#[allow(deprecated)]
use objc2_core_services::{LSCopyDefaultHandlerForURLScheme, LSSetDefaultHandlerForURLScheme};
use objc2_foundation::{
    NSBundle, NSCurrentLocaleDidChangeNotification, NSDateFormatter,
    NSDistributedNotificationCenter, NSError, NSLocale, NSNotification, NSNotificationCenter,
    NSNotificationSuspensionBehavior, NSString, NSSystemTimeZoneDidChangeNotification, NSURL,
    NSUserDefaults,
};

/// What the system tells Pigeon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Pigeon was opened again on the same data folder (PER-5).
    ShowRequested,
    /// The user's region or clock settings changed.
    LocaleChanged,
    /// The Mac's time zone changed.
    TimeZoneChanged,
    /// The accessibility display options or the system colors may have
    /// changed: on their notifications, and each time Pigeon comes forward.
    DisplayChanged,
}

/// The distributed notification a second Pigeon posts, with the data
/// folder as its object.
const SHOW: &str = "com.pigeon.dev.show";

type Handler = Box<dyn Fn(Signal)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    static OBSERVER: RefCell<Option<Retained<Observer>>> = const { RefCell::new(None) };
    /// The Quick Look panel's data source; the panel holds it weakly.
    static QUICK_LOOK: RefCell<Option<(std::path::PathBuf, Retained<QuickLookSource>)>> =
        const { RefCell::new(None) };
}

fn signal(signal: Signal) {
    HANDLER.with(|handler| {
        if let Some(handler) = &*handler.borrow() {
            handler(signal);
        }
    });
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and `Observer`
    // doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PigeonSignalObserver"]
    struct Observer;

    impl Observer {
        #[unsafe(method(showRequested:))]
        fn show_requested(&self, _notification: &NSNotification) {
            signal(Signal::ShowRequested);
        }

        #[unsafe(method(localeChanged:))]
        fn locale_changed(&self, _notification: &NSNotification) {
            signal(Signal::LocaleChanged);
        }

        #[unsafe(method(timeZoneChanged:))]
        fn time_zone_changed(&self, _notification: &NSNotification) {
            signal(Signal::TimeZoneChanged);
        }

        #[unsafe(method(displayChanged:))]
        fn display_changed(&self, _notification: &NSNotification) {
            signal(Signal::DisplayChanged);
        }
    }
);

impl Observer {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `NSObject`'s `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Call `handler` with each `Signal`; hand-over is from another Pigeon on
/// `data_dir`. Once, at launch.
pub fn listen(data_dir: &str, handler: impl Fn(Signal) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
    let observer = Observer::new(mtm);
    // SAFETY: the observer answers every selector here and lives in
    // `OBSERVER` for the rest of the process.
    unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserver_selector_name_object(
                &observer,
                sel!(displayChanged:),
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
            );
        // Delivered even while Pigeon is in the background, which AppKit
        // otherwise holds distributed notifications for.
        NSDistributedNotificationCenter::defaultCenter()
            .addObserver_selector_name_object_suspensionBehavior(
                &observer,
                sel!(showRequested:),
                Some(&NSString::from_str(SHOW)),
                Some(&NSString::from_str(data_dir)),
                NSNotificationSuspensionBehavior::DeliverImmediately,
            );
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &observer,
            sel!(localeChanged:),
            Some(NSCurrentLocaleDidChangeNotification),
            None,
        );
        let center = NSNotificationCenter::defaultCenter();
        center.addObserver_selector_name_object(
            &observer,
            sel!(timeZoneChanged:),
            Some(NSSystemTimeZoneDidChangeNotification),
            None,
        );
        for name in [
            NSSystemColorsDidChangeNotification,
            NSApplicationDidBecomeActiveNotification,
        ] {
            center.addObserver_selector_name_object(
                &observer,
                sel!(displayChanged:),
                Some(name),
                None,
            );
        }
    }
    OBSERVER.with(|o| *o.borrow_mut() = Some(observer));
}

/// Whether macOS shows times on a 24-hour clock: System Settings ›
/// General › Date & Time, which the region decides until the user does.
pub fn system_uses_24_hour() -> bool {
    let locale = NSLocale::autoupdatingCurrentLocale();
    NSDateFormatter::dateFormatFromTemplate_options_locale(
        &NSString::from_str("j"),
        0,
        Some(&locale),
    )
    .is_some_and(|format| uses_24_hour_pattern(&format.to_string()))
}

/// A date format pattern's hour: `H` and `k` are 24-hour. Quoted text
/// ("'h'") is literal.
fn uses_24_hour_pattern(pattern: &str) -> bool {
    pattern
        .split('\'')
        .step_by(2)
        .any(|unquoted| unquoted.contains(['H', 'k']))
}

/// System Settings › Accessibility › Display › Increase contrast.
pub fn increase_contrast() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast()
}

/// System Settings › Accessibility › Display › Reduce transparency.
pub fn reduce_transparency() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

/// System Settings › Accessibility › Display › Differentiate without color.
pub fn differentiate_without_color() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldDifferentiateWithoutColor()
}

/// The system accent color (System Settings › Appearance) in sRGB, or none
/// for Multicolor, where each app keeps its own.
pub fn accent_color() -> Option<gpui::Rgba> {
    // Multicolor is the absence of the default.
    NSUserDefaults::standardUserDefaults().objectForKey(&NSString::from_str("AppleAccentColor"))?;
    srgb(&NSColor::controlAccentColor())
}

/// The highlight color behind selected text, in sRGB.
pub fn selection_color() -> Option<gpui::Rgba> {
    srgb(&NSColor::selectedTextBackgroundColor())
}

fn srgb(color: &NSColor) -> Option<gpui::Rgba> {
    let color = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(gpui::Rgba {
        r: color.redComponent() as f32,
        g: color.greenComponent() as f32,
        b: color.blueComponent() as f32,
        a: color.alphaComponent() as f32,
    })
}

/// The standard About panel, with the bundle's name, version and
/// `NSHumanReadableCopyright`.
pub fn show_about() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
}

/// Window › Bring All to Front.
pub fn bring_all_to_front() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).arrangeInFront(None);
}

/// A light tap on a Force Touch trackpad, as something snaps into place.
pub fn haptic_alignment() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
        NSHapticFeedbackPattern::Alignment,
        NSHapticFeedbackPerformanceTime::Default,
    );
}

/// From a second Pigeon: ask the one running on `data_dir` to show its
/// window, and bring it forward.
pub fn hand_over(data_dir: &str, pid: Option<i32>) {
    // SAFETY: no user info.
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(SHOW),
                Some(&NSString::from_str(data_dir)),
                None,
                true,
            );
    }
    if let Some(app) = pid.and_then(NSRunningApplication::runningApplicationWithProcessIdentifier) {
        app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
}

/// The Dock icon's badge: the count, or none at zero (NTF-2).
pub fn set_dock_badge(count: u64) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

// Quick Look (MB-044): `QLPreviewPanel`, through the runtime. No crate for
// QuickLookUI is in the lock file; Quartz, which holds it, is linked so the
// class is there. Pigeon sets the panel's data source without taking
// control of the panel through the responder chain, which gpui's window
// doesn't offer; AppKit logs a warning for that and shows the panel.
// Closing it (Space, Esc) gives the keys back to the window under it.

#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and
    // `QuickLookSource` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PigeonQuickLookSource"]
    #[ivars = Retained<NSURL>]
    struct QuickLookSource;

    impl QuickLookSource {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: &AnyObject) -> isize {
            1
        }

        /// The item is the file's URL, kept alive by the source.
        #[unsafe(method(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: &AnyObject, _index: isize) -> *mut NSURL {
            Retained::as_ptr(self.ivars()).cast_mut()
        }
    }
);

impl QuickLookSource {
    fn new(mtm: MainThreadMarker, url: Retained<NSURL>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(url);
        // SAFETY: `NSObject`'s `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Show `path` in the Quick Look panel, or close the panel when it's
/// showing it already. The caller never passes something that could run
/// code.
pub fn quick_look(path: &std::path::Path) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(class) = AnyClass::get(c"QLPreviewPanel") else {
        return;
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    // SAFETY: `+sharedPreviewPanel` returns the app's panel; the messages
    // below are `QLPreviewPanel`'s, with the argument types it declares.
    unsafe {
        let panel: Option<Retained<AnyObject>> = msg_send![class, sharedPreviewPanel];
        let Some(panel) = panel else {
            return;
        };
        let visible: bool = msg_send![&panel, isVisible];
        let same = QUICK_LOOK.with(|q| q.borrow().as_ref().is_some_and(|(p, _)| p == path));
        if visible && same {
            let _: () = msg_send![&panel, orderOut: None::<&AnyObject>];
            return;
        }
        let source = QuickLookSource::new(mtm, url);
        let _: () = msg_send![&panel, setDataSource: &*source];
        let _: () = msg_send![&panel, reloadData];
        let _: () = msg_send![&panel, makeKeyAndOrderFront: None::<&AnyObject>];
        QUICK_LOOK.with(|q| *q.borrow_mut() = Some((path.to_path_buf(), source)));
    }
}

// Open at login (NTF-6): `SMAppService.mainApp`, through the runtime. No
// crate for ServiceManagement is in the lock file; the framework is linked
// so the class is there.

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// The login item as macOS has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginItem {
    /// Not offered here, and why.
    Unavailable(&'static str),
    Off,
    On,
    /// Registered, but the user has to allow it in System Settings.
    NeedsApproval,
}

/// Offered only to the installed copy, and never in test-instance mode.
pub fn login_item_offered() -> Result<(), &'static str> {
    if crate::instance::dev_mode() {
        return Err("Not in test-instance mode (PIGEON_DEV).");
    }
    let installed = std::env::current_exe().is_ok_and(|exe| {
        exe.starts_with("/Applications/") && exe.to_string_lossy().contains(".app/")
    });
    if installed {
        Ok(())
    } else {
        Err("Available once Pigeon runs from the Applications folder.")
    }
}

fn main_app_service() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"SMAppService")?;
    // SAFETY: `+[SMAppService mainAppService]` returns the app's service.
    unsafe { msg_send![class, mainAppService] }
}

pub fn login_item() -> LoginItem {
    if let Err(why) = login_item_offered() {
        return LoginItem::Unavailable(why);
    }
    let Some(service) = main_app_service() else {
        return LoginItem::Unavailable("Needs macOS 13 or later.");
    };
    // SAFETY: `status` is an `NSInteger` property.
    let status: isize = unsafe { msg_send![&service, status] };
    match status {
        1 => LoginItem::On,
        2 => LoginItem::NeedsApproval,
        _ => LoginItem::Off,
    }
}

/// Register or unregister the login item; why it failed, in macOS's words.
pub fn set_login_item(on: bool) -> Result<(), String> {
    login_item_offered().map_err(String::from)?;
    let service = main_app_service().ok_or("Needs macOS 13 or later.")?;
    // SAFETY: both take an `NSError **` and return `BOOL`.
    let result: Result<(), Retained<NSError>> = unsafe {
        if on {
            msg_send![&service, registerAndReturnError: _]
        } else {
            msg_send![&service, unregisterAndReturnError: _]
        }
    };
    result.map_err(|e| e.localizedDescription().to_string())
}

/// System Settings › General › Login Items, to allow it.
pub fn open_login_items_settings() {
    if let Some(class) = AnyClass::get(c"SMAppService") {
        // SAFETY: a class method taking nothing.
        let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
    }
}

// W3 (LNK-3): the default mail app, through Launch Services. Deprecated but
// synchronous; macOS may ask the user to confirm the change.

/// Who opens `mailto:` links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailHandler {
    /// Not offered here, and why.
    Unavailable(&'static str),
    Pigeon,
    /// Another app, by bundle id; empty when none is set.
    Other(String),
}

/// This app's bundle id, when it runs as an app.
fn bundle_id() -> Option<String> {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}

#[allow(deprecated)]
pub fn mail_handler() -> MailHandler {
    let Some(me) = bundle_id() else {
        return MailHandler::Unavailable("Available once Pigeon runs as an app.");
    };
    // SAFETY: a CFString in, an owned CFString (or none) out.
    let current = unsafe { LSCopyDefaultHandlerForURLScheme(&CFString::from_str("mailto")) }
        .map(|id| id.to_string())
        .unwrap_or_default();
    if current.eq_ignore_ascii_case(&me) {
        MailHandler::Pigeon
    } else {
        MailHandler::Other(current)
    }
}

/// Make Pigeon open `mailto:` links; Launch Services' error code if not.
#[allow(deprecated)]
pub fn make_default_mail_app() -> Result<(), String> {
    let me = bundle_id().ok_or("Pigeon isn’t running as an app.")?;
    // SAFETY: both are CFStrings.
    let status = unsafe {
        LSSetDefaultHandlerForURLScheme(&CFString::from_str("mailto"), &CFString::from_str(&me))
    };
    if status == 0 {
        Ok(())
    } else {
        eprintln!("Pigeon: LSSetDefaultHandlerForURLScheme: {status}");
        Err("macOS didn't make Pigeon the default. Try again, or choose it in Mail › Settings › General.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::uses_24_hour_pattern;

    #[test]
    fn reads_the_hour_cycle_from_a_pattern() {
        assert!(!uses_24_hour_pattern("h a"));
        assert!(uses_24_hour_pattern("HH"));
        assert!(uses_24_hour_pattern("k"));
        assert!(!uses_24_hour_pattern("'H'h a"));
    }
}
