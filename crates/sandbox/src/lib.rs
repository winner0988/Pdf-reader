//! Starts a child process with the restrictions in docs/architecture/worker-sandbox.md (ADR 0008).
//!
//! All unsafe Win32 code for process isolation lives in this crate. The child:
//! - runs in a Job Object: killed when the job handle closes (including when the main process
//!   dies), at most one process (no children), a memory cap, no clipboard or desktop access;
//! - runs with a restricted token at Low integrity: every privilege except the bypass-traverse
//!   check is removed, and it cannot write to anything the user owns;
//! - has exploit mitigations on: win32k system calls disabled, child processes blocked, no
//!   dynamic code, no images from remote shares or low-integrity locations, strict handle checks;
//! - runs in an AppContainer with no capabilities: no network (not even localhost) and no
//!   access to files that do not explicitly grant it, which includes all of the user's files;
//! - inherits only its three pipe handles and a minimal environment.

#![cfg(windows)]
// Win32 FFI. Every unsafe block states why it is sound.
#![allow(unsafe_code)]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io;
use std::mem::{size_of, size_of_val, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree,
    SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
    NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, FreeSid, GetLengthSid,
    NO_INHERITANCE, PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, DETACHED_PROCESS,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

/// `PROCESS_CREATION_CHILD_PROCESS_RESTRICTED` (winbase.h).
const CHILD_PROCESS_RESTRICTED: u32 = 0x01;

/// Exploit mitigations (winbase.h, `PROCESS_CREATION_MITIGATION_POLICY_*_ALWAYS_ON`).
const MITIGATIONS: u64 = (1 << 8) // force relocate images (mandatory ASLR)
    | (1 << 12) // terminate on heap corruption
    | (1 << 16) // bottom-up ASLR
    | (1 << 20) // high-entropy ASLR
    | (1 << 24) // strict handle checks: using an invalid handle crashes the process
    | (1 << 28) // win32k system calls disabled: no windows, GDI, clipboard or input
    | (1 << 32) // legacy extension points disabled
    | (1 << 36) // no dynamic code (no JIT, no writable+executable memory)
    | (1 << 52) // no image loads from remote shares
    | (1 << 56) // no image loads from low-integrity locations
    | (1 << 60); // prefer System32 images

/// Low mandatory integrity level.
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";

/// AppContainer profile the worker runs in.
pub const WORKER_APP_CONTAINER: &str = "PdfReader.Worker";

/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const HRESULT_ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

/// `ERROR_ACCESS_DENIED`.
const ERROR_ACCESS_DENIED: u32 = 5;

/// Serializes AppContainer profile creation and ACL updates (concurrent profile creation for
/// the same name fails).
static APP_CONTAINER_SETUP: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Memory the process may commit; allocations beyond it fail.
    pub memory_limit_bytes: usize,
    /// AppContainer profile to run in, with no capabilities: no network at all and no access
    /// to the user's files. The profile is created on first use and reused afterwards.
    pub app_container: Option<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
            app_container: Some(WORKER_APP_CONTAINER.to_owned()),
        }
    }
}

/// Removes an AppContainer profile: its registry entries and its folder under
/// `%LOCALAPPDATA%\Packages`. For the uninstaller; succeeds if the profile does not exist.
pub fn delete_app_container_profile(name: &str) -> io::Result<()> {
    let _guard = APP_CONTAINER_SETUP
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let name = wide(OsStr::new(name));
    // SAFETY: NUL-terminated name.
    let result = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    Ok(())
}

/// A running sandboxed process. Dropping it kills the process (the job closes).
pub struct Sandboxed {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}

