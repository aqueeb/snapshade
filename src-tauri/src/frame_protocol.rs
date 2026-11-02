//! The `frame://` custom URI scheme: frozen-frame PIXELS reach the overlay webview as an
//! HTTP body on request, never as base64/JSON over IPC (CLAUDE.md; RESEARCH Q5;
//! ARCHITECTURE Pattern 2; threat T-01-13/T-01-14).
//!
//! Plan B (Plan 01-09): nothing on the hotkey path fetches from this scheme any more —
//! the frozen frame is composited natively (`FrameLayer`) and the page receives
//! metadata only (`frame-ready`). The store therefore holds [`FrameSource`]s, not byte
//! buffers: on macOS a source wraps the retained `CGImage` and converts it to RGBA
//! **lazily, on the first `rgba()` call** (the `frame://` responder thread), caching the
//! result for the generation; never on the hotkey path. Phase 2's selection crop /
//! export is the consumer (D-02, D-03). The platform-neutral eager source
//! (`Arc<CaptureFrame>`) keeps the unit tests free of AppKit.
//!
//! URL: `frame://localhost/<display_id>?gen=<n>&token=<32 hex>` (macOS/WKWebView form);
//! on Windows (WebView2) and Android wry serves custom schemes as
//! `http://frame.localhost/<display_id>?…`, which [`parse_frame_url`] accepts too.
//! The wire format is rgba-only (Plan 01-13): the body is always the raw RGBA bytes
//! (`application/octet-stream`, `width * height * 4`, top-down, no header) with the
//! dimensions in `X-Frame-*` headers. There is no format selector: any query key other
//! than `gen` and `token` (including a legacy `fmt`) is ignored like every unknown key.
//! The BMP/BGRA variant of Plan 01-07 was measured slower (01-SPIKE-RESULT `## Frame
//! handoff`) and removed. `X-Frame-Served-Us` is the epoch-µs stamp right before
//! `respond` (informational since Plan 01-09: the D-09 point-3 timestamp `t2` is "frame
//! installed on the layer", stamped by the controller).
//!
//! **Access control (Plan 01-10; REVIEW WR-01; T-01-28).** This is the most sensitive
//! endpoint in the app — a full-resolution capture of every display — so pixels leave
//! Rust only when BOTH hold:
//! * the request's `Origin` header is exactly one of [`ALLOWED_ORIGINS`]
//!   (`tauri://localhost` macOS production, `http://tauri.localhost` Windows production,
//!   `http://localhost:1420` the Vite `devUrl`), checked by [`origin_allowed`]; the allowed
//!   origin is echoed back in `Access-Control-Allow-Origin` — never the wildcard;
//! * the query carries the current generation's `token`: 32 lowercase hex chars the
//!   controller generates per capture and delivers to the page ONLY inside the
//!   `frame-ready` event. It is an in-process capability token (unguessable from the page
//!   without the event; a predictable `gen` counter is no longer sufficient), not a
//!   cryptographic secret.
//!
//! Any other origin, an absent `Origin`, a missing / malformed / stale token, a stale
//! generation or an unknown display id is a 404 with an empty body and no CORS header.
//!
//! The [`FrameStore`] holds exactly ONE generation: every capture replaces all
//! sources (`gen + 1`) and the token, `overlay_cancel` clears them (T-01-16), and a
//! request for any other generation or an unknown display id is a 404 so a stale frame
//! can never be served to a fresh overlay (T-01-15).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use snapshade_core::capture::CaptureFrame;
use snapshade_core::geometry::DisplayId;
use tauri::{http, Manager, Runtime};

/// The managed state type: `app.manage(SharedFrameStore::default())`.
pub type SharedFrameStore = Arc<Mutex<FrameStore>>;

/// Geometry of one stored frame — everything the page and the response headers need
/// without touching pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMeta {
    pub display_id: DisplayId,
    /// Bitmap width in physical pixels.
    pub width: u32,
    /// Bitmap height in physical pixels.
    pub height: u32,
    /// Pixels per point of the display.
    pub scale: f64,
}

