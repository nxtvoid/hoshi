//! Serves the pages agents show in their threads to the transcript's frames, under the
//! `hoshi-page` scheme, sandboxed away from the app.

use crate::orchestration;
use tauri::http::{Response, StatusCode, header};
use tauri::{UriSchemeContext, UriSchemeResponder, Wry};

pub const SCHEME: &str = "hoshi-page";

/// Scripts and forms only: the page gets an origin of its own, apart from the app's.
const SANDBOX: &str = "sandbox allow-scripts allow-forms";

pub fn serve(
    context: UriSchemeContext<'_, Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    let id = request.uri().path().trim_start_matches('/').to_owned();
    std::thread::spawn(move || {
        let page = orchestration::engine(&app).html_page(&id);
        let response = match page {
            Some(html) => Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_SECURITY_POLICY, SANDBOX)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::CACHE_CONTROL, "private, max-age=3600")
                .body(html),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new()),
        };
        if let Ok(response) = response {
            responder.respond(response);
        }
    });
}
