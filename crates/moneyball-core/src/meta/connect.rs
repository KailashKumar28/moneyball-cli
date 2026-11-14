//! Continue with Facebook (plans/desktop/auth-plan.md, "The Meta
//! protocol"): Meta's dialog in the browser, the code swapped for a token
//! by Moneyball's broker, the token handed back to `oauth::Loopback`,
//! stored, then read back with `check_token`. No HTTP of its own.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::{check_token, or_stand_in, TokenCheck};
use crate::error::{Error, Result};
use crate::oauth::{self, Loopback};
use crate::{secrets, text};

/// The Graph API version of the dialog; the broker swaps the code on the same.
pub const VERSION: &str = "v26.0";
/// Moneyball's Meta app (type Business, with Facebook Login for Business).
/// APP_ID, CONFIG_ID and BROKER are the owner's to set once the app, its
/// configuration and the broker exist (auth-plan.md, "The owner's setup
/// outside the code"). Each is public: none is a secret.
pub const APP_ID: &str = "SET-ME";
/// The app's Login for Business configuration: a system-user token that
/// never expires, with `SCOPES`, for ad accounts and Pages.
pub const CONFIG_ID: &str = "SET-ME";
/// The broker (broker/, a Cloudflare Worker), which holds the app secret.
/// Its `/meta/callback` is the app's one valid OAuth redirect URI.
pub const BROKER: &str = "https://moneyball-broker.SET-ME.workers.dev";
/// What CONFIG_ID grants (auth-plan.md A3): the ads, and the leads of the
/// Pages' lead forms. A write scope beyond these is warned about.
pub const SCOPES: [&str; 6] = [
    "ads_read",
    "leads_retrieval",
    "pages_manage_ads",
    "pages_show_list",
    "pages_read_engagement",
    "business_management",
];

/// Continue with Facebook: `open` shows Meta's dialog (the system browser,
/// never a webview; it returns once asked, as a browser does, without
/// waiting for the page), then the token the broker's page brings back
/// is stored and Meta says what it is. Until then, and when the person
/// declines, cancels (`cancel`) or takes longer than `oauth::TIMEOUT`,
/// nothing is stored.
pub fn connect(
    open: impl FnOnce(&str) -> std::result::Result<(), String>,
    cancel: &AtomicBool,
) -> Result<TokenCheck> {
    let token = authorize(open, cancel, oauth::TIMEOUT)?;
    secrets::store_meta_token(&token)?;
    check_token(&token)
}

/// Protocol steps 1 to 5 up to the token in hand.
fn authorize(
    open: impl FnOnce(&str) -> std::result::Result<(), String>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<String> {
    let back = Loopback::bind()?;
    open(&dialog_url(back.port(), back.state())).map_err(|e| {
        Error::Authorization(format!("Moneyball could not open your browser: {}", e))
    })?;
    back.token("/meta/done", cancel, timeout)
}

/// Meta's dialog (protocol step 2). The state carries the port, so the
/// stateless broker knows where to hand the token back. In the e2e build
/// each part can point at a stand-in.
fn dialog_url(port: u16, state: &str) -> String {
    let redirect = format!("{}/meta/callback", or_stand_in("MB_E2E_BROKER", BROKER));
    format!(
        "{}/{}/dialog/oauth?client_id={}&config_id={}&response_type=code\
         &override_default_response_type=true&redirect_uri={}&state={}.{}",
        or_stand_in("MB_E2E_META_DIALOG", "https://www.facebook.com"),
        VERSION,
        text::percent_encode(&or_stand_in("MB_E2E_META_APP_ID", APP_ID)),
        text::percent_encode(&or_stand_in("MB_E2E_META_CONFIG_ID", CONFIG_ID)),
        text::percent_encode(&redirect),
        port,
        state
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::tests::post;

    const TOKEN: &str = "EAAMoneyballConnectTestToken0123456789abcdef";

    /// Meta and the broker in one: from the dialog's URL to the form the
    /// broker's page submits, `answer` and the state after the port.
    fn browser(answer: &'static str) -> impl FnOnce(&str) -> std::result::Result<(), String> {
        move |url: &str| {
            let state = url.split("&state=").nth(1).unwrap_or_default();
            let (port, state) = state.split_once('.').unwrap_or_default();
            let (port, body) = (port.parse().unwrap(), format!("{}&state={}", answer, state));
            std::thread::spawn(move || post(port, &body));
            Ok(())
        }
    }

    #[test]
    fn the_dialog_is_metas_with_moneyballs_app_and_the_brokers_callback() {
        let url = dialog_url(4321, "abc_-");
        assert_eq!(
            url,
            format!(
                "https://www.facebook.com/{}/dialog/oauth?client_id=SET-ME&config_id=SET-ME\
                 &response_type=code&override_default_response_type=true&redirect_uri=\
                 https%3A%2F%2Fmoneyball-broker.SET-ME.workers.dev%2Fmeta%2Fcallback\
                 &state=4321.abc_-",
                VERSION
            )
        );
    }

    #[test]
    fn the_token_comes_back_through_the_broker_and_a_decline_in_words() {
        let never = AtomicBool::new(false);
        let token = format!("token={}", TOKEN);
        let got = authorize(
            browser(Box::leak(token.into_boxed_str())),
            &never,
            oauth::TIMEOUT,
        );
        assert_eq!(got.unwrap(), TOKEN);

        let declined = "error=Facebook+sign-in+was+cancelled%2C+so+nothing+was+connected.";
        let e = authorize(browser(declined), &never, oauth::TIMEOUT).unwrap_err();
        assert_eq!(
            e.reason(),
            "Facebook sign-in was cancelled, so nothing was connected."
        );
    }

    #[test]
    fn a_browser_that_will_not_open_is_said_at_once() {
        let open = |_: &str| Err("no handler for https".to_string());
        let e = authorize(open, &AtomicBool::new(false), oauth::TIMEOUT).unwrap_err();
        assert_eq!(e.kind(), "authorization");
        assert_eq!(
            e.reason(),
            "Moneyball could not open your browser: no handler for https"
        );
    }
}
