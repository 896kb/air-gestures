//! The Meta sign-in window (kinesis `MetaLoginView.swift`): an incognito
//! WebKitGTK view on Meta's own page. It never sees the password's value;
//! it only intercepts the final redirect.
//!
//! How the callback is caught: wry's navigation handler is WebKitGTK's
//! `decide-policy` signal for navigation actions, which WebKit raises for
//! every main- and sub-frame navigation, server redirects included (the same
//! policy hook `WKNavigationDelegate` uses in the macOS app). The handler
//! cancels the `fb-viewapp:`/`frl_login` navigation and posts its URL to the
//! event loop. As a fallback, `fb-viewapp` is also registered as a custom
//! protocol, so a load that somehow skipped the policy check is still caught
//! there instead of failing as an unknown scheme.

use std::borrow::Cow;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::unix::WindowExtUnix;
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, PageLoadEvent, WebViewBuilderExtUnix};

use crate::metaauth::is_callback;

const TITLE: &str = "Sign in with Meta (kinesis-linux)";

/// What the window does with a navigation to `url`.
#[derive(Debug, PartialEq, Eq)]
pub enum Navigation {
    /// The login callback: cancel it and hand its URL back.
    Callback,
    Allow,
    Block,
}

pub fn decide(url: &str) -> Navigation {
    if is_callback(url) {
        Navigation::Callback
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
    Callback(String),
    /// A `window.open`/`target=_blank` link: load it in this view (like the macOS app).
    LoadHere(String),
    /// A page committed: show its real host in the title, so it's plain where the password goes.
    Committed(String),
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

/// Show Meta's sign-in page at `url`. Returns the callback URL, or `None` if
/// the window was closed first. Must run on the main thread.
pub fn open(url: &str) -> Result<Option<String>, String> {
    // tao panics if GTK can't reach a display; say so plainly instead.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return Err(
            "the sign-in window needs a graphical session (no WAYLAND_DISPLAY or DISPLAY)".into(),
        );
    }
    // Initializes GTK (tao calls gtk::init) on this thread.
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title(window_title(url))
        .with_inner_size(tao::dpi::LogicalSize::new(640.0, 760.0))
        .build(&event_loop)
        .map_err(|error| format!("could not open the sign-in window: {error}"))?;

    let navigation_proxy = event_loop.create_proxy();
    let protocol_proxy = event_loop.create_proxy();
    let popup_proxy = event_loop.create_proxy();
    let load_proxy = event_loop.create_proxy();
    let webview = wry::WebViewBuilder::new()
        .with_url(url)
        .with_incognito(true)
        .with_navigation_handler(move |target: String| match decide(&target) {
            Navigation::Callback => {
                let _ = navigation_proxy.send_event(UserEvent::Callback(target));
                false
            }
            Navigation::Allow => true,
            Navigation::Block => false,
        })
        .with_custom_protocol("fb-viewapp".into(), move |_id, request| {
            let _ = protocol_proxy.send_event(UserEvent::Callback(request.uri().to_string()));
            wry::http::Response::builder()
                .status(204)
                .body(Cow::Borrowed(&[][..]))
                .expect("static response")
        })
        .with_new_window_req_handler(move |target, _features| {
            let _ = match decide(&target) {
                Navigation::Callback => popup_proxy.send_event(UserEvent::Callback(target)),
                Navigation::Allow => popup_proxy.send_event(UserEvent::LoadHere(target)),
                Navigation::Block => Ok(()),
            };
            NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |event, target| {
            if matches!(event, PageLoadEvent::Started) {
                let _ = load_proxy.send_event(UserEvent::Committed(target));
            }
        })
        .build_gtk(window.gtk_window())
        .map_err(|error| format!("could not start the web view: {error}"))?;

    let mut result = None;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Callback(target)) => {
                result = Some(target);
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::LoadHere(target)) => {
                let _ = webview.load_url(&target);
            }
            Event::UserEvent(UserEvent::Committed(target)) => {
                window.set_title(&window_title(&target))
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_and_blank_pages_load_and_the_callback_is_caught() {
        assert_eq!(decide("https://auth.meta.com/login"), Navigation::Allow);
        assert_eq!(decide("HTTPS://auth.meta.com/"), Navigation::Allow);
        assert_eq!(decide("about:blank"), Navigation::Allow);
        assert_eq!(decide("http://auth.meta.com/"), Navigation::Block);
        assert_eq!(decide("file:///etc/passwd"), Navigation::Block);
        assert_eq!(decide("javascript:alert(1)"), Navigation::Block);
        assert_eq!(
            decide("fb-viewapp://frl_login?token=a&blob=b"),
            Navigation::Callback
        );
        assert_eq!(decide("FB-VIEWAPP:x"), Navigation::Callback);
        assert_eq!(
            decide("https://m.meta.com/frl_login/?token=a"),
            Navigation::Callback
        );
    }

    #[test]
    fn the_title_names_the_real_host() {
        assert!(window_title("https://auth.meta.com/?x=1").starts_with("auth.meta.com — "));
        assert_eq!(window_title("about:blank"), TITLE);
    }
}
