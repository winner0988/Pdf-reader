//! WebView navigation allowlist (Issue #424, AGENTS.md principle 1 and principle 6).
//!
//! The main window's WebView must only navigate to the app's own pages:
//! - In production builds, only `http://tauri.localhost` (the custom scheme used by Tauri on Windows).
//! - In development builds (`cfg!(debug_assertions)`), `http://localhost:1420` (the Vite dev server).
//!
//! All other schemes (`https:`, `file:`, `data:`, `blob:`, `javascript:`, `about:`, etc.),
//! external hosts, fake domains, credentials, and unexpected ports are rejected.

use url::Url;

/// Checks whether the WebView is allowed to navigate to the given URL.
///
/// Returns `true` only for the app's own origin:
/// - Production (`dev == false`): `http://tauri.localhost` (no explicit port, no credentials).
/// - Development (`dev == true`): `http://localhost:1420` or `http://tauri.localhost`.
pub fn navigation_allowed(url: &Url, dev: bool) -> bool {
    // Reject any credentials (username or password).
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }

    if url.scheme() != "http" {
        return false;
    }

    let Some(host) = url.host_str() else {
        return false;
    };

    if host == "tauri.localhost" {
        return url.port().is_none();
    }

    if dev && host == "localhost" {
        return url.port() == Some(1420);
    }

    false
}

/// Checks whether navigation is allowed, logging blocked attempts to stderr.
///
/// In compliance with the offline verification log policy (`docs/security/offline-verification.md`),
/// only the scheme and host are logged—never the full path or query parameters, which could
/// contain sensitive document content.
pub fn check_navigation(url: &Url, dev: bool) -> bool {
    let allowed = navigation_allowed(url, dev);
    if !allowed {
        let host = url.host_str().unwrap_or("<none>");
        eprintln!(
            "blocked navigation to scheme '{}', host '{}'",
            url.scheme(),
            host
        );
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Url {
        Url::parse(s).unwrap_or_else(|e| panic!("invalid URL in test {s:?}: {e}"))
    }

    #[test]
    fn production_url_is_allowed() {
        assert!(navigation_allowed(&parse("http://tauri.localhost"), false));
        assert!(navigation_allowed(&parse("http://tauri.localhost/"), false));
        assert!(navigation_allowed(
            &parse("http://tauri.localhost/index.html"),
            false
        ));
        assert!(navigation_allowed(
            &parse("http://tauri.localhost/some/path?query=test#hash"),
            false
        ));

        // Also allowed in dev mode
        assert!(navigation_allowed(&parse("http://tauri.localhost"), true));
        assert!(navigation_allowed(&parse("http://tauri.localhost/"), true));
    }

    #[test]
    fn dev_server_url_is_allowed_only_in_dev() {
        let dev_url = parse("http://localhost:1420");
        let dev_url_path = parse("http://localhost:1420/index.html?token=1");

        assert!(navigation_allowed(&dev_url, true));
        assert!(navigation_allowed(&dev_url_path, true));

        // Rejected in production mode
        assert!(!navigation_allowed(&dev_url, false));
        assert!(!navigation_allowed(&dev_url_path, false));
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        let schemes = [
            "https://tauri.localhost/",
            "https://localhost:1420/",
            "https://example.invalid/",
            "file:///C:/Windows/win.ini",
            "file://localhost/test",
            "data:text/html,<h1>blocked</h1>",
            "blob:http://tauri.localhost/1234-5678",
            "javascript:alert(1)",
            "about:blank",
            "ws://localhost:1420",
            "wss://tauri.localhost",
            "ftp://tauri.localhost/",
        ];

        for uri in schemes {
            let url = parse(uri);
            assert!(
                !navigation_allowed(&url, false),
                "expected {uri} to be blocked in prod"
            );
            assert!(
                !navigation_allowed(&url, true),
                "expected {uri} to be blocked in dev"
            );
        }
    }

    #[test]
    fn fake_domains_are_rejected() {
        let fake_domains = [
            "http://tauri.localhost.evil.example/",
            "http://tauri.localhost.com/",
            "http://tauri.localhost-fake.net/",
            "http://tauri.localhost@evil.example/",
            "http://not-tauri.localhost/",
            "http://evil.example/?ref=tauri.localhost",
        ];

        for uri in fake_domains {
            let url = parse(uri);
            assert!(
                !navigation_allowed(&url, false),
                "expected {uri} to be blocked in prod"
            );
            assert!(
                !navigation_allowed(&url, true),
                "expected {uri} to be blocked in dev"
            );
        }
    }

    #[test]
    fn credentials_are_rejected() {
        let with_creds = [
            "http://user@tauri.localhost/",
            "http://user:pass@tauri.localhost/",
            "http://:pass@tauri.localhost/",
            "http://user@localhost:1420/",
            "http://user:pass@localhost:1420/",
        ];

        for uri in with_creds {
            let url = parse(uri);
            assert!(
                !navigation_allowed(&url, false),
                "expected {uri} to be blocked in prod"
            );
            assert!(
                !navigation_allowed(&url, true),
                "expected {uri} to be blocked in dev"
            );
        }
    }

    #[test]
    fn unexpected_ports_are_rejected() {
        let bad_ports = [
            "http://tauri.localhost:8080/",
            "http://tauri.localhost:1420/",
            "http://tauri.localhost:3000/",
            "http://localhost:8080/",
            "http://localhost:3000/",
            "http://localhost/", // default port 80, not 1420
        ];

        for uri in bad_ports {
            let url = parse(uri);
            assert!(
                !navigation_allowed(&url, false),
                "expected {uri} to be blocked in prod"
            );
            assert!(
                !navigation_allowed(&url, true),
                "expected {uri} to be blocked in dev"
            );
        }
    }

    #[test]
    fn other_loopback_hosts_are_rejected() {
        let loopbacks = [
            "http://127.0.0.1:1420/",
            "http://127.0.0.1/",
            "http://[::1]:1420/",
        ];

        for uri in loopbacks {
            let url = parse(uri);
            assert!(
                !navigation_allowed(&url, false),
                "expected {uri} to be blocked in prod"
            );
            assert!(
                !navigation_allowed(&url, true),
                "expected {uri} to be blocked in dev"
            );
        }
    }
}