impl Sandboxed {
    /// Starts `program` with `args` inside the sandbox.
    pub fn spawn(program: &Path, args: &[&OsStr], config: &SandboxConfig) -> io::Result<Self> {
        let (child_stdin, parent_stdin) = pipe()?;
        let (parent_stdout, child_stdout) = pipe()?;
        let (parent_stderr, child_stderr) = pipe()?;
        for handle in [&child_stdin, &child_stdout, &child_stderr] {
            // SAFETY: the handle is owned and open for the duration of the call.
            check(unsafe {
                SetHandleInformation(
                    handle.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT,
                )
            })?;
        }

        let job = create_job(config)?;
        let token = restricted_low_integrity_token()?;

        // These values must stay alive until CreateProcessAsUserW returns (the attribute
        // list stores pointers to them).
        let inherited: [HANDLE; 3] = [
            child_stdin.as_raw_handle(),
            child_stdout.as_raw_handle(),
            child_stderr.as_raw_handle(),
        ];
        let jobs: [HANDLE; 1] = [job.as_raw_handle()];
        let mitigations: u64 = MITIGATIONS;
        let child_policy: u32 = CHILD_PROCESS_RESTRICTED;
        let container = match config.app_container.as_deref() {
            Some(name) => {
                let _guard = APP_CONTAINER_SETUP
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let container = AppContainerSid::obtain(name)?;
                container.grant_read_execute(program)?;
                Some(container)
            }
            None => None,
        };
        let capabilities = container.as_ref().map(|container| SECURITY_CAPABILITIES {
            AppContainerSid: container.0,
            Capabilities: null_mut(),
            CapabilityCount: 0, // none: in particular no internetClient / privateNetwork
            Reserved: 0,
        });

        let mut attributes = AttributeList::new(if capabilities.is_some() { 5 } else { 4 })?;
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &mitigations)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &child_policy)?;
        if let Some(capabilities) = &capabilities {
            attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, capabilities)?;
        }

        // SAFETY: STARTUPINFOEXW is plain data; all-zero is a valid initial value.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = inherited[0];
        startup.StartupInfo.hStdOutput = inherited[1];
        startup.StartupInfo.hStdError = inherited[2];
        startup.lpAttributeList = attributes.as_ptr();

        let application = wide(program.as_os_str());
        let mut command_line = command_line(program.as_os_str(), args);
        let environment = minimal_environment(container.is_some());
        // System32: always readable, also from an AppContainer; the worker uses no relative paths.
        let current_dir = std::env::var_os("SystemRoot")
            .map(|root| wide(Path::new(&root).join("System32").as_os_str()));

        // SAFETY: PROCESS_INFORMATION is plain data; all-zero is a valid initial value.
        let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
        // SAFETY: every pointer refers to a live, NUL-terminated buffer or initialized struct;
        // the attribute list and the arrays it points to outlive the call.
        check(unsafe {
            CreateProcessAsUserW(
                token.as_raw_handle(),
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1, // inherit handles: limited to `inherited` by the handle list
                // DETACHED_PROCESS: no console. A Low-integrity process cannot create one
                // (it fails with STATUS_DLL_INIT_FAILED); the worker only uses its pipes.
                EXTENDED_STARTUPINFO_PRESENT | DETACHED_PROCESS | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                current_dir.as_ref().map_or(null(), |dir| dir.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        })?;
        // SAFETY: CreateProcessAsUserW succeeded, so both handles are valid and ours to close.
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
        drop((child_stdin, child_stdout, child_stderr));

        Ok(Self {
            process,
            job,
            pid: info.dwProcessId,
            stdin: Some(File::from(parent_stdin)),
            stdout: Some(File::from(parent_stdout)),
            stderr: Some(File::from(parent_stderr)),
        })
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    /// The most memory the process has committed so far, as tracked by its job.
    pub fn peak_memory_bytes(&self) -> io::Result<usize> {
        // SAFETY: plain data; all-zero is valid.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        // SAFETY: the job handle is valid; the pointer and size describe `info`.
        check(unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw mut info).cast(),
                size_of_val(&info) as u32,
                null_mut(),
            )
        })?;
        Ok(info.PeakProcessMemoryUsed)
    }

    /// Duplicates `file` into the child with read-only access and returns the handle value as
    /// the child sees it. The child never learns the path.
    pub fn duplicate_read_only(&self, file: &File) -> io::Result<u64> {
        let mut target: HANDLE = null_mut();
        // SAFETY: both process handles and the source handle are valid for the call.
        check(unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                file.as_raw_handle(),
                self.process.as_raw_handle(),
                &mut target,
                FILE_GENERIC_READ,
                0,
                0,
            )
        })?;
        Ok(target as usize as u64)
    }

    /// Terminates the process (and anything else in its job).
    pub fn kill(&self) -> io::Result<()> {
        // SAFETY: the job handle is valid.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    /// Waits up to `timeout` for the process to exit and returns its exit code.
    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<Option<u32>> {
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: the process handle is valid.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), millis) } {
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                // SAFETY: the process handle is valid and `code` is writable.
                check(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
                Ok(Some(code))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle was just returned by a successful Win32 call and is not owned elsewhere.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// An anonymous pipe; neither end is inheritable. Returns (read end, write end).
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: both out-pointers are valid; null attributes mean non-inheritable handles.
    check(unsafe { CreatePipe(&mut read, &mut write, null(), 0) })?;
    Ok((owned(read)?, owned(write)?))
}

fn create_job(config: &SandboxConfig) -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name create an unnamed job with a default security descriptor.
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;

    // SAFETY: plain data; all-zero is valid.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = config.memory_limit_bytes;
    // SAFETY: the pointer and size describe `limits`.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of_val(&limits) as u32,
        )
    })?;

    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    // SAFETY: the pointer and size describe `ui`.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicUIRestrictions,
            (&raw const ui).cast(),
            size_of_val(&ui) as u32,
        )
    })?;
    Ok(job)
}

