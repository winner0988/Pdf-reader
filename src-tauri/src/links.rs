//! Web links from a PDF, before anything is opened (MVP-12, docs/architecture/links.md).
//!
//! The frontend never hands the main process a URI: it names a link by id, the URI comes from
//! the worker again, and it is checked here once more. Only `http`, `https` and `mailto` pass.
//! What the system is given is the URL as a browser would read it, rewritten to plain ASCII
//! (host in punycode, everything else percent-encoded), so nothing hidden in the PDF's text and
//! nothing a command line could misread (spaces, quotes) reaches the program that opens it.

use ipc_contract::text::classify_uri;
use ipc_contract::types::{ErrorCode, IpcError, LinkPreview, LinkTarget};
use url::Url;

/// Characters left out of what is opened even where URL syntax allows them: they could be
/// misread by a program's command line (`"`, space) or are unusual enough in a real URL to be
/// an attempt at that (`<`, `>`, `\`, `^`, `` ` ``, `{`, `|`, `}`).
const ALWAYS_ENCODED: &[u8] = b" \"<>\\^`{|}";

fn not_openable(reason: &str) -> IpcError {
    IpcError {
        code: ErrorCode::InvalidArgument,
        message: format!("not an openable link: {reason}"),
    }
}

/// Checks a URI from a PDF and describes it for the confirmation. Fails for anything that is
/// not an `http`, `https` or `mailto` URL a browser could parse.
pub fn preview(uri: &str) -> Result<LinkPreview, IpcError> {
    if !matches!(classify_uri(uri), LinkTarget::Uri { .. }) {
        return Err(not_openable("scheme"));
    }
    // The WHATWG URL parser, as browsers use: what it makes of the text is what gets opened,
    // and so what the confirmation must describe.
    let url = Url::parse(uri).map_err(|_| not_openable("malformed"))?;
    let (host, ascii_host) = match url.scheme() {
        "http" | "https" => {
            let ascii = url.host_str().ok_or_else(|| not_openable("no host"))?;
            let unicode = match url.host() {
                Some(url::Host::Domain(_)) => url::quirks::domain_to_unicode(ascii),
                _ => ascii.to_owned(),
            };
            let differs = unicode != ascii;
            (Some(unicode), differs.then(|| ascii.to_owned()))
        }
        "mailto" => mail_domain(url.path()),
        _ => return Err(not_openable("scheme")),
    };
    Ok(LinkPreview {
        uri: uri.to_owned(),
        opens: plain_ascii(url.as_str()),
        host,
        ascii_host,
    })
}

