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
        "mailto" => mail_domains(&url),
        _ => return Err(not_openable("scheme")),
    };
    Ok(LinkPreview {
        uri: uri.to_owned(),
        opens: plain_ascii(url.as_str()),
        host,
        ascii_host,
    })
}

/// The domains of the recipients of a `mailto` URL, as they read and (when different) in
/// punycode.
fn mail_domains(url: &Url) -> (Option<String>, Option<String>) {
    let mut recipients = Vec::new();
    let decoded_path = percent_decode(url.path());
    for slice in decoded_path.split([',', ';']) {
        recipients.push(slice.to_owned());
    }
    if let Some(query) = url.query() {
        for part in query.split('&') {
            let (name, val) = part.split_once('=').unwrap_or((part, ""));
            let decoded_name = percent_decode(name);
            if decoded_name.eq_ignore_ascii_case("to")
                || decoded_name.eq_ignore_ascii_case("cc")
                || decoded_name.eq_ignore_ascii_case("bcc")
            {
                let decoded_val = percent_decode(val);
                for slice in decoded_val.split([',', ';']) {
                    recipients.push(slice.to_owned());
                }
            }
        }
    }

    let mut domains: Vec<String> = Vec::new();
    for recipient in recipients {
        let trimmed = recipient.trim();
        if let Some((_, domain)) = trimmed.rsplit_once('@')
            && !domain.is_empty()
            && !domains.iter().any(|d| d.eq_ignore_ascii_case(domain))
        {
            domains.push(domain.to_owned());
        }
    }

    if domains.is_empty() {
        return (None, None);
    }

    let mut any_different = false;
    let mut ascii_domains = Vec::with_capacity(domains.len());
    for domain in &domains {
        let ascii = url::quirks::domain_to_ascii(domain);
        if !ascii.is_empty() && !ascii.eq_ignore_ascii_case(domain) {
            any_different = true;
            ascii_domains.push(ascii);
        } else {
            ascii_domains.push(domain.clone());
        }
    }

    let host = domains.join(", ");
    let ascii_host = any_different.then(|| ascii_domains.join(", "));
    (Some(host), ascii_host)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .filter(|pair| pair.iter().all(|b| b.is_ascii_hexdigit()))
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

    #[test]
    fn a_mail_link_shows_every_recipients_domain() {
        let cases = [
            (
                "mailto:someone@first.example,ceo@second.example",
                "first.example, second.example",
                "mailto:someone@first.example,ceo@second.example",
            ),
            (
                "mailto:someone@first.example;ceo@second.example",
                "first.example, second.example",
                "mailto:someone@first.example;ceo@second.example",
            ),
            (
                "mailto:someone@first.example%2Cceo@second.example",
                "first.example, second.example",
                "mailto:someone@first.example%2Cceo@second.example",
            ),
            (
                "mailto:someone@first.example%3Bceo@second.example",
                "first.example, second.example",
                "mailto:someone@first.example%3Bceo@second.example",
            ),
            (
                "mailto:?to=someone@first.example",
                "first.example",
                "mailto:?to=someone@first.example",
            ),
            (
                "mailto:a@first.example?cc=b@second.example&bcc=c@third.example",
                "first.example, second.example, third.example",
                "mailto:a@first.example?cc=b@second.example&bcc=c@third.example",
            ),
            (
                "mailto:a@first.example?TO=b@second.example",
                "first.example, second.example",
                "mailto:a@first.example?TO=b@second.example",
            ),
            (
                "mailto:a@first.example,b@FIRST.example;c@second.example",
                "first.example, second.example",
                "mailto:a@first.example,b@FIRST.example;c@second.example",
            ),
            (
                "mailto:a@first.example?subject=x@evil.example&body=y@evil.example",
                "first.example",
                "mailto:a@first.example?subject=x@evil.example&body=y@evil.example",
            ),
        ];
        for (uri, host, expected_opens) in cases {
            let p = preview(uri).unwrap();
            assert_eq!(p.host.as_deref(), Some(host), "{uri}");
            assert_eq!(p.opens, expected_opens, "{uri}");
        }
    }

    #[test]
    fn a_mail_domain_never_hides_a_character() {
        let p1 = preview("mailto:x@second.example%+0").unwrap();
        assert_eq!(p1.host.as_deref(), Some("second.example%+0"));
        assert!(!p1.host.as_deref().unwrap().chars().any(char::is_control));
        assert_eq!(p1.opens, "mailto:x@second.example%+0");

        let p2 = preview("mailto:x@%+4second.example").unwrap();
        assert_eq!(p2.host.as_deref(), Some("%+4second.example"));
        assert!(!p2.host.as_deref().unwrap().chars().any(char::is_control));
        assert_eq!(p2.opens, "mailto:x@%+4second.example");

        let p3 = preview("mailto:x@second.example%00").unwrap();
        assert_eq!(p3.host.as_deref(), Some("second.example\u{0}"));
        assert_eq!(p3.opens, "mailto:x@second.example%00");

        let p4 = preview("mailto:x@Example.COM").unwrap();
        assert_eq!(p4.ascii_host, None);
        assert_eq!(p4.opens, "mailto:x@Example.COM");
    }
}

