//! The update check (#64): the app's one network request (ADR 0009, option B,
//! docs/architecture/update-check.md).
//!
//! - Only when the user presses 「檢查更新」 in the settings: one request per press. Never at
//!   start-up, on a schedule or again after a failure, and one at a time.
//! - Only from the main process, only to a fixed HTTPS address: this project's latest release on
//!   GitHub's API. The page gives no address, nor anything else that goes into the request.
//! - One GET with a fixed `User-Agent` and `Accept`, as GitHub asks. No cookie, credential,
//!   identifier, document or usage data: each check is a new WinHTTP session, told to handle no
//!   cookies, never to log on and not to follow redirects.
//! - Only the release's version number is read from the answer. The page offered for the
//!   download is a fixed address too ([`RELEASES_PAGE`]), opened like any web link: only after
//!   the user confirms it.
//!
//! WinHTTP is part of Windows, so the app has no HTTP or TLS library of its own (deny.toml still
//! bans them all); scripts/ci/forbidden-patterns.txt keeps WinHTTP to this module.

// WinHTTP; see the SAFETY comments in `fetch`.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

use ipc_contract::types::{ErrorCode, IpcError, UpdateCheck};
use serde::Deserialize;

/// Where a request goes. Only [`GITHUB`] is ever asked; the tests ask a local server.
struct Target {
    host: &'static str,
    port: u16,
    path: &'static str,
    https: bool,
}

/// This project's latest release (drafts and pre-releases are never "latest").
const GITHUB: Target = Target {
    host: "api.github.com",
    port: 443,
    path: "/repos/winner0988/Pdf-reader/releases/latest",
    https: true,
};

/// The page a new release is downloaded from.
pub const RELEASES_PAGE: &str = "https://github.com/winner0988/Pdf-reader/releases/latest";

/// The only headers this code sends: GitHub refuses requests without a `User-Agent`, and asks
/// for this `Accept`. Neither says anything about the user, the computer or the app's version.
/// WinHTTP adds only what HTTP itself needs (`Host`, `Connection`).
const USER_AGENT: &str = "Pdf-reader";
const ACCEPT: &str = "Accept: application/vnd.github+json";

/// The most of the answer read; GitHub's is a few kilobytes.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Milliseconds for each step: finding the name, connecting, sending, and each wait for data.
const TIMEOUT_MS: i32 = 15_000;

/// Whether a check is running: a second one is refused rather than sent too.
static CHECKING: AtomicBool = AtomicBool::new(false);

/// Marks a check as running until dropped.
struct Running;

impl Running {
    fn start() -> Option<Self> {
        // `then`, not `then_some`: a `Running` made and dropped here would clear the flag.
        (!CHECKING.swap(true, Ordering::AcqRel)).then(|| Running)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        CHECKING.store(false, Ordering::Release);
    }
}

fn failed(reason: &str) -> IpcError {
    IpcError {
        code: ErrorCode::NetworkFailed,
        message: format!("update check: {reason}"),
    }
}

/// Asks GitHub for the latest release and compares it with `current`, this app's version.
pub fn check(current: &str) -> Result<UpdateCheck, IpcError> {
    check_at(&GITHUB, current)
}

fn check_at(target: &Target, current: &str) -> Result<UpdateCheck, IpcError> {
    let Some(_running) = Running::start() else {
        return Err(IpcError {
            code: ErrorCode::InvalidArgument,
            message: "an update check is already running".to_owned(),
        });
    };
    let (status, body) = fetch(target)?;
    interpret(current, status, &body)
}

