use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::core::HSTRING;

/// Fixed AppUserModelId used for both `CreateToastNotifierWithId` here and
/// the registry display-name/icon registration in `registry.rs` — must
/// match exactly or the toast falls back to an unbranded identity.
pub const AUMID: &str = "AutoGSE.CLI";

fn xml_escape(s: &str) -> String {
    s.chars().fold(String::with_capacity(s.len()), |mut acc, c| {
        match c {
            '&' => acc.push_str("&amp;"),
            '<' => acc.push_str("&lt;"),
            '>' => acc.push_str("&gt;"),
            '"' => acc.push_str("&quot;"),
            '\'' => acc.push_str("&apos;"),
            _ => acc.push(c),
        }
        acc
    })
}

/// Builds a minimal `ToastGeneric` payload (PRD §9.2's two-line mockups: a
/// title line and a body line). Pure and XML-escaped, so it's testable
/// without touching the WinRT APIs themselves.
pub fn build_toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    )
}

/// Shows a native Windows toast notification. Best-effort: any failure
/// (notifications disabled in Settings, no COM/WinRT available, etc.) is
/// swallowed here — a failed toast must never fail the underlying
/// inject/revert operation, which has already succeeded or failed on its
/// own terms by the time this is called.
///
/// `Show()` returning success only means the request was accepted — the
/// actual hand-off to the notification platform happens asynchronously, and
/// this process exits immediately after `main()` returns with nothing else
/// keeping it alive. Confirmed empirically: without a brief pause here, the
/// toast silently never appears even though every WinRT call reports
/// success, because the process (and its COM apartment) tears down before
/// the platform finishes delivery. A short sleep is the pragmatic fix for a
/// short-lived CLI with no other reason to stay alive.
pub fn show(title: &str, body: &str) {
    if try_show(title, body).is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
}

fn try_show(title: &str, body: &str) -> windows::core::Result<()> {
    unsafe {
        // Idempotent per-thread; never paired with RoUninitialize since this
        // is a short-lived CLI process that exits right after.
        RoInitialize(RO_INIT_MULTITHREADED).ok();
    }

    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(build_toast_xml(title, body)))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_structure() {
        let xml = build_toast_xml("Title", "Body text");
        assert!(xml.contains("<text>Title</text>"));
        assert!(xml.contains("<text>Body text</text>"));
        assert!(xml.starts_with("<toast>"));
        assert!(xml.ends_with("</toast>"));
    }

    #[test]
    fn escapes_special_characters() {
        let xml = build_toast_xml("A & B <game>", "Path \"C:\\Games\"");
        assert!(xml.contains("A &amp; B &lt;game&gt;"));
        assert!(xml.contains("Path &quot;C:\\Games&quot;"));
        assert!(!xml.contains("<game>"));
    }

    /// Manual QA only — WinRT toast display can't be asserted by an
    /// automated test; this visually confirms a real toast appears.
    /// `cargo test notify::tests::live_show_real_toast -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_show_real_toast() {
        try_show("AutoGSE: Test Toast", "If you can see this, WinRT toast notifications are working.").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
    }

    #[test]
    fn handles_empty_strings() {
        let xml = build_toast_xml("", "");
        assert_eq!(xml, "<toast><visual><binding template=\"ToastGeneric\"><text></text><text></text></binding></visual></toast>");
    }
}