/// A frozen frame the store can serve: cheap metadata, and RGBA pixels produced on
/// demand. Implementations must be safe to share across the `frame://` responder
/// threads and the main thread.
pub trait FrameSource: Send + Sync {
    /// Dimensions and scale; never converts pixels.
    fn meta(&self) -> FrameMeta;
    /// The RGBA frame. May convert on the first call (and should cache the result for
    /// later calls); an `Err` is a human-readable reason the responder turns into a 404.
    fn rgba(&self) -> Result<Arc<CaptureFrame>, String>;
}

/// Eager source over an already-converted frame (tests, the fake platform, any caller
/// that went through `ScreenCapturer::capture_all`).
impl FrameSource for Arc<CaptureFrame> {
    fn meta(&self) -> FrameMeta {
        FrameMeta {
            display_id: self.display_id,
            width: self.width,
            height: self.height,
            scale: self.scale,
        }
    }

    fn rgba(&self) -> Result<Arc<CaptureFrame>, String> {
        Ok(Arc::clone(self))
    }
}

/// One generation of frame sources, keyed by display id, plus the generation's
/// capability token (see the module doc).
#[derive(Default)]
pub struct FrameStore {
    generation: u64,
    token: String,
    frames: HashMap<DisplayId, Arc<dyn FrameSource>>,
}

impl std::fmt::Debug for FrameStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is deliberately not printed.
        f.debug_struct("FrameStore")
            .field("generation", &self.generation)
            .field("displays", &self.frames.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl FrameStore {
    /// Replace every source with a new generation and its token (an old generation is
    /// dropped wholesale — the store never grows past one capture).
    pub fn replace(&mut self, generation: u64, token: String, sources: Vec<Arc<dyn FrameSource>>) {
        self.generation = generation;
        self.token = token;
        self.frames = sources
            .into_iter()
            .map(|source| (source.meta().display_id, source))
            .collect();
    }

    /// The source for `display_id` if — and only if — `generation` is current AND
    /// `token` is the current generation's token (byte-exact).
    pub fn get(
        &self,
        display_id: DisplayId,
        generation: u64,
        token: &str,
    ) -> Option<Arc<dyn FrameSource>> {
        if generation != self.generation || token != self.token {
            return None;
        }
        self.frames.get(&display_id).cloned()
    }

    /// The current generation's token (empty before the first capture) — for the
    /// controller's `frame-ready` payload and the in-process self-test.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Drop every source (Esc / cancel) — on macOS this releases the retained
    /// `CGImage`s and any cached RGBA. The generation counter is kept so the next
    /// capture is still strictly newer.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// The current generation (0 before the first capture).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of sources held.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Length of a frame token: 16 bytes as lowercase hex.
pub const TOKEN_LEN: usize = 32;

/// Origins that may read pixels over `frame://` (REVIEW WR-01). Exact string matches —
/// no prefix, suffix or wildcard logic.
pub const ALLOWED_ORIGINS: &[&str] = &[
    "tauri://localhost",      // macOS production (WKWebView)
    "http://tauri.localhost", // Windows production (WebView2, Phase 5)
    "http://localhost:1420",  // Vite devUrl
];

/// The allow-listed origin equal to `origin`, or `None` for an absent header and for
/// anything not in [`ALLOWED_ORIGINS`] (`tauri://localhost.evil` is not `tauri://localhost`).
pub fn origin_allowed(origin: Option<&str>) -> Option<&'static str> {
    let origin = origin?;
    ALLOWED_ORIGINS
        .iter()
        .copied()
        .find(|allowed| *allowed == origin)
}

/// `true` for a well-formed token: exactly [`TOKEN_LEN`] ASCII hex digits.
fn is_token(value: &str) -> bool {
    value.len() == TOKEN_LEN && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The URL prefixes a webview may use for this scheme: the native scheme form (WKWebView)
/// and wry's `http(s)://<scheme>.localhost` mapping (WebView2, Android). Exact hosts only.
const URL_PREFIXES: [&str; 3] = [
    "frame://localhost/",
    "http://frame.localhost/",
    "https://frame.localhost/",
];

/// Parse `frame://localhost/<display_id>?gen=<n>&token=<32 hex>` (or its WebView2 form) into
/// `(display_id, gen, token)` (the two query keys in either order).
///
/// Returns `None` for any deviation: a host other than `localhost`, a non-numeric or
/// empty display id, an extra path segment, a missing / non-numeric / duplicated `gen`,
/// a missing / duplicated / wrong-length / non-hex `token`, or a malformed query pair.
/// Any other query key (including a legacy `fmt`) is ignored — there is no format
/// selector. Strict on purpose (T-01-14): the handler 404s on `None` and never panics.
pub fn parse_frame_url(url: &str) -> Option<(u32, u64, &str)> {
    let rest = URL_PREFIXES
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
    let (path, query) = rest.split_once('?')?;
    if path.is_empty() || path.contains('/') {
        return None;
    }
    let display_id: u32 = path.parse().ok()?;
    let mut generation: Option<u64> = None;
    let mut token: Option<&str> = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "gen" => {
                if generation.is_some() {
                    return None;
                }
                generation = Some(value.parse().ok()?);
            }
            "token" => {
                if token.is_some() || !is_token(value) {
                    return None;
                }
                token = Some(value);
            }
            _ => {}
        }
    }
    Some((display_id, generation?, token?))
}