/// The domain of the first address of a `mailto` URL, as it reads and (when different) in
/// punycode.
fn mail_domain(path: &str) -> (Option<String>, Option<String>) {
    let first = path.split(',').next().unwrap_or_default();
    let Some((_, domain)) = first.rsplit_once('@') else {
        return (None, None);
    };
    let domain = percent_decode(domain);
    if domain.is_empty() {
        return (None, None);
    }
    let ascii = url::quirks::domain_to_ascii(&domain);
    let differs = !ascii.is_empty() && ascii != domain;
    (Some(domain), differs.then_some(ascii))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes every byte that is not printable ASCII, and the few printable ones in
/// [`ALWAYS_ENCODED`].
fn plain_ascii(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_graphic() && !ALWAYS_ENCODED.contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_contract::types::{LinkArgs, OutlineLinkArgs};

    fn opens(uri: &str) -> String {
        preview(uri).unwrap().opens
    }

    #[test]
    fn only_web_and_mail_links_can_be_opened() {
        for uri in [
            "https://example.invalid/docs",
            "http://example.invalid",
            "HTTPS://Example.Invalid/A",
            "mailto:someone@example.invalid",
            "https://192.0.2.1:8443/x",
            "http://[2001:db8::1]/",
        ] {
            assert!(preview(uri).is_ok(), "{uri}");
        }
        for uri in [
            "javascript:app.alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "\\\\share.example.invalid\\x",
            "smb://share.example.invalid/x",
            "ms-msdt:/id PCWDiagnostic",
            "search-ms:query=x",
            "data:text/html,<script>1</script>",
            "https://exa mple.invalid/",
            "http://",
            "",
        ] {
            assert_eq!(
                preview(uri).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{uri}"
            );
        }
    }

    #[test]
    fn shows_the_host_that_will_be_contacted() {
        let link = preview("HTTPS://Example.Invalid/A").unwrap();
        assert_eq!(link.host.as_deref(), Some("example.invalid"));
        assert_eq!(link.ascii_host, None);
        assert_eq!(link.opens, "https://example.invalid/A");
        // The name before "@" is only a user name; the site is what follows it.
        let disguised = preview("https://bank.example@evil.example.invalid/login").unwrap();
        assert_eq!(disguised.host.as_deref(), Some("evil.example.invalid"));
    }

    #[test]
    fn an_internationalised_host_is_shown_as_it_reads_and_opened_in_punycode() {
        // Cyrillic "а" (U+0430) looks like Latin "a".
        let link = preview("https://\u{0430}pple.example.invalid/").unwrap();
        assert_eq!(link.host.as_deref(), Some("\u{0430}pple.example.invalid"));
        let ascii = link.ascii_host.unwrap();
        assert!(ascii.starts_with("xn--"), "{ascii}");
        assert_eq!(link.opens, format!("https://{ascii}/"));

        let mail = preview("mailto:someone@b\u{00FC}cher.example").unwrap();
        assert_eq!(mail.host.as_deref(), Some("b\u{00FC}cher.example"));
        assert_eq!(mail.ascii_host.as_deref(), Some("xn--bcher-kva.example"));
    }

    #[test]
    fn hidden_characters_never_reach_the_system() {
        let link = preview("https://example.invalid/\u{202E}fdp.exe").unwrap();
        assert_eq!(link.uri, "https://example.invalid/\u{202E}fdp.exe");
        assert_eq!(link.opens, "https://example.invalid/%E2%80%AEfdp.exe");
        // Line breaks and tabs are dropped by the URL parser: the host is the one it reads.
        let split = preview("https://example.invalid\n.evil.example/").unwrap();
        assert_eq!(split.host.as_deref(), Some("example.invalid.evil.example"));
    }

    #[test]
    fn nothing_a_command_line_could_misread_is_passed_on() {
        assert_eq!(
            opens("https://example.invalid/a\"b c?q=\"x y\"#\"z\""),
            "https://example.invalid/a%22b%20c?q=%22x%20y%22#%22z%22"
        );
        assert_eq!(
            opens("mailto:\"a b\"@example.invalid?subject=hi there"),
            "mailto:%22a%20b%22@example.invalid?subject=hi%20there"
        );
        for uri in [
            "https://example.invalid/\u{202E}\u{0000}\t{|}^`<>\\",
            "mailto:x@example.invalid?body=\u{7F}\u{00A0}\u{200B}",
            &format!("https://example.invalid/{}", "\u{4E2D}".repeat(3000)),
        ] {
            let opened = opens(uri);
            assert!(
                opened
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && !ALWAYS_ENCODED.contains(&byte)),
                "{opened}"
            );
        }
    }

    #[test]
    fn the_frontend_can_only_name_a_link_never_give_a_uri() {
        let args: LinkArgs = serde_json::from_value(serde_json::json!({
            "doc": 1, "link": { "pageIndex": 0, "index": 2 }
        }))
        .unwrap();
        assert_eq!(args.link.index, 2);
        for extra in [
            serde_json::json!({ "doc": 1, "link": { "pageIndex": 0, "index": 2 }, "uri": "file:///C:/x.exe" }),
            serde_json::json!({ "doc": 1, "uri": "https://example.invalid/" }),
            serde_json::json!({ "doc": 1, "link": "https://example.invalid/" }),
        ] {
            assert!(serde_json::from_value::<LinkArgs>(extra).is_err());
        }
        // The same for outline items (#49): a position, nothing else.
        assert!(
            serde_json::from_value::<OutlineLinkArgs>(serde_json::json!({ "doc": 1, "item": 0 }))
                .is_ok()
        );
        assert!(
            serde_json::from_value::<OutlineLinkArgs>(
                serde_json::json!({ "doc": 1, "item": 0, "uri": "file:///C:/x.exe" })
            )
            .is_err()
        );
    }
}
