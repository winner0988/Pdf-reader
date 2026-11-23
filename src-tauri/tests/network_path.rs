//! Property tests for `is_network_path` against Windows `PathCreateFromUrlW` (#616).
//!
//! Compares the contract's UNC network path classifier with Windows shell API behavior
//! on random `file:` URLs.
#![cfg(windows)]
#![allow(unsafe_code)]

use ipc_contract::text::is_network_path;
use std::os::windows::ffi::OsStrExt;

const SEED_NETWORK_PATH: u64 = 0x616_0003;
const CASES_NETWORK_PATH: usize = 10_000;

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

/// Calls Windows `PathCreateFromUrlW` with a 2084 `u16` output buffer.
fn path_create_from_url(url: &str) -> Option<String> {
    let mut wide_url: Vec<u16> = std::ffi::OsStr::new(url).encode_wide().collect();
    wide_url.push(0);

    let mut buf = [0u16; 2084];
    let mut buf_len = buf.len() as u32;

    // SAFETY:
    // `wide_url` is a valid null-terminated UTF-16 string buffer.
    // `buf` is a valid mutable slice of 2084 u16 elements, and `buf_len` is initialized
    // to the buffer length. `dwflags` is 0.
    let hr = unsafe {
        windows_sys::Win32::UI::Shell::PathCreateFromUrlW(
            wide_url.as_ptr(),
            buf.as_mut_ptr(),
            &mut buf_len,
            0,
        )
    };

    if hr == 0 && buf_len > 0 {
        let len = (0..buf_len as usize)
            .position(|i| buf[i] == 0)
            .unwrap_or(buf_len as usize);
        if len > 0 {
            Some(String::from_utf16_lossy(&buf[..len]))
        } else {
            None
        }
    } else {
        None
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
                let mut candidate = String::with_capacity(current.len() - (end_byte - start_byte));
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

fn gen_random_host(rng: &mut Rng) -> String {
    let len = 1 + rng.next_usize(8);
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        let b = if rng.gen_bool() {
            b'a' + (rng.next_usize(26) as u8)
        } else {
            b'A' + (rng.next_usize(26) as u8)
        };
        s.push(b as char);
    }
    s
}

fn gen_file_url(rng: &mut Rng) -> String {
    let slash_pieces = ["/", "\\", "%2F", "%2f", "%5C", "%5c"];
    let host_choices = [
        "localhost",
        "LOCALHOST",
        "LocalHost",
        "localhost.",
        "localhost.example",
        "localhostserver",
        "localhost:445",
        "%6Cocalhost",
        "",
        "127.0.0.1",
        "[::1]",
    ];
    let path_choices = [
        "C:/doc.pdf",
        "share/doc.pdf",
        "C:/Windows/calc.exe",
        "share/sub/doc.pdf",
        "doc.pdf",
        "test/file.txt",
    ];

    let slashes_2 = if rng.gen_bool() {
        "//"
    } else {
        match rng.next_usize(3) {
            0 => r"\\",
            1 => r"/\",
            _ => r"\/",
        }
    };

    let kind = rng.next_usize(5);
    match kind {
        0 => {
            // host: localhost
            let host = *rng.choose(&["localhost", "LOCALHOST", "LocalHost"]);
            let num_post = rng.next_usize(5);
            let mut post = String::new();
            for _ in 0..num_post {
                post.push_str(rng.choose(&slash_pieces));
            }
            let next_server = if rng.gen_bool() {
                format!("{}/", gen_random_host(rng))
            } else if rng.gen_bool() {
                "server/".to_string()
            } else {
                String::new()
            };
            let path = *rng.choose(&path_choices);
            format!("file:{slashes_2}{host}{post}{next_server}{path}")
        }
        1 => {
            // host: empty ("")
            let num_post = rng.next_usize(5);
            let mut post = String::new();
            for _ in 0..num_post {
                post.push_str(rng.choose(&slash_pieces));
            }
            let next_server = if rng.gen_bool() {
                format!("{}/", gen_random_host(rng))
            } else {
                "server/".to_string()
            };
            let path = *rng.choose(&path_choices);
            format!("file:{slashes_2}{post}{next_server}{path}")
        }
        2 => {
            // host: other fixed host
            let host = if rng.next_usize(12) < host_choices.len() {
                let h = host_choices[rng.next_usize(host_choices.len())];
                if h.is_empty() || h.eq_ignore_ascii_case("localhost") {
                    "server"
                } else {
                    h
                }
            } else {
                *rng.choose(&[
                    "server",
                    "remote",
                    "myhost",
                    "sharehost",
                    "corp",
                    "fileserver",
                ])
            };
            let num_post = rng.next_usize(5);
            let mut post = String::new();
            for _ in 0..num_post {
                post.push_str(rng.choose(&slash_pieces));
            }
            if post.is_empty() {
                post.push(if rng.gen_bool() { '/' } else { '\\' });
            }
            let path = *rng.choose(&path_choices);
            format!("file:{slashes_2}{host}{post}{path}")
        }
        3 => {
            // host: random letters (1 to 8 letters)
            let host = gen_random_host(rng);
            let num_post = rng.next_usize(5);
            let mut post = String::new();
            for _ in 0..num_post {
                post.push_str(rng.choose(&slash_pieces));
            }
            if post.is_empty() {
                post.push(if rng.gen_bool() { '/' } else { '\\' });
            }
            let path = *rng.choose(&path_choices);
            format!("file:{slashes_2}{host}{post}{path}")
        }
        _ => {
            // file: followed directly by %2F, %5C or 0 to 1 slash then %2F, %5C
            let mut prefix = String::new();
            if rng.gen_bool() {
                prefix.push(if rng.gen_bool() { '/' } else { '\\' });
            }
            let num_pieces = 1 + rng.next_usize(4);
            for _ in 0..num_pieces {
                prefix.push_str(rng.choose(&slash_pieces));
            }
            let next_server = if rng.gen_bool() {
                format!("{}/", gen_random_host(rng))
            } else {
                "server/".to_string()
            };
            let path = *rng.choose(&path_choices);
            format!("file:{prefix}{next_server}{path}")
        }
    }
}

#[test]
fn property_8_is_network_path_matches_windows_path_create_from_url() {
    let mut rng = Rng::new(SEED_NETWORK_PATH);
    let mut conversion_failures = 0;
    let mut checked_cases = 0;

    for case in 0..CASES_NETWORK_PATH {
        let url = gen_file_url(&mut rng);
        match path_create_from_url(&url) {
            Some(path) => {
                checked_cases += 1;
                let is_unc = path.starts_with(r"\\");
                let is_net = is_network_path(url.as_bytes());
                if is_unc != is_net {
                    let shrunk = shrink_failing_input(url.clone(), |cand| {
                        path_create_from_url(cand)
                            .map(|p| p.starts_with(r"\\") != is_network_path(cand.as_bytes()))
                            .unwrap_or(false)
                    });
                    panic!(
                        "Property 8 violated!\nSeed: {SEED_NETWORK_PATH:#X}\nCase: {case}\nURL: {url:?}\nWindows path: {path:?}\nstarts_with(\"\\\\\\\\\"): {is_unc}\nis_network_path: {is_net}\nShrunk: {shrunk:?}"
                    );
                }
            }
            None => {
                conversion_failures += 1;
            }
        }
    }

    println!(
        "Property 8 verified on {checked_cases} URLs ({conversion_failures} conversion failures omitted)"
    );
}

#[test]
fn windows_agrees_on_the_668_and_661_shapes() {
    let test_urls = [
        // Issue #668 5 shapes + 2 controls
        r"file://localhost/\server\share\doc.pdf",
        r"file://localhost///server/share/doc.pdf",
        r"file://localhost/%5Cserver/share",
        r"file:%5C%5Cserver\share",
        r"file:/%2Fserver/share",
        r"file://localhost//server/share",
        r"file:///server/share",
        // PR #661 table
        r"file://localhost/C:/doc.pdf",
        r"file://LOCALHOST/C:/doc.pdf",
        r"file://localhost",
        r"file://",
        r"file://localhost/share/doc.pdf",
        r"file:\/localhost/C:/doc.pdf",
        r"file:/\localhost/C:/doc.pdf",
        r"file://localhost\C:\doc.pdf",
        r"file:///C:/doc.pdf",
        r"file://localhost.example/share/doc.pdf",
        r"file://localhostserver/share/doc.pdf",
        r"file://localhost:445/share/doc.pdf",
        r"file://LocalHost.Example/s/x",
        r"file://localhost./share/x",
        r"file://%6Cocalhost/share/x",
        r"file:/\localhost.example/share/doc.pdf",
        r"file:\\localhost.example\share\doc.pdf",
        r"file://127.0.0.1/share/doc.pdf",
        r"file://server.example/share/doc.pdf",
        r"file:////server/share",
        // Case variations of %2f and %5c
        r"file://localhost/%5cserver/share",
        r"file://localhost/%2Fserver/share",
        r"file://localhost/%2fserver/share",
        r"file:%5c%5cserver\share",
        r"file:/%2fserver/share",
        r"file:%2F%2Fserver/share",
        r"file:%2f%2fserver/share",
        r"file:%2F%5Cserver/share",
        r"file:%5C%2Fserver/share",
        // localhost followed by 1 to 4 slashes (/ and \ mixed)
        r"file://localhost/server/share",
        r"file://localhost\server\share",
        r"file://localhost//server/share",
        r"file://localhost/\server\share",
        r"file://localhost\/server/share",
        r"file://localhost\\server/share",
        r"file://localhost\\server\share",
        r"file://localhost///server/share",
        r"file://localhost//\server\share",
        r"file://localhost/\//server\share",
        r"file://localhost/\\server\share",
        r"file://localhost\//server\share",
        r"file://localhost\\/server\share",
        r"file://localhost\\\server\share",
        r"file://localhost////server/share",
        r"file://localhost///\server\share",
        r"file://localhost//\\server\share",
        r"file://localhost/\\/server\share",
        r"file://localhost/\\\server\share",
        r"file://localhost\///server\share",
        r"file://localhost\\\\server\share",
        // localhost with encoded slashes
        r"file://localhost/%2F/server/share",
        r"file://localhost//%2Fserver/share",
        r"file://localhost/%5C/server/share",
        r"file://localhost//%5Cserver/share",
        r"file://localhost/\%5Cserver/share",
        r"file://localhost\%5Cserver/share",
        r"file://localhost/%2F%2Fserver/share",
        r"file://localhost/%5C%5Cserver/share",
        // No host / empty host combinations
        r"file:/server/share",
        r"file://server/share",
        r"file:///server/share",
        r"file:////server/share",
        r"file://///server/share",
        r"file:%5C%5Cserver\share",
        r"file:%2F%2Fserver/share",
        r"file:/%5Cserver/share",
        r"file:\%5Cserver\share",
        r"file:\%2Fserver/share",
        r"file:/%2Fserver/share",
        r"file://%5Cserver/share",
        r"file://%2Fserver/share",
        r"file:///%5Cserver/share",
        r"file:///%2Fserver/share",
        r"file:////%5Cserver/share",
        r"file:////%2Fserver/share",
        r"file:%2Fserver/share",
        r"file:%5Cserver/share",
        // localhost:445
        r"file://localhost:445/share",
        r"file://localhost:445//share",
        r"file://localhost:445/%5Cshare",
        r"file://localhost:445/\share",
        r"file://localhost:445///share",
        // Host casing variations
        r"file://LOCALHOST/\server\share\doc.pdf",
        r"file://LOCALHOST/%5Cserver/share",
        r"file://LocalHost///server/share/doc.pdf",
        r"file://LocalHost//server/share",
    ];

    let mut mismatches = Vec::new();
    println!("\n=== Table: windows_agrees_on_the_668_and_661_shapes ===");
    for url in test_urls {
        let win_path = path_create_from_url(url);
        let win_unc = win_path
            .as_ref()
            .map(|p| p.starts_with(r"\\"))
            .unwrap_or(false);
        let contract_net = is_network_path(url.as_bytes());
        let matches = win_unc == contract_net;
        println!(
            "URL: {:<42} | WinPath: {:<32?} | WinUNC: {:<5} | is_net: {:<5} | Match: {}",
            url, win_path, win_unc, contract_net, matches
        );
        if !matches {
            mismatches.push((url, win_path, win_unc, contract_net));
        }
    }
    println!(
        "=== End Table (total mismatches: {}) ===\n",
        mismatches.len()
    );

    assert!(
        mismatches.is_empty(),
        "Mismatches found between Windows PathCreateFromUrlW and is_network_path:\n{:#?}",
        mismatches
    );
}
