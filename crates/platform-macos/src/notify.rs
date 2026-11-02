//! `UNUserNotificationCenter` notifications with a click delegate (EXPORT-06, HOTKEY-03;
//! PITFALLS Pitfall 18). The delegate receives the response, decodes the action from the
//! request's `userInfo` and hands it to the callback the app installed; the app then
//! reveals the file or opens Options on the main thread.
//!
//! `UNUserNotificationCenter` aborts the process when the executable is not inside an
//! `.app` bundle (no bundle proxy), which is the case for `cargo run` / `cargo test`
//! binaries. [`MacNotifier::new`] detects that and degrades to a stderr line, so dev builds
//! never crash on the first copy (notifications are never fatal — `Notifier` contract).

use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSObject, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use snapshade_platform_api::{NotificationAction, Notifier, PlatformError};

/// Called with the decoded action when the user clicks a notification. Runs on whatever
/// thread UserNotifications delivers the response on (documented: the main thread).
pub type ClickHandler = Arc<dyn Fn(NotificationAction) + Send + Sync>;

const KEY_ACTION: &str = "action";
const KEY_PATH: &str = "path";
const ACTION_REVEAL: &str = "reveal";
const ACTION_OPTIONS: &str = "options";
const ACTION_URL: &str = "url";
const KEY_URL: &str = "url";

struct DelegateIvars {
    on_click: ClickHandler,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the struct has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "SnapshadeNotificationDelegate"]
    #[ivars = DelegateIvars]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Show the banner even while Snapshade counts as the foreground app (an overlay
            // panel can be key at copy time).
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let action = decode_action(&response.notification().request().content().userInfo());
            (self.ivars().on_click)(action);
            completion.call(());
        }
    }
);

impl NotificationDelegate {
    fn new(on_click: ClickHandler) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { on_click });
        // SAFETY: plain NSObject init on a freshly allocated, ivar-initialised instance.
        unsafe { msg_send![super(this), init] }
    }
}

fn decode_action(info: &NSDictionary) -> NotificationAction {
    let get = |key: &str| -> Option<String> {
        // SAFETY: userInfo values we wrote are NSStrings; anything else is ignored.
        let value = info.objectForKey(&NSString::from_str(key))?;
        let s: Option<Retained<NSString>> = value.downcast::<NSString>().ok();
        s.map(|s| s.to_string())
    };
    match get(KEY_ACTION).as_deref() {
        Some(ACTION_REVEAL) => get(KEY_PATH)
            .map(|p| NotificationAction::RevealFile(PathBuf::from(p)))
            .unwrap_or(NotificationAction::None),
        Some(ACTION_OPTIONS) => NotificationAction::OpenOptions,
        Some(ACTION_URL) => get(KEY_URL)
            .map(NotificationAction::OpenUrl)
            .unwrap_or(NotificationAction::None),
        _ => NotificationAction::None,
    }
}

fn encode_action(action: &NotificationAction) -> Retained<NSDictionary<NSString, NSString>> {
    let pairs: Vec<(Retained<NSString>, Retained<NSString>)> = match action {
        NotificationAction::None => Vec::new(),
        NotificationAction::RevealFile(path) => vec![
            (
                NSString::from_str(KEY_ACTION),
                NSString::from_str(ACTION_REVEAL),
            ),
            (
                NSString::from_str(KEY_PATH),
                NSString::from_str(&path.to_string_lossy()),
            ),
        ],
        NotificationAction::OpenOptions => vec![(
            NSString::from_str(KEY_ACTION),
            NSString::from_str(ACTION_OPTIONS),
        )],
        NotificationAction::OpenUrl(url) => vec![
            (
                NSString::from_str(KEY_ACTION),
                NSString::from_str(ACTION_URL),
            ),
            (NSString::from_str(KEY_URL), NSString::from_str(url)),
        ],
    };
    let keys: Vec<&NSString> = pairs.iter().map(|(k, _)| &**k).collect();
    let values: Vec<Retained<NSString>> = pairs.iter().map(|(_, v)| v.clone()).collect();
    NSDictionary::from_retained_objects(&keys, &values)
}

/// `Notifier` for macOS. Construct once on the main thread (the delegate is installed on
/// the current notification center); `notify` may be called from any thread.
pub struct MacNotifier {
    /// `None` when the process is not an `.app` bundle (dev binary): log-only mode.
    center: Option<Retained<UNUserNotificationCenter>>,
    _delegate: Option<Retained<NotificationDelegate>>,
    authorized: OnceCell<()>,
}

// SAFETY: UNUserNotificationCenter is documented thread-safe for `addNotificationRequest`
// and the delegate holds only a `Send + Sync` callback; the `OnceCell` is written once from
// the first `notify` call and only read afterwards (benign duplicate request if raced).
unsafe impl Send for MacNotifier {}
unsafe impl Sync for MacNotifier {}

impl std::fmt::Debug for MacNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MacNotifier(bundle={})", self.center.is_some())
    }
}

/// `true` when the running executable lives inside an `.app` bundle — the precondition for
/// `UNUserNotificationCenter` (it aborts with "bundleProxyForCurrentProcess is nil" otherwise).
pub fn running_from_bundle() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

impl MacNotifier {
    pub fn new(on_click: ClickHandler) -> Result<Self, PlatformError> {
        MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        if !running_from_bundle() {
            eprintln!("notifications: not an .app bundle — notifications are logged only");
            return Ok(Self {
                center: None,
                _delegate: None,
                authorized: OnceCell::new(),
            });
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = NotificationDelegate::new(on_click);
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Ok(Self {
            center: Some(center),
            _delegate: Some(delegate),
            authorized: OnceCell::new(),
        })
    }

    /// Lazily ask for Alert + Sound authorization on the first notification (Pitfall 18).
    fn ensure_authorized(&self, center: &UNUserNotificationCenter) {
        self.authorized.get_or_init(|| {
            let block = RcBlock::new(|granted: objc2::runtime::Bool, error: *mut NSError| {
                if !granted.as_bool() {
                    eprintln!("notifications: authorization not granted");
                }
                if !error.is_null() {
                    // SAFETY: a non-null NSError pointer from the completion handler.
                    let err = unsafe { &*error };
                    eprintln!("notifications: authorization error: {err}");
                }
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &block,
            );
        });
    }
}

impl Notifier for MacNotifier {
    fn notify(
        &self,
        title: &str,
        body: &str,
        action: NotificationAction,
    ) -> Result<(), PlatformError> {
        let Some(center) = &self.center else {
            eprintln!("notification: {title} — {body} ({action:?})");
            return Ok(());
        };
        self.ensure_authorized(center);
        // SAFETY: standard alloc/init of a mutable content object.
        let content: Retained<UNMutableNotificationContent> =
            unsafe { msg_send![UNMutableNotificationContent::alloc(), init] };
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let info = encode_action(&action);
        // SAFETY: the dictionary holds NSString keys and values, which are property-list
        // types; the generic erasure to AnyObject is a plain pointer cast of the same object.
        let erased: &NSDictionary = unsafe { &*(Retained::as_ptr(&info) as *const NSDictionary) };
        unsafe { content.setUserInfo(erased) };
        let id = NSString::from_str(&format!(
            "snapshade-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros())
                .unwrap_or(0)
        ));
        // A nil trigger delivers immediately.
        let request =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let block = RcBlock::new(|error: *mut NSError| {
            if !error.is_null() {
                // SAFETY: non-null NSError from the completion handler.
                let err = unsafe { &*error };
                eprintln!("notifications: add request failed: {err}");
            }
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
        Ok(())
    }
}