/// A copy of our token with every privilege removed and the integrity level set to Low.
fn restricted_low_integrity_token() -> io::Result<OwnedHandle> {
    let mut current: HANDLE = null_mut();
    // The restricted token inherits these access rights; TOKEN_ADJUST_DEFAULT is needed to lower
    // its integrity level.
    let access = TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT;
    // SAFETY: GetCurrentProcess returns a pseudo-handle; the out-pointer is valid.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut current) })?;
    let current = owned(current)?;

    let mut restricted: HANDLE = null_mut();
    // SAFETY: the source token is valid; no SID or privilege arrays are passed.
    check(unsafe {
        CreateRestrictedToken(
            current.as_raw_handle(),
            DISABLE_MAX_PRIVILEGE,
            0,
            null(),
            0,
            null(),
            0,
            null(),
            &mut restricted,
        )
    })?;
    let restricted = owned(restricted)?;

    let sid_string = wide(OsStr::new(LOW_INTEGRITY_SID));
    let mut sid = null_mut();
    // SAFETY: NUL-terminated input; on success `sid` must be released with LocalFree.
    check(unsafe { ConvertStringSidToSidW(sid_string.as_ptr(), &mut sid) })?;
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: SE_GROUP_INTEGRITY as u32,
        },
    };
    // SAFETY: `label` points at a valid SID; the length covers the struct and the SID.
    let result = check(unsafe {
        SetTokenInformation(
            restricted.as_raw_handle(),
            TokenIntegrityLevel,
            (&raw const label).cast(),
            (size_of::<TOKEN_MANDATORY_LABEL>() + GetLengthSid(sid) as usize) as u32,
        )
    });
    // SAFETY: `sid` was allocated by ConvertStringSidToSidW and is not used after this.
    unsafe { LocalFree(sid) };
    result?;
    Ok(restricted)
}

/// The SID of an AppContainer profile; the profile is created if it does not exist yet.
struct AppContainerSid(PSID);

impl AppContainerSid {
    fn obtain(name: &str) -> io::Result<Self> {
        let name = wide(OsStr::new(name));
        let mut sid: PSID = null_mut();
        // SAFETY: NUL-terminated strings; no capabilities are requested.
        let created = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if created == HRESULT_ALREADY_EXISTS {
            // SAFETY: NUL-terminated name and a valid out-pointer.
            let derived =
                unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
            if derived < 0 {
                return Err(io::Error::from_raw_os_error(derived));
            }
        } else if created < 0 {
            return Err(io::Error::from_raw_os_error(created));
        }
        Ok(Self(sid))
    }
}

impl AppContainerSid {
    /// Lets the container read and execute `program`. AppContainers cannot open files that do
    /// not grant them access, and user-owned folders (a checkout, a per-user install) do not.
    /// Only the executable itself is opened up, never its folder. If the ACL cannot be changed
    /// (e.g. under Program Files, which already grants app packages access), this is a no-op.
    fn grant_read_execute(&self, program: &Path) -> io::Result<()> {
        let path = wide(program.as_os_str());
        let mut dacl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: NUL-terminated path; on success `descriptor` owns the memory `dacl` points into
        // and is released with LocalFree below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: self.0.cast(),
            },
        };
        let mut updated = null_mut();
        // SAFETY: `dacl` is valid while `descriptor` is alive; GRANT_ACCESS merges with an
        // existing entry for the same SID instead of duplicating it.
        let status = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut updated) };
        // SAFETY: `descriptor` came from GetNamedSecurityInfoW and is not used after this.
        unsafe { LocalFree(descriptor) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        // SAFETY: NUL-terminated path and a valid ACL from SetEntriesInAclW.
        let status = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                updated,
                null(),
            )
        };
        // SAFETY: `updated` came from SetEntriesInAclW and is not used after this.
        unsafe { LocalFree(updated.cast()) };
        match status {
            0 | ERROR_ACCESS_DENIED => Ok(()),
            other => Err(io::Error::from_raw_os_error(other as i32)),
        }
    }
}

