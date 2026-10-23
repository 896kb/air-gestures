//! The Meta sign-in window: an incognito
//! WebKitGTK view on Meta's own page. It never sees the password's value;
//! it only intercepts the final redirect.
//!
//! How the callback is caught: wry's navigation handler is WebKitGTK's
//! `decide-policy` signal for navigation actions, which WebKit raises for
//! every main- and sub-frame navigation, server redirects included (the same
//! policy hook `WKNavigationDelegate` uses in the macOS app). The handler
//! cancels every `fb-viewapp:`/`frl_login` navigation; one that the caller's
//! `accept` validates is posted to the event loop, which hides the window and
//! returns it, while any other is ignored and the window stays open. As a
//! fallback, `fb-viewapp` is also registered as a custom protocol (validated
//! the same way), so a load that somehow skipped the policy check is still
//! caught there instead of failing as an unknown scheme.

use std::borrow::Cow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::unix::WindowExtUnix;
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, PageLoadEvent, WebViewBuilderExtUnix};

use crate::metaauth::is_callback;

const TITLE: &str = "Sign in with Meta (air-gestures)";
const REJECTED: &str = "ignored a sign-in redirect that didn't match; keep going in the window";
/// How long the hidden window gets to be flushed before the loop returns.
const CLOSE_DELAY: Duration = Duration::from_millis(50);

/// What the window does with a navigation to `url`.
#[derive(Debug, PartialEq, Eq)]
pub enum Navigation {
    /// The validated login callback: cancel it, hand its URL back, close.
    Accepted,
    /// Callback-shaped but not valid (wrong/missing token, empty blob): cancel it, keep the window.
    Rejected,
    Allow,
    Block,
}

/// `accept` validates a callback-shaped URL (token digest + non-empty blob).
pub fn decide(url: &str, accept: &dyn Fn(&str) -> bool) -> Navigation {
    if is_callback(url) {
        if accept(url) {
            Navigation::Accepted
        } else {
            Navigation::Rejected
        }
    } else if url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        || url == "about:blank"
    {
        Navigation::Allow
    } else {
        Navigation::Block
    }
}

enum UserEvent {
    /// A validated callback URL.
    Accepted(String),
    /// A `window.open`/`target=_blank` request: decided on the loop (like the macOS app, allowed
    /// pages load in this view).
    Popup(String),
    /// A page committed: show its real host in the title, so it's plain where the password goes.
    Committed(String),
    /// Wakes the loop once the close delay has passed.
    Wake,
}

fn window_title(url: &str) -> String {
    match url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
    {
        Some(host) => format!("{host} — {TITLE}"),
        None => TITLE.to_owned(),
    }
}

/// Builds the event loop (tao calls `gtk::init` here), turning tao's panic on
/// an unusable display into an error.
fn event_loop() -> Result<EventLoop<UserEvent>, String> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let built = catch_unwind(AssertUnwindSafe(|| {
        EventLoopBuilder::<UserEvent>::with_user_event().build()
    }));
    std::panic::set_hook(hook);
    built.map_err(|_| "no usable display for the sign-in window".to_owned())
}

