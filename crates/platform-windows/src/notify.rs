//! WinRT toast notifications with click delivery (EXPORT-06, HOTKEY-03). An unpackaged app
//! needs an Application User Model ID that matches a Start Menu shortcut (the NSIS
//! installer creates it with the bundle identifier); without it `Show` fails and the
//! failure is logged, never fatal.

use std::sync::Arc;

use snapshade_platform_api::{NotificationAction, PlatformError};
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

pub type ClickHandler = Arc<dyn Fn(NotificationAction) + Send + Sync>;

pub struct ToastNotifier {
    aumid: String,
    on_click: ClickHandler,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl ToastNotifier {
    pub fn new(aumid: &str, on_click: ClickHandler) -> Self {
        Self {
            aumid: aumid.to_string(),
            on_click,
        }
    }

    pub fn notify(
        &self,
        title: &str,
        body: &str,
        action: NotificationAction,
    ) -> Result<(), PlatformError> {
        let xml = format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
            escape(title),
            escape(body)
        );
        let doc = XmlDocument::new().map_err(|e| PlatformError::Os(format!("XmlDocument: {e}")))?;
        doc.LoadXml(&HSTRING::from(xml))
            .map_err(|e| PlatformError::Os(format!("LoadXml: {e}")))?;
        let toast = ToastNotification::CreateToastNotification(&doc)
            .map_err(|e| PlatformError::Os(format!("CreateToastNotification: {e}")))?;
        let on_click = Arc::clone(&self.on_click);
        let action = Arc::new(action);
        toast
            .Activated(&TypedEventHandler::new(move |_, _| {
                on_click((*action).clone());
                Ok(())
            }))
            .map_err(|e| PlatformError::Os(format!("Activated: {e}")))?;
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&self.aumid))
                .map_err(|e| PlatformError::Os(format!("CreateToastNotifierWithId: {e}")))?;
        notifier
            .Show(&toast)
            .map_err(|e| PlatformError::Os(format!("Show: {e}")))
    }
}