/// What GitHub's answer (`status`, `body`) means for `current`.
fn interpret(current: &str, status: u32, body: &[u8]) -> Result<UpdateCheck, IpcError> {
    let current_version = version(current).ok_or_else(|| failed("the app's version"))?;
    let current = current.to_owned();
    match status {
        200 => {}
        // The project has not published a release yet.
        404 => return Ok(UpdateCheck::NoRelease { current }),
        _ => return Err(failed(&format!("GitHub answered {status}"))),
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release =
        serde_json::from_slice(body).map_err(|_| failed("the answer is not a release"))?;
    let latest = version(&release.tag_name).ok_or_else(|| failed("the tag is not a version"))?;
    Ok(if latest > current_version {
        // Rewritten from the numbers: nothing else of the answer reaches the page.
        let [major, minor, patch] = latest;
        UpdateCheck::Available {
            current,
            latest: format!("{major}.{minor}.{patch}"),
        }
    } else {
        UpdateCheck::UpToDate { current }
    })
}

/// `major.minor.patch`, with an optional `v` before it (as in the tag `v1.2.3`): three numbers of
/// at most five digits, nothing else.
fn version(text: &str) -> Option<[u32; 3]> {
    let mut fields = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut version = [0; 3];
    for number in &mut version {
        let field = fields.next()?;
        if field.is_empty() || field.len() > 5 || !field.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *number = field.parse().ok()?;
    }
    fields.next().is_none().then_some(version)
}

/// A WinHTTP handle, closed when dropped.
#[cfg(windows)]
struct Handle(*mut std::ffi::c_void);

#[cfg(windows)]
impl Handle {
    fn new(raw: *mut std::ffi::c_void, step: &str) -> Result<Self, IpcError> {
        if raw.is_null() {
            Err(failed(step))
        } else {
            Ok(Self(raw))
        }
    }

    /// Sets a numeric option of this handle; false if WinHTTP refuses it.
    fn set(&self, option: u32, value: u32) -> bool {
        // SAFETY: a live handle, and a buffer of the size given, which outlives the call.
        unsafe {
            windows_sys::Win32::Networking::WinHttp::WinHttpSetOption(
                self.0,
                option,
                (&raw const value).cast(),
                size_of::<u32>() as u32,
            ) != 0
        }
    }
}

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle WinHTTP returned, closed once; a request's handle is dropped before
        // its connection's, and that one before the session's.
        unsafe { windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0) };
    }
}

/// Sends the GET and reads the answer: its status and at most [`MAX_BODY_BYTES`] of body.
#[cfg(windows)]
fn fetch(target: &Target) -> Result<(u32, Vec<u8>), IpcError> {
    use std::ptr;

    use windows_sys::Win32::Networking::WinHttp::*;

    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let (agent, host, verb, path, accept) = (
        wide(USER_AGENT),
        wide(target.host),
        wide("GET"),
        wide(target.path),
        wide(ACCEPT),
    );

    // SAFETY, for every call below: the strings are NUL-terminated UTF-16 that outlive the
    // calls; handles are only used while their `Handle` lives (declared parent first, so dropped
    // child first); every buffer is passed with its true length.
    unsafe {
        // The proxy an administrator set for WinHTTP (`netsh winhttp`), if any. No automatic
        // proxy discovery: it would send requests of its own on the local network.
        let session = Handle::new(
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        // TLS 1.2 or 1.3 only; Windows 10's WinHTTP has no 1.3.
        let modern = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        if !session.set(WINHTTP_OPTION_SECURE_PROTOCOLS, modern)
            && !session.set(
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
            )
        {
            return Err(failed("TLS versions"));
        }
        if WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS) == 0 {
            return Err(failed("timeouts"));
        }
        let connection = Handle::new(
            WinHttpConnect(session.0, host.as_ptr(), target.port, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                if target.https { WINHTTP_FLAG_SECURE } else { 0 },
            ),
            "WinHttpOpenRequest",
        )?;
        // The request and nothing else: no cookies, no logging on, no redirects, and the
        // connection closed afterwards.
        let disabled = WINHTTP_DISABLE_COOKIES
            | WINHTTP_DISABLE_AUTHENTICATION
            | WINHTTP_DISABLE_REDIRECTS
            | WINHTTP_DISABLE_KEEP_ALIVE;
        if !request.set(WINHTTP_OPTION_DISABLE_FEATURE, disabled)
            || !request.set(
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
            )
        {
            return Err(failed("request options"));
        }
        // -1: the header is NUL-terminated.
        if WinHttpAddRequestHeaders(
            request.0,
            accept.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        ) == 0
        {
            return Err(failed("headers"));
        }
        if WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0
        {
            return Err(failed("no answer"));
        }
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            (&raw mut status).cast(),
            &mut size,
            ptr::null_mut(),
        ) == 0
        {
            return Err(failed("status"));
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            ) == 0
            {
                return Err(failed("reading the answer"));
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_BODY_BYTES {
                return Err(failed("the answer is too large"));
            }
        }
        Ok((status, body))
    }
}

