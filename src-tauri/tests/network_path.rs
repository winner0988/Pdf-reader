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

fn gen_file_url(rng: &mut Rng) -> String {
    let host_choices = [
        "localhost",
        "LOCALHOST",
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

    // 0 to 4 slashes (/ or \)
    let num_slashes = rng.next_usize(5);
    let mut slashes = String::new();
    for _ in 0..num_slashes {
        if rng.next_usize(2) == 0 {
            slashes.push('/');
        } else {
            slashes.push('\\');
        }
    }

    let slash_char = if rng.next_usize(2) == 0 { '/' } else { '\\' };

    match num_slashes {
        2 => {
            let host = if rng.next_usize(12) < host_choices.len() {
                host_choices[rng.next_usize(host_choices.len())]
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
            let path = *rng.choose(&path_choices);
            if host.is_empty() {
                if rng.gen_bool() {
                    format!("file:{slashes}")
                } else {
                    format!("file:{slashes}{slash_char}{path}")
                }
            } else {
                format!("file:{slashes}{host}{slash_char}{path}")
            }
        }
        3 => {
            let path = *rng.choose(&path_choices);
            format!("file:{slashes}{path}")
        }
        4 => {
            let server = if rng.next_usize(10) < 6 {
                *rng.choose(&[
                    "server",
                    "remote",
                    "myhost",
                    "sharehost",
                    "corp",
                    "fileserver",
                ])
            } else {
                *rng.choose(&["localhost.example", "localhostserver", "127.0.0.1", "[::1]"])
            };
            let share = *rng.choose(&["share", "doc.pdf", "data", "public"]);
            format!("file:{slashes}{server}{slash_char}{share}")
        }
        _ => {
            let path = *rng.choose(&path_choices);
            format!("file:{slashes}{path}")
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