/// Epoch microseconds (the wall clock the webview's `performance.timeOrigin`
/// shares), for `X-Frame-Served-Us`.
fn epoch_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0)
}

/// Empty 404 — no body, no CORS header (nothing for a foreign origin to read).
fn not_found() -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(404)
        .body(Vec::new())
        .expect("static 404 response")
}

/// Printable, bounded form of a rejected `Origin` value for the stderr line.
fn printable(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// Register the `frame` scheme on the builder. The handler validates the URL (incl. the
/// token shape), the `Origin` header and the store entry synchronously (cheap: an `Arc`
/// clone under a short lock) and hands the conversion (first request of a generation) and
/// the 24–59 MB body copy to a spawned thread so the main thread is never blocked. A
/// conversion error is a 404 plus one stderr line, never a hang (T-01-25).
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol("frame", |ctx, request, responder| {
        let url = request.uri().to_string();
        let Some((display_id, generation, token)) = parse_frame_url(&url) else {
            responder.respond(not_found());
            return;
        };
        // WR-01: only an allow-listed page origin may read pixels; the header is read
        // from the request WKWebView / WebView2 hands wry (`http::Request::headers`).
        let origin_header = request
            .headers()
            .get("Origin")
            .and_then(|v| v.to_str().ok());
        let Some(origin) = origin_allowed(origin_header) else {
            eprintln!(
                "frame:// rejected: origin {}",
                origin_header.map_or_else(|| "<absent>".to_string(), printable)
            );
            responder.respond(not_found());
            return;
        };
        let source = {
            let store = ctx.app_handle().state::<SharedFrameStore>();
            let guard = store.lock().unwrap_or_else(|p| p.into_inner());
            guard.get(display_id, generation, token)
        };
        let Some(source) = source else {
            responder.respond(not_found());
            return;
        };
        let app = ctx.app_handle().clone();
        std::thread::spawn(move || {
            // Lazy conversion (first request of this generation) — off the hotkey path.
            let frame = match source.rgba() {
                Ok(frame) => frame,
                Err(reason) => {
                    eprintln!("frame:// display {display_id} gen {generation}: {reason}");
                    responder.respond(not_found());
                    return;
                }
            };
            let meta = source.meta();
            let served_us = epoch_us();
            // t2 for the fetch path (first stamp wins — see `Timings::record_served`).
            app.state::<crate::latency::SharedTimings>()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .record_served(generation, display_id, served_us);
            let response = http::Response::builder()
                .status(200)
                .header("Content-Type", "application/octet-stream")
                // The matched allow-list entry, echoed — never the wildcard (WR-01).
                .header("Access-Control-Allow-Origin", origin)
                .header(
                    "Access-Control-Expose-Headers",
                    "X-Frame-Width, X-Frame-Height, X-Frame-Scale, X-Frame-Served-Us",
                )
                .header("X-Frame-Width", meta.width.to_string())
                .header("X-Frame-Height", meta.height.to_string())
                .header("X-Frame-Scale", meta.scale.to_string())
                .header("X-Frame-Served-Us", served_us.to_string())
                .body(frame.rgba.clone());
            match response {
                Ok(response) => responder.respond(response),
                Err(err) => {
                    eprintln!("frame:// response build failed: {err}");
                    responder.respond(not_found());
                }
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::{
        origin_allowed, parse_frame_url, FrameMeta, FrameSource, FrameStore, ALLOWED_ORIGINS,
    };
    use snapshade_core::capture::CaptureFrame;

    /// A well-formed 32-hex token for the URL tests.
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_a_valid_url() {
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1?gen=7&token={TOKEN}")),
            Some((1, 7, TOKEN))
        );
        assert_eq!(
            parse_frame_url(&format!(
                "frame://localhost/4294967295?gen=18446744073709551615&token={TOKEN}"
            )),
            Some((u32::MAX, u64::MAX, TOKEN))
        );
    }

    #[test]
    fn parses_the_webview2_http_form() {
        assert_eq!(
            parse_frame_url(&format!("http://frame.localhost/2?gen=5&token={TOKEN}")),
            Some((2, 5, TOKEN))
        );
        assert_eq!(
            parse_frame_url(&format!("https://frame.localhost/2?gen=5&token={TOKEN}")),
            Some((2, 5, TOKEN))
        );
        // Exact hosts only: no sub-domain, no other scheme's host.
        assert_eq!(
            parse_frame_url(&format!(
                "http://frame.localhost.evil/2?gen=5&token={TOKEN}"
            )),
            None
        );
        assert_eq!(
            parse_frame_url(&format!("http://tauri.localhost/2?gen=5&token={TOKEN}")),
            None
        );
    }

    #[test]
    fn parses_gen_and_token_in_either_order() {
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/3?token={TOKEN}&gen=9")),
            Some((3, 9, TOKEN))
        );
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/3?gen=9&token={TOKEN}")),
            Some((3, 9, TOKEN))
        );
        // Unknown keys around them are still ignored.
        assert_eq!(
            parse_frame_url(&format!(
                "frame://localhost/3?fmt=bmp&token={TOKEN}&x=1&gen=9"
            )),
            Some((3, 9, TOKEN))
        );
        // Upper-case hex parses (the store compares byte-exact, so it still 404s there).
        let upper = TOKEN.to_ascii_uppercase();
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/3?gen=9&token={upper}")),
            Some((3, 9, upper.as_str()))
        );
    }

    #[test]
    fn rejects_a_missing_or_malformed_token() {
        assert_eq!(
            parse_frame_url("frame://localhost/1?gen=7"),
            None,
            "missing"
        );
        assert_eq!(
            parse_frame_url("frame://localhost/1?gen=7&token="),
            None,
            "empty"
        );
        assert_eq!(
            parse_frame_url(&format!(
                "frame://localhost/1?gen=7&token={TOKEN}&token={TOKEN}"
            )),
            None,
            "duplicated"
        );
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1?gen=7&token={}", &TOKEN[..31])),
            None,
            "too short"
        );
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1?gen=7&token={TOKEN}0")),
            None,
            "too long"
        );
        assert_eq!(
            parse_frame_url(&format!(
                "frame://localhost/1?gen=7&token={}g",
                &TOKEN[..31]
            )),
            None,
            "non-hex"
        );
        assert_eq!(
            parse_frame_url("frame://localhost/1?gen=7&token=zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            None,
            "non-hex, right length"
        );
        assert_eq!(
            parse_frame_url("frame://localhost/1?gen=7&token"),
            None,
            "malformed pair"
        );
    }

    #[test]
    fn ignores_unknown_query_keys_including_a_legacy_fmt() {
        // No format selector exists any more (Plan 01-13): `fmt` is just another
        // unknown key, so it can neither select an encoding nor reach dead header code.
        for url in [
            format!("frame://localhost/1?gen=7&fmt=bmp&token={TOKEN}"),
            format!("frame://localhost/1?fmt=rgba&gen=7&token={TOKEN}"),
            format!("frame://localhost/1?gen=7&token={TOKEN}&fmt=png"),
            format!("frame://localhost/1?gen=7&other=x&token={TOKEN}"),
        ] {
            assert_eq!(parse_frame_url(&url), Some((1, 7, TOKEN)), "{url}");
        }
        // `gen` strictness is unchanged by the presence of other keys.
        assert_eq!(
            parse_frame_url(&format!(
                "frame://localhost/1?fmt=bmp&gen=1&gen=2&token={TOKEN}"
            )),
            None
        );
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1?gen=7&token={TOKEN}&fmt")),
            None,
            "malformed pair"
        );
    }

    #[test]
    fn rejects_a_non_numeric_display_id() {
        for path in ["abc", "-1", ""] {
            assert_eq!(
                parse_frame_url(&format!("frame://localhost/{path}?gen=1&token={TOKEN}")),
                None,
                "{path:?}"
            );
        }
    }

    #[test]
    fn rejects_a_missing_or_non_numeric_gen() {
        assert_eq!(parse_frame_url("frame://localhost/1"), None);
        assert_eq!(parse_frame_url("frame://localhost/1?"), None);
        for query in [
            format!("other=2&token={TOKEN}"),
            format!("gen=x&token={TOKEN}"),
            format!("gen=&token={TOKEN}"),
            format!("gen=1&gen=2&token={TOKEN}"),
        ] {
            assert_eq!(
                parse_frame_url(&format!("frame://localhost/1?{query}")),
                None,
                "{query}"
            );
        }
    }

    #[test]
    fn rejects_an_extra_path_segment() {
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1/2?gen=1&token={TOKEN}")),
            None
        );
        assert_eq!(
            parse_frame_url(&format!("frame://localhost/1/?gen=1&token={TOKEN}")),
            None
        );
    }

    #[test]
    fn rejects_a_host_other_than_localhost() {
        for url in [
            format!("frame://evil/1?gen=1&token={TOKEN}"),
            format!("http://localhost/1?gen=1&token={TOKEN}"),
            format!("frame://localhost.evil/1?gen=1&token={TOKEN}"),
        ] {
            assert_eq!(parse_frame_url(&url), None, "{url}");
        }
        assert_eq!(parse_frame_url(""), None);
    }

    #[test]
    fn origin_allow_list_is_exact() {
        assert_eq!(
            origin_allowed(Some("tauri://localhost")),
            Some("tauri://localhost")
        );
        assert_eq!(
            origin_allowed(Some("http://tauri.localhost")),
            Some("http://tauri.localhost")
        );
        assert_eq!(
            origin_allowed(Some("http://localhost:1420")),
            Some("http://localhost:1420")
        );
        assert_eq!(origin_allowed(Some("tauri://localhost.evil")), None);
        assert_eq!(origin_allowed(Some("tauri://localhost/")), None);
        assert_eq!(origin_allowed(Some("http://localhost:1421")), None);
        assert_eq!(origin_allowed(Some("https://tauri.localhost")), None);
        assert_eq!(origin_allowed(Some("null")), None);
        assert_eq!(origin_allowed(Some("*")), None);
        assert_eq!(origin_allowed(Some("")), None);
        assert_eq!(origin_allowed(None), None);
        assert!(!ALLOWED_ORIGINS.contains(&"*"), "no wildcard in the list");
    }

    fn frame(id: u32) -> CaptureFrame {
        CaptureFrame {
            display_id: id,
            width: 2,
            height: 1,
            scale: 1.0,
            rgba: vec![0; 8],
        }
    }

    fn eager(id: u32) -> Arc<dyn FrameSource> {
        Arc::new(Arc::new(frame(id)))
    }

    #[test]
    fn eager_source_exposes_meta_and_the_same_frame() {
        let source = eager(7);
        assert_eq!(
            source.meta(),
            FrameMeta {
                display_id: 7,
                width: 2,
                height: 1,
                scale: 1.0,
            }
        );
        let a = source.rgba().unwrap();
        let b = source.rgba().unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "eager source hands out one shared frame"
        );
        assert_eq!(a.rgba.len(), 8);
    }

    /// A source that converts on demand and counts conversions (the shape of the macOS
    /// `StoredNativeFrame` without AppKit).
    struct Lazy {
        conversions: AtomicUsize,
        fail: bool,
    }

    impl FrameSource for Lazy {
        fn meta(&self) -> FrameMeta {
            FrameMeta {
                display_id: 3,
                width: 2,
                height: 1,
                scale: 2.0,
            }
        }

        fn rgba(&self) -> Result<Arc<CaptureFrame>, String> {
            self.conversions.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err("rgba_data: simulated".to_string());
            }
            Ok(Arc::new(frame(3)))
        }
    }

    #[test]
    fn store_never_converts_on_replace_or_get() {
        let lazy = Arc::new(Lazy {
            conversions: AtomicUsize::new(0),
            fail: false,
        });
        let mut store = FrameStore::default();
        store.replace(
            1,
            TOKEN.to_string(),
            vec![lazy.clone() as Arc<dyn FrameSource>],
        );
        let got = store.get(3, 1, TOKEN).expect("current generation");
        assert_eq!(got.meta().display_id, 3);
        assert_eq!(
            lazy.conversions.load(Ordering::SeqCst),
            0,
            "replace + get + meta must not convert pixels"
        );
        assert!(got.rgba().is_ok());
        assert_eq!(lazy.conversions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failing_source_reports_the_reason_and_does_not_panic() {
        let lazy: Arc<dyn FrameSource> = Arc::new(Lazy {
            conversions: AtomicUsize::new(0),
            fail: true,
        });
        assert_eq!(lazy.rgba().unwrap_err(), "rgba_data: simulated");
    }

    #[test]
    fn store_serves_only_the_current_generation_with_its_token() {
        const OTHER: &str = "ffffffffffffffffffffffffffffffff";
        let mut store = FrameStore::default();
        assert_eq!(store.generation(), 0);
        assert_eq!(store.token(), "");
        assert!(
            store.get(1, 0, "").is_none(),
            "nothing before the first capture"
        );

        store.replace(1, TOKEN.to_string(), vec![eager(1), eager(2)]);
        assert_eq!(store.len(), 2);
        assert_eq!(store.token(), TOKEN);
        assert!(store.get(1, 1, TOKEN).is_some());
        assert!(store.get(2, 1, TOKEN).is_some());
        assert!(store.get(3, 1, TOKEN).is_none(), "unknown display id");
        assert!(store.get(1, 0, TOKEN).is_none(), "stale generation");
        assert!(store.get(1, 2, TOKEN).is_none(), "future generation");
        assert!(store.get(1, 1, OTHER).is_none(), "wrong token is a 404");
        assert!(store.get(1, 1, "").is_none(), "empty token is a 404");
        assert!(
            store.get(1, 1, &TOKEN.to_ascii_uppercase()).is_none(),
            "token comparison is byte-exact"
        );

        store.replace(2, OTHER.to_string(), vec![eager(1)]);
        assert_eq!(store.len(), 1, "a new generation drops every old source");
        assert!(store.get(1, 1, TOKEN).is_none());
        assert!(
            store.get(1, 2, TOKEN).is_none(),
            "the old token dies with its generation"
        );
        assert!(store.get(1, 2, OTHER).is_some());

        store.clear();
        assert!(store.is_empty());
        assert_eq!(store.generation(), 2, "clear keeps the counter");
        assert!(store.get(1, 2, OTHER).is_none());
    }
}
