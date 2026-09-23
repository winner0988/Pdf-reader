//! Test helper that runs inside the sandbox and prints what it could or could not do.
//! Used only by crates/sandbox/tests; never shipped. Must not link user32 (win32k is disabled).

#![allow(unsafe_code)]

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = probe::run(&args);
    println!("{result}");
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod probe {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{BufRead, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub fn run(args: &[String]) -> String {
        match args.first().map(String::as_str) {
            Some("echo") => {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line).ok();
                format!("echo:{}", line.trim_end())
            }
            Some("integrity") => integrity_rid(),
            Some("spawn") => match std::process::Command::new(std::env::current_exe().unwrap())
                .arg("noop")
                .status()
            {
                Ok(_) => "spawned".into(),
                Err(_) => "blocked".into(),
            },
            Some("write") => {
                let path = std::path::Path::new(&args[1])
                    .join(format!("probe-{}.txt", std::process::id()));
                match std::fs::write(&path, b"x") {
                    Ok(()) => {
                        std::fs::remove_file(&path).ok();
                        "written".into()
                    }
                    Err(_) => "denied".into(),
                }
            }
            Some("read-handle-stdin") => {
                // The value arrives after start-up: a duplicated handle only exists in the
                // process it was duplicated into.
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line).ok();
                let value: usize = line.trim().parse().unwrap();
                // SAFETY: the test duplicated this handle into us for exclusive use.
                let mut file = unsafe { File::from_raw_handle(value as RawHandle) };
                let mut content = String::new();
                let read = file.read_to_string(&mut content).map(|_| content.len());
                let write = file.write_all(b"tamper");
                format!(
                    "read:{} write:{}",
                    read.map_or("error".to_string(), |len| len.to_string()),
                    if write.is_ok() { "allowed" } else { "denied" }
                )
            }
            Some("load-user32") => {
                let name: Vec<u16> = OsStr::new("user32.dll")
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                // SAFETY: NUL-terminated name.
                let module = unsafe { LoadLibraryW(name.as_ptr()) };
                if module.is_null() {
                    "failed".into()
                } else {
                    "loaded".into()
                }
            }
            Some("alloc") => {
                let megabytes: usize = args[1].parse().unwrap();
                let mut block = Vec::<u8>::new();
                match block.try_reserve_exact(megabytes * 1024 * 1024) {
                    Ok(()) => {
                        block.resize(megabytes * 1024 * 1024, 1);
                        format!("allocated:{}", block.len() / (1024 * 1024))
                    }
                    Err(_) => "alloc-failed".into(),
                }
            }
            Some("env") => {
                let mut names: Vec<String> = std::env::vars_os()
                    .map(|(name, _)| name.to_string_lossy().to_uppercase())
                    .collect();
                names.sort();
                format!("env:{}", names.join(","))
            }
            Some("var") => std::env::var_os(&args[1])
                .map_or("unset".into(), |value| value.to_string_lossy().into_owned()),
            Some("connect") => {
                let address = format!("127.0.0.1:{}", args[1]);
                match std::net::TcpStream::connect_timeout(
                    &address.parse().unwrap(),
                    std::time::Duration::from_secs(3),
                ) {
                    Ok(_) => "connected".into(),
                    Err(_) => "blocked".into(),
                }
            }
            Some("read") => match std::fs::read(&args[1]) {
                Ok(content) => format!("read:{}", content.len()),
                Err(_) => "denied".into(),
            },
            Some("sleep") => {
                std::thread::sleep(std::time::Duration::from_secs(60));
                "woke".into()
            }
            Some("noop") | None => "noop".into(),
            Some(other) => format!("unknown:{other}"),
        }
    }

    fn integrity_rid() -> String {
        let mut token: HANDLE = null_mut();
        // SAFETY: pseudo-handle and valid out-pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return "error".into();
        }
        let mut buffer = vec![0u8; 256];
        let mut length = 0u32;
        // SAFETY: buffer and length describe writable memory.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut length,
            )
        };
        if ok == 0 {
            return "error".into();
        }
        // SAFETY: on success the buffer starts with a TOKEN_MANDATORY_LABEL pointing into it.
        let rid = unsafe {
            let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let count = *GetSidSubAuthorityCount(label.Label.Sid);
            *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1)
        };
        format!("integrity:{rid}")
    }
}
