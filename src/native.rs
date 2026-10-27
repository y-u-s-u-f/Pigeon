//! The few macOS calls gpui doesn't make (sprint 2, W1): the Dock badge,
//! waking from sleep, handing over to the Pigeon already running, and the
//! login item. Everything here runs on the main thread; elsewhere it does
//! nothing.

use std::cell::RefCell;

use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, NSObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
    NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSError, NSNotification, NSNotificationSuspensionBehavior,
    NSString,
};

/// What the system tells Pigeon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The Mac woke from sleep.
    Wake,
    /// Pigeon was opened again on the same data folder (PER-5).
    ShowRequested,
}

/// The distributed notification a second Pigeon posts, with the data
/// folder as its object.
const SHOW: &str = "com.pigeon.dev.show";

type Handler = Box<dyn Fn(Signal)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    static OBSERVER: RefCell<Option<Retained<Observer>>> = const { RefCell::new(None) };
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
        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            signal(Signal::Wake);
        }

        #[unsafe(method(showRequested:))]
        fn show_requested(&self, _notification: &NSNotification) {
            signal(Signal::ShowRequested);
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

/// Call `handler` on wake, and when another Pigeon on `data_dir` hands
/// over. Once, at launch.
pub fn listen(data_dir: &str, handler: impl Fn(Signal) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
    let observer = Observer::new(mtm);
    // SAFETY: the observer answers both selectors and lives in `OBSERVER`
    // for the rest of the process.
    unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserver_selector_name_object(
                &observer,
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
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
    }
    OBSERVER.with(|o| *o.borrow_mut() = Some(observer));
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