/// Show Meta's sign-in page at `url`. Returns the first callback URL that
/// `accept` validates, or `None` if the window was closed first. Rejected
/// callbacks are cancelled and the window stays open. Must run on the main thread.
pub fn open(url: &str, accept: impl Fn(&str) -> bool + 'static) -> Result<Option<String>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return Err(
            "the sign-in window needs a graphical session (no WAYLAND_DISPLAY or DISPLAY)".into(),
        );
    }
    let mut event_loop = event_loop()?;
    let window = WindowBuilder::new()
        .with_title(window_title(url))
        .with_inner_size(tao::dpi::LogicalSize::new(640.0, 760.0))
        .build(&event_loop)
        .map_err(|error| format!("could not open the sign-in window: {error}"))?;

    let accept: Rc<dyn Fn(&str) -> bool> = Rc::new(accept);
    let navigation_accept = accept.clone();
    let protocol_accept = accept.clone();
    let navigation_proxy = event_loop.create_proxy();
    let protocol_proxy = event_loop.create_proxy();
    let popup_proxy = event_loop.create_proxy();
    let load_proxy = event_loop.create_proxy();
    let wake_proxy = event_loop.create_proxy();
    let webview = wry::WebViewBuilder::new()
        .with_url(url)
        .with_incognito(true)
        .with_navigation_handler(
            move |target: String| match decide(&target, &*navigation_accept) {
                Navigation::Accepted => {
                    let _ = navigation_proxy.send_event(UserEvent::Accepted(target));
                    false
                }
                Navigation::Rejected => {
                    eprintln!("{REJECTED}");
                    false
                }
                Navigation::Allow => true,
                Navigation::Block => false,
            },
        )
        .with_custom_protocol("fb-viewapp".into(), move |_id, request| {
            let target = request.uri().to_string();
            if protocol_accept(&target) {
                let _ = protocol_proxy.send_event(UserEvent::Accepted(target));
            } else {
                eprintln!("{REJECTED}");
            }
            wry::http::Response::builder()
                .status(204)
                .body(Cow::Borrowed(&[][..]))
                .expect("static response")
        })
        .with_new_window_req_handler(move |target, _features| {
            let _ = popup_proxy.send_event(UserEvent::Popup(target));
            NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |event, target| {
            if matches!(event, PageLoadEvent::Started) {
                let _ = load_proxy.send_event(UserEvent::Committed(target));
            }
        })
        // tao already puts a GtkBox in the window (a GtkBin holds one child),
        // so the web view must go into that box, not the window itself.
        .build_gtk(
            window
                .default_vbox()
                .ok_or("the sign-in window has no container for the web view")?,
        )
        .map_err(|error| format!("could not start the web view: {error}"))?;

    let mut result = None;
    let mut closing = false;
    event_loop.run_return(|event, _, control_flow| {
        if closing {
            // The window is hidden; leave once the delay has let GTK flush that.
            if matches!(
                event,
                Event::NewEvents(StartCause::ResumeTimeReached { .. })
            ) {
                *control_flow = ControlFlow::Exit;
            }
            return;
        }
        *control_flow = ControlFlow::Wait;
        let mut close = false;
        match event {
            Event::UserEvent(UserEvent::Accepted(target)) => {
                result = Some(target);
                close = true;
            }
            Event::UserEvent(UserEvent::Popup(target)) => match decide(&target, &*accept) {
                Navigation::Accepted => {
                    result = Some(target);
                    close = true;
                }
                Navigation::Rejected => eprintln!("{REJECTED}"),
                Navigation::Allow => {
                    let _ = webview.load_url(&target);
                }
                Navigation::Block => {}
            },
            Event::UserEvent(UserEvent::Committed(target)) => {
                window.set_title(&window_title(&target))
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => close = true,
            _ => {}
        }
        if close {
            closing = true;
            window.set_visible(false);
            *control_flow = ControlFlow::WaitUntil(Instant::now() + CLOSE_DELAY);
            // tao's GTK loop blocks without a timer of its own; this wakes it after the delay.
            let proxy = wake_proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(CLOSE_DELAY + Duration::from_millis(10));
                let _ = proxy.send_event(UserEvent::Wake);
            });
        }
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE: &str = "epfEORXEWkN2obB5";

    fn accept(url: &str) -> bool {
        crate::metaauth::accepts_callback(url, NATIVE)
    }

    #[test]
    fn only_https_and_blank_pages_load() {
        assert_eq!(
            decide("https://auth.meta.com/login", &accept),
            Navigation::Allow
        );
        assert_eq!(decide("HTTPS://auth.meta.com/", &accept), Navigation::Allow);
        assert_eq!(decide("about:blank", &accept), Navigation::Allow);
        assert_eq!(decide("http://auth.meta.com/", &accept), Navigation::Block);
        assert_eq!(decide("file:///etc/passwd", &accept), Navigation::Block);
        assert_eq!(decide("javascript:alert(1)", &accept), Navigation::Block);
    }

    #[test]
    fn a_valid_callback_is_accepted() {
        let token = crate::metaauth::expected_callback_token(NATIVE);
        let url = format!("fb-viewapp://frl_login?token={token}&blob=AbC%2B");
        assert_eq!(decide(&url, &accept), Navigation::Accepted);
        let upper = format!("FB-VIEWAPP://frl_login?token={token}&blob=x");
        assert_eq!(decide(&upper, &accept), Navigation::Accepted);
    }

    #[test]
    fn a_callback_with_a_wrong_token_or_no_blob_is_rejected() {
        let wrong = "fb-viewapp://frl_login?token=0123456789abcdef&blob=b";
        assert_eq!(decide(wrong, &accept), Navigation::Rejected);
        let token = crate::metaauth::expected_callback_token(NATIVE);
        let no_blob = format!("fb-viewapp://frl_login?token={token}&blob=");
        assert_eq!(decide(&no_blob, &accept), Navigation::Rejected);
        assert_eq!(decide("fb-viewapp:x", &accept), Navigation::Rejected);
    }

    #[test]
    fn an_https_page_merely_mentioning_frl_login_is_rejected() {
        assert_eq!(
            decide("https://m.meta.com/frl_login/?token=a", &accept),
            Navigation::Rejected
        );
    }

    #[test]
    fn the_title_names_the_real_host() {
        assert!(window_title("https://auth.meta.com/?x=1").starts_with("auth.meta.com — "));
        assert_eq!(window_title("about:blank"), TITLE);
    }
}
