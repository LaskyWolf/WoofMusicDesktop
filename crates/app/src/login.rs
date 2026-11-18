//! Google sign-in in a webview, run as its own process (`woofmusic --login`)
//! so tao's GTK loop never shares a process with iced's. Prints the session
//! as `AuthState` JSON on stdout, or exits 1 without one.
//!
//! `woofmusic --refresh` reads a saved `AuthState` on stdin and renews its
//! rotating tokens in a hidden webview (SwiftyTube `SessionTokenRefresher`).

use std::collections::BTreeMap;
use std::process::exit;
use std::time::{Duration, Instant};

use swiftytube::auth::auth_state::AuthState;
use swiftytube::auth::cookie_harvest::{self, Cookie};
use swiftytube::auth::login_webview_harvest as harvest;
use swiftytube::auth::session_token_refresher as refresher;
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{WebView, WebViewBuilder};

pub fn run(refresh: bool) -> ! {
    let base: Option<AuthState> = refresh
        .then(|| serde_json::from_reader(std::io::stdin()).unwrap_or_else(|error| fail(&error)));
    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Sign in to YouTube Music")
        .with_inner_size(LogicalSize::new(480.0, 720.0))
        .with_visible(!refresh)
        .build(&event_loop)
        .unwrap_or_else(|error| fail(&error));
    // Incognito: nothing of the sign-in stays on disk, and the next sign-in
    // asks for an account again.
    let builder = WebViewBuilder::new()
        .with_user_agent(harvest::USER_AGENT)
        .with_incognito(true);
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("tao windows have a vbox"))
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    let webview = webview.unwrap_or_else(|error| fail(&error));

    if let Some(base) = &base {
        for (name, value) in &base.cookies {
            let cookie = wry::cookie::Cookie::build((name.as_str(), value.as_str()))
                .domain(refresher::seed_domain(name))
                .path("/")
                .secure(true)
                .build();
            let _ = webview.set_cookie(&cookie);
        }
        load(&webview, refresher::LOGIN_ENTRY_URL);
    } else {
        load(&webview, harvest::LOGIN_URL);
    }

    let proxy = event_loop.create_proxy();
    let started = Instant::now();
    let mut next_tick = started + Duration::from_secs(1);
    let mut landed_at: Option<Instant> = None;
    let mut attempt = 0;
    let mut session: Option<(BTreeMap<String, String>, String)> = None;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(next_tick);
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => exit(1),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) if session.is_none() => {
                next_tick += Duration::from_secs(1);
                *flow = ControlFlow::WaitUntil(next_tick);
                if landed_at.is_none() {
                    if webview.url().is_ok_and(|url| harvest::landed(&url)) {
                        landed_at = Some(Instant::now());
                    } else if refresh
                        && started.elapsed().as_secs() >= u64::from(refresher::POLL_SECONDS)
                    {
                        // Google did not accept the session: a real sign-in is needed.
                        exit(1);
                    } else {
                        // Signing in takes the user as long as it takes.
                        return;
                    }
                }
                let store = cookies(&webview);
                if let Some(base) = &base {
                    if landed_at.is_some_and(|at| {
                        at.elapsed().as_secs() >= u64::from(refresher::SETTLE_SECONDS)
                    }) {
                        let cookies = refresher::reissued(&base.cookies, &store);
                        if cookies.is_empty() {
                            exit(1);
                        }
                        print(AuthState {
                            visitor_data: cookies.get("visitor_data").cloned(),
                            cookies,
                            ..base.clone()
                        });
                    }
                    return;
                }
                attempt += 1;
                let (cookies, sapisid) = cookie_harvest::select(&store);
                if harvest::is_complete(attempt, &cookies, sapisid.as_deref()) {
                    session = sapisid.map(|sapisid| (cookies, sapisid));
                    let proxy = proxy.clone();
                    let _ = webview.evaluate_script_with_callback(
                        harvest::DATA_SYNC_ID_SCRIPT,
                        move |result| {
                            let _ = proxy.send_event(result);
                        },
                    );
                } else if attempt >= harvest::MAX_ATTEMPTS {
                    exit(1);
                }
            }
            // The page answered with the data sync id: the session is whole.
            Event::UserEvent(result) => {
                let Some((cookies, sapisid)) = session.take() else {
                    return;
                };
                // The script's value comes back as JSON: `"id"` or `""`.
                let data_sync_id = serde_json::from_str::<String>(&result)
                    .ok()
                    .filter(|id| !id.is_empty());
                print(AuthState {
                    visitor_data: cookies.get("visitor_data").cloned(),
                    cookies,
                    sapisid: Some(sapisid),
                    data_sync_id,
                });
            }
            _ => {}
        }
    })
}

fn load(webview: &WebView, url: &str) {
    webview.load_url(url).unwrap_or_else(|error| fail(&error));
}

fn cookies(webview: &WebView) -> Vec<Cookie> {
    webview
        .cookies()
        .unwrap_or_default()
        .into_iter()
        .map(|cookie| Cookie {
            name: cookie.name().to_owned(),
            value: cookie.value().to_owned(),
            domain: cookie.domain().unwrap_or_default().to_owned(),
        })
        .collect()
}

fn print(auth: AuthState) -> ! {
    println!(
        "{}",
        serde_json::to_string(&auth).expect("AuthState serializes")
    );
    exit(0)
}

fn fail(error: &dyn std::fmt::Display) -> ! {
    eprintln!("{error}");
    exit(1)
}