#[cfg(not(windows))]
fn fetch(_target: &Target) -> Result<(u32, Vec<u8>), IpcError> {
    Err(failed("only on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = "0.1.0";

    fn release(tag: &str) -> Vec<u8> {
        format!(r#"{{"tag_name": "{tag}", "name": "anything", "assets": []}}"#).into_bytes()
    }

    #[test]
    fn reads_versions_and_nothing_else() {
        assert_eq!(version("1.2.3"), Some([1, 2, 3]));
        assert_eq!(version("v10.20.30"), Some([10, 20, 30]));
        assert_eq!(version("v0.1.0"), Some([0, 1, 0]));
        for text in [
            "",
            "v",
            "1.2",
            "1.2.3.4",
            "1..3",
            "v1.2.3-beta",
            "1.2.3 ",
            " 1.2.3",
            "+1.2.3",
            "1.2.-3",
            "vv1.2.3",
            "１.２.３",
            "123456.0.0",
            "<script>.1.2",
        ] {
            assert_eq!(version(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_newer_release_is_offered_and_an_older_or_the_same_is_not() {
        assert_eq!(
            interpret(CURRENT, 200, &release("v0.2.0")).unwrap(),
            UpdateCheck::Available {
                current: CURRENT.to_owned(),
                latest: "0.2.0".to_owned()
            }
        );
        // Numbers compare as numbers, not text.
        assert_eq!(
            interpret("0.9.0", 200, &release("v0.10.0")).unwrap(),
            UpdateCheck::Available {
                current: "0.9.0".to_owned(),
                latest: "0.10.0".to_owned()
            }
        );
        for tag in ["v0.1.0", "0.1.0", "v0.0.9"] {
            assert_eq!(
                interpret(CURRENT, 200, &release(tag)).unwrap(),
                UpdateCheck::UpToDate {
                    current: CURRENT.to_owned()
                },
                "{tag}"
            );
        }
    }

    #[test]
    fn no_release_yet_is_said_so() {
        assert_eq!(
            interpret(CURRENT, 404, br#"{"message": "Not Found"}"#).unwrap(),
            UpdateCheck::NoRelease {
                current: CURRENT.to_owned()
            }
        );
    }

    #[test]
    fn anything_unexpected_is_a_failure() {
        for (status, body) in [
            (200, b"not json".to_vec()),
            (200, br#"{"name": "no tag"}"#.to_vec()),
            (200, release("latest")),
            (200, release("v1.2.3\u{202E}")),
            (403, br#"{"message": "API rate limit exceeded"}"#.to_vec()),
            (500, Vec::new()),
            (302, Vec::new()),
        ] {
            let error = interpret(CURRENT, status, &body).unwrap_err();
            assert_eq!(error.code, ErrorCode::NetworkFailed, "{status}");
        }
        // The app's own version must be one too.
        assert!(interpret("0.1", 200, &release("v1.0.0")).is_err());
    }

    #[test]
    fn only_the_fixed_address_is_ever_asked() {
        assert_eq!(
            (GITHUB.host, GITHUB.port, GITHUB.https, GITHUB.path),
            (
                "api.github.com",
                443,
                true,
                "/repos/winner0988/Pdf-reader/releases/latest"
            )
        );
        let page = crate::links::preview(RELEASES_PAGE).expect("an openable link");
        assert_eq!(page.host.as_deref(), Some("github.com"));
        assert_eq!(
            page.opens,
            "https://github.com/winner0988/Pdf-reader/releases/latest"
        );
    }

    #[test]
    fn one_check_at_a_time() {
        // Were a request sent anyway, it would go nowhere: a local port no one listens on.
        let nowhere = Target {
            host: "127.0.0.1",
            port: 1,
            path: "/",
            https: false,
        };
        let first = Running::start().expect("no check is running");
        assert!(Running::start().is_none());
        assert!(
            Running::start().is_none(),
            "a refused start cleared the flag"
        );
        assert_eq!(
            check_at(&nowhere, CURRENT).unwrap_err().code,
            ErrorCode::InvalidArgument,
            "refused without sending anything"
        );
        drop(first);
        let again = Running::start();
        assert!(again.is_some());
    }

    /// WinHTTP as configured by `fetch`, against a local HTTP server: what it sends, and what it
    /// does not do.
    #[cfg(windows)]
    mod wire {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        use super::*;

        /// Answers each connection with the next of `answers` and reports the requests it got,
        /// with their headers in lower case.
        fn server(answers: Vec<&'static str>) -> (u16, mpsc::Receiver<String>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("address").port();
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                for answer in answers {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let request = read_request(&mut stream);
                    let _ = sender.send(request);
                    let _ = stream.write_all(answer.as_bytes());
                }
            });
            (port, receiver)
        }

        fn read_request(stream: &mut TcpStream) -> String {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("timeout");
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => request.push(byte[0]),
                    _ => break,
                }
            }
            String::from_utf8_lossy(&request).to_ascii_lowercase()
        }

        fn local(port: u16) -> Target {
            Target {
                host: "127.0.0.1",
                port,
                path: "/repos/winner0988/Pdf-reader/releases/latest",
                https: false,
            }
        }

        fn ok(body: &str) -> String {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Set-Cookie: session=secret\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }

        #[test]
        fn sends_one_get_with_only_the_fixed_headers() {
            let answer: &'static str = Box::leak(ok(r#"{"tag_name":"v9.9.9"}"#).into_boxed_str());
            let (port, requests) = server(vec![answer, answer]);
            for _ in 0..2 {
                let (status, body) = fetch(&local(port)).expect("fetch");
                assert_eq!(status, 200);
                assert_eq!(
                    interpret(CURRENT, status, &body).unwrap(),
                    UpdateCheck::Available {
                        current: CURRENT.to_owned(),
                        latest: "9.9.9".to_owned()
                    }
                );
            }
            // Each check sends exactly this; the second has no cookie from the first.
            for _ in 0..2 {
                let request = requests.recv().expect("a request");
                let mut lines = request.lines();
                assert_eq!(
                    lines.next(),
                    Some("get /repos/winner0988/pdf-reader/releases/latest http/1.1")
                );
                let mut names: Vec<&str> = lines
                    .filter(|line| !line.is_empty())
                    .map(|line| line.split(':').next().unwrap_or_default())
                    .collect();
                names.sort_unstable();
                assert_eq!(
                    names,
                    ["accept", "connection", "host", "user-agent"],
                    "{request}"
                );
                assert!(
                    request.contains("\r\nuser-agent: pdf-reader\r\n"),
                    "{request}"
                );
                assert!(
                    request.contains("\r\naccept: application/vnd.github+json\r\n"),
                    "{request}"
                );
            }
        }

        #[test]
        fn does_not_follow_a_redirect() {
            let (port, requests) = server(vec![
                "HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\
                 Connection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ]);
            let (status, _) = fetch(&local(port)).expect("fetch");
            assert_eq!(status, 302);
            assert!(interpret(CURRENT, status, &[]).is_err());
            requests.recv().expect("the request");
            assert!(
                requests.recv_timeout(Duration::from_millis(500)).is_err(),
                "a second request was sent"
            );
        }

        #[test]
        fn never_logs_on() {
            let (port, requests) = server(vec![
                "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Negotiate\r\n\
                 WWW-Authenticate: NTLM\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ]);
            let (status, _) = fetch(&local(port)).expect("fetch");
            assert_eq!(status, 401);
            let request = requests.recv().expect("the request");
            assert!(!request.contains("authorization"), "{request}");
            assert!(
                requests.recv_timeout(Duration::from_millis(500)).is_err(),
                "it tried to log on"
            );
        }

        #[test]
        fn stops_reading_a_too_large_answer() {
            let large: &'static str =
                Box::leak(ok(&"x".repeat(MAX_BODY_BYTES + 1)).into_boxed_str());
            let (port, _requests) = server(vec![large]);
            let error = fetch(&local(port)).unwrap_err();
            assert_eq!(error.code, ErrorCode::NetworkFailed);
        }

        #[test]
        fn no_server_is_a_failure() {
            let port = {
                let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
                listener.local_addr().expect("address").port()
            };
            let error = fetch(&local(port)).unwrap_err();
            assert_eq!(error.code, ErrorCode::NetworkFailed);
        }
    }
}