#[cfg(test)]
mod properties {
    use super::*;

    const SEED_GENERAL: u64 = 0x616_0001;
    const CASES_GENERAL: usize = 10_000;

    const SEED_MAILTO: u64 = 0x616_0002;
    const CASES_MAILTO: usize = 5_000;

    struct Rng {
        state: u64,
    }

    impl Rng {
        fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }

        fn next_usize(&mut self, bound: usize) -> usize {
            if bound == 0 {
                return 0;
            }
            (self.next_u64() % (bound as u64)) as usize
        }

        fn choose<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
            &slice[self.next_usize(slice.len())]
        }

        fn gen_bool(&mut self) -> bool {
            (self.next_u64() & 1) == 1
        }
    }

    fn shrink_failing_input<F>(mut current: String, mut still_fails: F) -> String
    where
        F: FnMut(&str) -> bool,
    {
        let mut changed = true;
        while changed {
            changed = false;
            let char_indices: Vec<usize> = current.char_indices().map(|(i, _)| i).collect();
            let n_chars = char_indices.len();
            if n_chars == 0 {
                break;
            }
            let mut chunk_size = n_chars / 2;
            loop {
                let size = chunk_size.max(1);
                let mut i = 0;
                while i + size <= n_chars {
                    let start_byte = char_indices[i];
                    let end_byte = if i + size < n_chars {
                        char_indices[i + size]
                    } else {
                        current.len()
                    };
                    let mut candidate =
                        String::with_capacity(current.len() - (end_byte - start_byte));
                    candidate.push_str(&current[..start_byte]);
                    candidate.push_str(&current[end_byte..]);
                    if still_fails(&candidate) {
                        current = candidate;
                        changed = true;
                        break;
                    }
                    i += 1;
                }
                if changed || size == 1 {
                    break;
                }
                chunk_size /= 2;
            }
        }
        current
    }

    /// Independent WHATWG percent-decode for test verification (#616).
    /// Decodes `%XX` only when followed by two ASCII hex digits; otherwise keeps bytes as-is.
    /// Result is interpreted as UTF-8 (lossy).
    fn test_whatwg_percent_decode(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let h1 = bytes[i + 1];
                let h2 = bytes[i + 2];
                if h1.is_ascii_hexdigit() && h2.is_ascii_hexdigit() {
                    let v1 = match h1 {
                        b'0'..=b'9' => h1 - b'0',
                        b'a'..=b'f' => h1 - b'a' + 10,
                        b'A'..=b'F' => h1 - b'A' + 10,
                        _ => 0,
                    };
                    let v2 = match h2 {
                        b'0'..=b'9' => h2 - b'0',
                        b'a'..=b'f' => h2 - b'a' + 10,
                        b'A'..=b'F' => h2 - b'A' + 10,
                        _ => 0,
                    };
                    decoded.push((v1 << 4) | v2);
                    i += 3;
                    continue;
                }
            }
            decoded.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&decoded).into_owned()
    }

    fn gen_random_scheme(rng: &mut Rng) -> String {
        let base_schemes = [
            "http",
            "https",
            "mailto",
            "file",
            "javascript",
            "data",
            "smb",
            "ftp",
            "ms-settings",
        ];
        let name = if rng.next_usize(10) < 8 {
            (*rng.choose(&base_schemes)).to_string()
        } else {
            let len = 1 + rng.next_usize(6);
            (0..len)
                .map(|_| {
                    if rng.gen_bool() {
                        (b'a' + rng.next_usize(26) as u8) as char
                    } else {
                        (b'A' + rng.next_usize(26) as u8) as char
                    }
                })
                .collect()
        };
        name.chars()
            .map(|c| {
                if rng.gen_bool() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    }

    fn gen_general_url(rng: &mut Rng) -> String {
        let has_scheme = rng.next_usize(20) > 0;
        let mut out = String::new();
        if has_scheme {
            out.push_str(&gen_random_scheme(rng));
            let sep = *rng.choose(&[":", "://", ":/\\", ":\\\\", ":/", ":\\", "::", ""]);
            out.push_str(sep);
        }

        let fragments = [
            "%20",
            "%2E",
            "%41",
            "%ff",
            "%00",
            "%+0",
            "%G1",
            "%4",
            "%",
            "%%",
            "@",
            ",",
            ";",
            "?",
            "#",
            "&",
            "=",
            "\\",
            "/",
            " ",
            "\t",
            "\r",
            "\n",
            "\u{0}",
            "\u{1}",
            "\u{1F}",
            "\u{7F}",
            "\u{200B}",
            "例子",
            "bücher",
            "xn--",
            "xn--fsqu00a",
            "[::1]",
            "[2001:db8::1]",
            "[::]",
            ":80",
            ":443",
            ":8080",
            "localhost",
            "LOCALHOST",
            "LocalHost",
            "localhost.",
            "localhost.example",
            "localhostserver",
            "localhost:445",
            "%6Cocalhost",
            "127.0.0.1",
            "a",
            "B",
            "123",
            "foo",
            "bar",
            "doc.pdf",
        ];

        let target_len = rng.next_usize(201);
        while out.chars().count() < target_len {
            let choice = rng.next_usize(fragments.len() + 2);
            let piece = if choice < fragments.len() {
                fragments[choice].to_string()
            } else if choice == fragments.len() {
                let b = rng.next_usize(32) as u8;
                (b as char).to_string()
            } else {
                let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                (*rng.choose(chars) as char).to_string()
            };
            if out.chars().count() + piece.chars().count() > target_len && !out.is_empty() {
                break;
            }
            out.push_str(&piece);
        }
        out
    }

    fn check_properties_1_to_6(uri: &str) -> Result<(), String> {
        let preview_attempt =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(uri)));
        let preview_res = match preview_attempt {
            Ok(r) => r,
            Err(_) => {
                // Property 6: no panic
                return Err(format!("Property 6: preview panicked on input {uri:?}"));
            }
        };

        let p = match preview_res {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };

        // Property 1: Url::parse(opens) scheme is http, https, or mailto
        let parsed_opens = Url::parse(&p.opens)
            .map_err(|e| format!("Property 1: Url::parse(opens={:?}) failed: {e}", p.opens))?;
        let opens_scheme = parsed_opens.scheme();
        if !matches!(opens_scheme, "http" | "https" | "mailto") {
            return Err(format!(
                "Property 1: opens scheme is {opens_scheme:?}, expected http/https/mailto"
            ));
        }

        // Property 2: Url::parse(opens).scheme() == Url::parse(input).scheme()
        let parsed_input = Url::parse(uri)
            .map_err(|e| format!("Property 2: Url::parse(input={:?}) failed: {e}", uri))?;
        if opens_scheme != parsed_input.scheme() {
            return Err(format!(
                "Property 2: opens scheme {opens_scheme:?} != input scheme {:?}",
                parsed_input.scheme()
            ));
        }

        // Property 3: opens contains only visible ASCII and no ALWAYS_ENCODED characters
        for &byte in p.opens.as_bytes() {
            if !byte.is_ascii_graphic() || ALWAYS_ENCODED.contains(&byte) {
                return Err(format!(
                    "Property 3: opens contains invalid byte {byte:#04X} ({:?})",
                    byte as char
                ));
            }
        }

        // Property 4: for http/https, host matches Url::parse(opens) host converted to Unicode
        if matches!(opens_scheme, "http" | "https") {
            let ascii = parsed_opens
                .host_str()
                .ok_or_else(|| format!("Property 4: no host in parsed opens {:?}", p.opens))?;
            let expected_host = match parsed_opens.host() {
                Some(url::Host::Domain(_)) => url::quirks::domain_to_unicode(ascii),
                _ => ascii.to_owned(),
            };
            if p.host.as_deref() != Some(expected_host.as_str()) {
                return Err(format!(
                    "Property 4: host was {:?}, expected {expected_host:?}",
                    p.host
                ));
            }
        }

        // Property 5: preview(opens) is idempotent: opens and host do not change
        let second_attempt =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(&p.opens)));
        let second_res = match second_attempt {
            Ok(r) => r,
            Err(_) => {
                return Err(format!("Property 5: preview(opens={:?}) panicked", p.opens));
            }
        };
        let second = second_res
            .map_err(|e| format!("Property 5: preview(opens={:?}) failed: {e:?}", p.opens))?;
        if second.opens != p.opens {
            return Err(format!(
                "Property 5: opens changed from {:?} to {:?}",
                p.opens, second.opens
            ));
        }
        if second.host != p.host {
            return Err(format!(
                "Property 5: host changed from {:?} to {:?}",
                p.host, second.host
            ));
        }

        Ok(())
    }

    #[test]
    fn properties_1_to_6_preview_general() {
        let mut rng = Rng::new(SEED_GENERAL);
        for case in 0..CASES_GENERAL {
            let input = gen_general_url(&mut rng);
            if let Err(err) = check_properties_1_to_6(&input) {
                let shrunk = shrink_failing_input(input.clone(), |cand| {
                    check_properties_1_to_6(cand).is_err()
                });
                panic!(
                    "Property violated!\nSeed: {SEED_GENERAL:#X}\nCase: {case}\nError: {err}\nOriginal: {:?}\nShrunk:   {:?}",
                    input, shrunk
                );
            }
        }
    }

    fn gen_mailto_recipient(rng: &mut Rng) -> (String, String) {
        let local_chars = b"abcdefghijklmnopqrstuvwxyz0123456789._+-";
        let local_len = 1 + rng.next_usize(8);
        let local: String = (0..local_len)
            .map(|_| *rng.choose(local_chars) as char)
            .collect();

        let domain_pieces = [
            "example",
            "invalid",
            "test",
            "org",
            "co",
            "uk",
            "sub",
            "mail",
            "EXAMPLE",
            "Invalid",
            "Bücher",
            "例子",
            "\u{0430}pple",
            // Valid %XX (excluding 2C, 3B, 40, 26, 3D, 3F, 23)
            "%61",
            "%62",
            "%C3%BC",
            "%E4%BE%8B",
            "%00",
            "%ff",
            "%2E",
            // Invalid %XX
            "%+0",
            "%G1",
            "%4",
            "%",
            "%zz",
            ".",
            "-",
        ];
        let n_pieces = 1 + rng.next_usize(4);
        let mut domain = String::new();
        for _ in 0..n_pieces {
            let choice = rng.next_usize(domain_pieces.len() + 2);
            if choice < domain_pieces.len() {
                domain.push_str(domain_pieces[choice]);
            } else if choice == domain_pieces.len() {
                let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                domain.push(*rng.choose(chars) as char);
            } else {
                let forbidden = [
                    0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x2C, 0x3B, 0x40, 0x26, 0x3D, 0x3F, 0x23,
                ];
                let mut byte = rng.next_usize(256) as u8;
                while forbidden.contains(&byte) {
                    byte = rng.next_usize(256) as u8;
                }
                domain.push_str(&format!("%{byte:02X}"));
            }
        }
        if domain.trim().is_empty() {
            domain.push_str("example.com");
        }

        let expected_domain = test_whatwg_percent_decode(&domain);
        let recipient_str = format!("{local}@{domain}");
        (recipient_str, expected_domain)
    }

    fn gen_mailto_url(rng: &mut Rng) -> (String, Vec<String>) {
        let n_recipients = 1 + rng.next_usize(4);
        let mut recipients = Vec::with_capacity(n_recipients);
        let mut expected_domains = Vec::with_capacity(n_recipients);
        for _ in 0..n_recipients {
            let (rec, dom) = gen_mailto_recipient(rng);
            recipients.push(rec);
            expected_domains.push(dom);
        }

        let seps = [",", ";", "%2C", "%3B", "%2c", "%3b"];

        let n_path = rng.next_usize(n_recipients + 1);
        let path_recipients = &recipients[..n_path];
        let query_recipients = &recipients[n_path..];

        let mut path_str = String::new();
        for (i, rec) in path_recipients.iter().enumerate() {
            if i > 0 {
                path_str.push_str(rng.choose(&seps));
            }
            path_str.push_str(rec);
        }

        let mut query_params: Vec<String> = Vec::new();
        if !query_recipients.is_empty() {
            let param_names = ["to", "To", "TO", "cc", "Cc", "CC", "bcc", "Bcc", "BCC"];
            let mut i = 0;
            while i < query_recipients.len() {
                let take = 1 + rng.next_usize(query_recipients.len() - i);
                let name = *rng.choose(&param_names);
                let mut val = String::new();
                for (j, rec) in query_recipients[i..i + take].iter().enumerate() {
                    if j > 0 {
                        val.push_str(rng.choose(&seps));
                    }
                    val.push_str(rec);
                }
                query_params.push(format!("{name}={val}"));
                i += take;
            }
        }

        if rng.next_usize(5) == 0 {
            query_params.push("subject=Test%20Mail&body=Hello".to_string());
        }

        let mut url = format!("mailto:{path_str}");
        if !query_params.is_empty() {
            url.push('?');
            url.push_str(&query_params.join("&"));
        }

        (url, expected_domains)
    }

    fn check_property_7(url: &str, expected_domains: &[String]) -> Result<(), (String, String)> {
        let preview_attempt =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(url)));
        let preview_res = match preview_attempt {
            Ok(r) => r,
            Err(_) => {
                return Err((
                    format!("Property 6/7: preview panicked on {url:?}"),
                    expected_domains.first().cloned().unwrap_or_default(),
                ));
            }
        };

        let p = preview_res.map_err(|e| {
            (
                format!("Property 7: preview({url:?}) failed: {e:?}"),
                expected_domains.first().cloned().unwrap_or_default(),
            )
        })?;

        let host_str = p.host.as_deref().unwrap_or("");
        let actual_domains: Vec<&str> = host_str.split(", ").collect();

        for expected in expected_domains {
            if expected.is_empty() {
                continue;
            }
            if !actual_domains
                .iter()
                .any(|d| d.eq_ignore_ascii_case(expected))
            {
                return Err((
                    format!(
                        "Property 7 violated: expected domain {expected:?} not found in host list {actual_domains:?} for url {url:?}"
                    ),
                    expected.to_string(),
                ));
            }
        }

        Ok(())
    }

    #[test]
    fn property_7_mailto_domains() {
        let mut rng = Rng::new(SEED_MAILTO);
        for case in 0..CASES_MAILTO {
            let (url, expected_domains) = gen_mailto_url(&mut rng);
            if let Err((err, missing_domain)) = check_property_7(&url, &expected_domains) {
                let shrunk = shrink_failing_input(url.clone(), |cand| {
                    if !missing_domain.is_empty()
                        && !cand
                            .to_ascii_lowercase()
                            .contains(&missing_domain.to_ascii_lowercase())
                    {
                        return false;
                    }
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(cand))) {
                        Err(_) => true,
                        Ok(Err(_)) => true,
                        Ok(Ok(p)) => {
                            let host_str = p.host.as_deref().unwrap_or("");
                            let actual_domains: Vec<&str> = host_str.split(", ").collect();
                            !actual_domains
                                .iter()
                                .any(|d| d.eq_ignore_ascii_case(missing_domain.trim()))
                        }
                    }
                });
                panic!(
                    "Property 7 violated!\nSeed: {SEED_MAILTO:#X}\nCase: {case}\nError: {err}\nOriginal: {:?}\nShrunk:   {:?}",
                    url, shrunk
                );
            }
        }
    }
}