impl Drop for AppContainerSid {
    fn drop(&mut self) {
        // SAFETY: the SID came from CreateAppContainerProfile or
        // DeriveAppContainerSidFromAppContainerName, both documented to be released by FreeSid.
        unsafe { FreeSid(self.0) };
    }
}

/// PROC_THREAD_ATTRIBUTE_LIST storage (pointer-aligned).
struct AttributeList {
    buffer: Vec<usize>,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: querying the required size with a null list is the documented usage; it
        // "fails" with ERROR_INSUFFICIENT_BUFFER and fills in `size`.
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: `buffer` is at least `size` bytes and pointer-aligned.
        check(unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), count, 0, &mut size)
        })?;
        Ok(Self { buffer })
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.buffer.as_mut_ptr().cast()
    }

    /// Stores a pointer to `value`, which must outlive the process creation call.
    fn set<T>(&mut self, attribute: u32, value: &T) -> io::Result<()> {
        // SAFETY: the list was initialized; `value` is valid for `size_of_val(value)` bytes.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                (value as *const T).cast(),
                size_of_val(value),
                null_mut(),
                null(),
            )
        })
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in `new`.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// Only what Windows itself needs to start the process and load system DLLs; nothing else
/// from the user's environment.
fn minimal_environment(app_container: bool) -> Vec<u16> {
    let mut block = Vec::new();
    // CreateProcess needs LOCALAPPDATA for an AppContainer (it fails with ERROR_ENVVAR_NOT_FOUND
    // otherwise) and rewrites it, adding TEMP and TMP, to the container's private folder.
    let names: &[&str] = if app_container {
        &["SystemRoot", "LOCALAPPDATA"]
    } else {
        &["SystemRoot"]
    };
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            block.extend(OsStr::new(name).encode_wide());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        }
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0); // an empty block is two NULs
    }
    block
}

/// Builds a command line that CommandLineToArgvW / the Rust runtime parse back into `args`.
fn command_line(program: &OsStr, args: &[&OsStr]) -> Vec<u16> {
    let mut line = Vec::new();
    for (index, arg) in std::iter::once(program)
        .chain(args.iter().copied())
        .enumerate()
    {
        if index > 0 {
            line.push(u16::from(b' '));
        }
        let arg: Vec<u16> = arg.encode_wide().collect();
        let needs_quotes = arg.is_empty()
            || arg
                .iter()
                .any(|&c| c == u16::from(b' ') || c == u16::from(b'\t'));
        if needs_quotes {
            line.push(u16::from(b'"'));
        }
        let mut backslashes = 0;
        for &c in &arg {
            if c == u16::from(b'\\') {
                backslashes += 1;
            } else {
                if c == u16::from(b'"') {
                    line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(c);
        }
        if needs_quotes {
            line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            line.push(u16::from(b'"'));
        }
    }
    line.push(0);
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let wide = command_line(OsStr::new(program), &args);
        String::from_utf16(&wide[..wide.len() - 1]).unwrap()
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(
            line(r"C:\app\w.exe", &["echo", "1"]),
            r"C:\app\w.exe echo 1"
        );
        assert_eq!(
            line(r"C:\Pdf reader\w.exe", &["a b", ""]),
            r#""C:\Pdf reader\w.exe" "a b" """#
        );
    }

    #[test]
    fn escapes_quotes_and_trailing_backslashes() {
        assert_eq!(line("w", &[r#"say "hi""#]), r#"w "say \"hi\"""#);
        assert_eq!(
            line("w", &[r"C:\dir with space\"]),
            r#"w "C:\dir with space\\""#
        );
    }

    #[test]
    fn environment_contains_only_what_windows_needs() {
        let names = |app_container| {
            let block = String::from_utf16(&minimal_environment(app_container)).unwrap();
            assert!(block.ends_with("\0\0"));
            block
                .split('\0')
                .filter(|entry| !entry.is_empty())
                .map(|entry| entry.split('=').next().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert!(names(false).iter().all(|name| name == "SystemRoot"));
        assert!(
            names(true)
                .iter()
                .all(|name| name == "SystemRoot" || name == "LOCALAPPDATA")
        );
    }
}
