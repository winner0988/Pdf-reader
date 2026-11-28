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

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{
    DuplicateHandle, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
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
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CONTAINER_INHERIT_ACE, CreateRestrictedToken, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE,
    EqualSid, FreeSid, GetAce, GetAclInformation, GetLengthSid, INHERIT_ONLY_ACE, NO_INHERITANCE,
    OBJECT_INHERIT_ACE, PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_APPEND_DATA, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_WRITE_DATA, SYNCHRONIZE,
};
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
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
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

/// Win32k system calls disabled: no windows, GDI, DirectX, clipboard or input
/// (`PROCESS_CREATION_MITIGATION_POLICY_WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON`).
const WIN32K_DISABLED: u64 = 1 << 28;

/// Exploit mitigations (winbase.h, `PROCESS_CREATION_MITIGATION_POLICY_*_ALWAYS_ON`).
const MITIGATIONS: u64 = (1 << 8) // force relocate images (mandatory ASLR)
    | (1 << 12) // terminate on heap corruption
    | (1 << 16) // bottom-up ASLR
    | (1 << 20) // high-entropy ASLR
    | (1 << 24) // strict handle checks: using an invalid handle crashes the process
    | WIN32K_DISABLED
    | (1 << 32) // legacy extension points disabled
    | (1 << 36) // no dynamic code (no JIT, no writable+executable memory)
    | (1 << 52) // no image loads from remote shares
    | (1 << 56) // no image loads from low-integrity locations
    | (1 << 60); // prefer System32 images

/// Low mandatory integrity level.
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";

/// All Application Packages SID (AppContainer read/execute on system & portable files).
const ALL_APPLICATION_PACKAGES_SID: &str = "S-1-15-2-1";

/// All Restricted Application Packages SID (WebView2 renderer AppContainers on Windows 10).
const ALL_RESTRICTED_APPLICATION_PACKAGES_SID: &str = "S-1-15-2-2";

/// `ACCESS_ALLOWED_ACE_TYPE` (`winnt.h`).
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

/// Windows 11 first build number (`RtlGetVersion` distinguishes Windows 10 `< 22000`).
const WIN11_BUILD_NUMBER: u32 = 22000;

/// AppContainer profile the worker runs in.
pub const WORKER_APP_CONTAINER: &str = "PdfReader.Worker";

/// AppContainer profile the portable POC worker runs in (#449, ADR 0034).
pub const PORTABLE_WORKER_APP_CONTAINER: &str = "PdfReader.Portable.Worker";

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
    /// Whether `spawn` grants Read & Execute access on `program` to the AppContainer SID when
    /// needed (ignoring `ERROR_ACCESS_DENIED` on read-only installs). Portable mode verifies
    /// and sets ACLs during startup preflight instead so ACL failures abort with a message.
    pub grant_program_acl: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
            app_container: Some(WORKER_APP_CONTAINER.to_owned()),
            grant_program_acl: true,
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
                if config.grant_program_acl {
                    container.grant_read_execute(program)?;
                }
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
        self.duplicate(file, FILE_GENERIC_READ)
    }

    /// Duplicates `file` into the child with write-only access: it can write the file's content,
    /// but not read it, delete or rename it, or change its attributes. For a new file the parent
    /// created, such as a document being saved (ADR 0013, proposed); the child never learns the
    /// path.
    pub fn duplicate_write_only(&self, file: &File) -> io::Result<u64> {
        self.duplicate(file, FILE_WRITE_DATA | FILE_APPEND_DATA | SYNCHRONIZE)
    }

    fn duplicate(&self, file: &File, access: u32) -> io::Result<u64> {
        let mut target: HANDLE = null_mut();
        // SAFETY: both process handles and the source handle are valid for the call; the
        // duplicate gets exactly `access`, never more than the source has.
        check(unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                file.as_raw_handle(),
                self.process.as_raw_handle(),
                &mut target,
                access,
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

/// Security descriptor returned by `GetNamedSecurityInfoW`, freed via `LocalFree` on drop.
struct SecurityDescriptorGuard {
    descriptor: *mut c_void,
    dacl: *mut ACL,
}

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            // SAFETY: `descriptor` came from GetNamedSecurityInfoW and is no longer referenced.
            unsafe { LocalFree(self.descriptor) };
        }
    }
}

/// SID returned by `ConvertStringSidToSidW`, freed via `LocalFree` on drop.
struct ConvertedSid(PSID);

impl ConvertedSid {
    fn parse(sid: &str) -> io::Result<Self> {
        let wide_sid = wide(OsStr::new(sid));
        let mut raw = null_mut();
        // SAFETY: NUL-terminated string SID; `raw` is released with LocalFree on drop.
        check(unsafe { ConvertStringSidToSidW(wide_sid.as_ptr(), &mut raw) })?;
        Ok(Self(raw))
    }
}

impl Drop for ConvertedSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was allocated by ConvertStringSidToSidW.
            unsafe { LocalFree(self.0) };
        }
    }
}

fn read_dacl(path: &Path) -> io::Result<SecurityDescriptorGuard> {
    let wide_path = wide(path.as_os_str());
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: NUL-terminated path; `descriptor` owns `dacl` and is released on drop.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
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
    Ok(SecurityDescriptorGuard { descriptor, dacl })
}

fn ace_grants_rx(mask: u32) -> bool {
    let rx = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
    (mask & rx) == rx
        || (mask & GENERIC_ALL) != 0
        || ((mask & GENERIC_READ) != 0 && (mask & GENERIC_EXECUTE) != 0)
}

fn dacl_grants_rx(dacl: *const ACL, sid: PSID, require_inheritance: bool) -> io::Result<bool> {
    if dacl.is_null() {
        return Ok(true);
    }
    // SAFETY: `ACL_SIZE_INFORMATION` is plain old data.
    let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    // SAFETY: `dacl` came from a live `SecurityDescriptorGuard` and `info` is valid for AclSizeInformation.
    check(unsafe {
        GetAclInformation(
            dacl,
            (&raw mut info).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    })?;
    let required_inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    for index in 0..info.AceCount {
        let mut ace_ptr = null_mut();
        // SAFETY: `index < info.AceCount` on a valid ACL.
        check(unsafe { GetAce(dacl, index, &mut ace_ptr) })?;
        // SAFETY: `GetAce` succeeded, pointing to an `ACE_HEADER` within `dacl`.
        let header = unsafe { *ace_ptr.cast::<ACE_HEADER>() };
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        let flags = u32::from(header.AceFlags);
        if (flags & INHERIT_ONLY_ACE) != 0 {
            continue;
        }
        if require_inheritance && (flags & required_inherit) != required_inherit {
            continue;
        }
        // SAFETY: `AceType == ACCESS_ALLOWED_ACE_TYPE`, so the entry layout is `ACCESS_ALLOWED_ACE`.
        let ace = unsafe { &*ace_ptr.cast::<ACCESS_ALLOWED_ACE>() };
        // `SidStart` is the first DWORD of the variable-length SID stored inside the ACE.
        let ace_sid: PSID = (&raw const ace.SidStart).cast_mut().cast();
        // SAFETY: both `ace_sid` and `sid` point to valid Windows SIDs.
        if unsafe { EqualSid(ace_sid, sid) } != 0 && ace_grants_rx(ace.Mask) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn apply_rx_access(
    path: &Path,
    sids: &[PSID],
    inheritance: u32,
    ignore_access_denied: bool,
) -> io::Result<()> {
    let guard = read_dacl(path)?;
    let require_inheritance = inheritance != NO_INHERITANCE;
    let mut entries = Vec::new();
    for &sid in sids {
        if !dacl_grants_rx(guard.dacl, sid, require_inheritance)? {
            entries.push(EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: inheritance,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_UNKNOWN,
                    ptstrName: sid.cast(),
                },
            });
        }
    }
    if entries.is_empty() {
        return Ok(());
    }
    let mut updated = null_mut();
    // SAFETY: `guard.dacl` is alive until `guard` drops and `entries` holds valid SIDs.
    let status = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            guard.dacl,
            &mut updated,
        )
    };
    drop(guard);
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let wide_path = wide(path.as_os_str());
    // SAFETY: NUL-terminated path and a valid ACL from `SetEntriesInAclW`.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            updated,
            null(),
        )
    };
    // SAFETY: `updated` came from `SetEntriesInAclW` and is not used after this.
    unsafe { LocalFree(updated.cast()) };
    match status {
        0 => Ok(()),
        ERROR_ACCESS_DENIED if ignore_access_denied => Ok(()),
        other => Err(io::Error::from_raw_os_error(other as i32)),
    }
}

/// Returns true on Windows 10 (`RtlGetVersion` major == 10 and build < 22000).
pub fn is_windows_10() -> bool {
    // SAFETY: `OSVERSIONINFOW` is plain data and `dwOSVersionInfoSize` is set below.
    let mut info: OSVERSIONINFOW = unsafe { zeroed() };
    info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: `info` has `dwOSVersionInfoSize` initialized as required by `RtlGetVersion`.
    let status = unsafe { RtlGetVersion(&raw mut info) };
    status == 0 && info.dwMajorVersion == 10 && info.dwBuildNumber < WIN11_BUILD_NUMBER
}

/// Ensures the portable worker executable is readable and executable by `PORTABLE_WORKER_APP_CONTAINER`
/// (creating the profile if absent) or `S-1-15-2-1`, failing if ACL modification is denied (ADR 0034).
pub fn ensure_portable_worker_acl(worker: &Path) -> io::Result<()> {
    let _guard = APP_CONTAINER_SETUP
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let container = AppContainerSid::obtain(PORTABLE_WORKER_APP_CONTAINER)?;
    let all_app_packages = ConvertedSid::parse(ALL_APPLICATION_PACKAGES_SID)?;
    let dacl_guard = read_dacl(worker)?;
    let already_granted = dacl_grants_rx(dacl_guard.dacl, container.0, false)?
        || dacl_grants_rx(dacl_guard.dacl, all_app_packages.0, false)?;
    drop(dacl_guard);
    if already_granted {
        return Ok(());
    }
    apply_rx_access(worker, &[container.0], NO_INHERITANCE, false)
}

/// On Windows 10, ensures `dir` (and inherited files/subfolders inside `WebView2\`) grant
/// `(OI)(CI)(RX)` to both `S-1-15-2-1` and `S-1-15-2-2` so WebView2 renderers can load DLLs (ADR 0034).
/// No-op on Windows 11+.
pub fn ensure_webview2_windows10_acls(dir: &Path) -> io::Result<()> {
    if !is_windows_10() {
        return Ok(());
    }
    grant_webview2_appcontainer_acls(dir)
}

fn grant_webview2_appcontainer_acls(dir: &Path) -> io::Result<()> {
    let all_packages = ConvertedSid::parse(ALL_APPLICATION_PACKAGES_SID)?;
    let restricted = ConvertedSid::parse(ALL_RESTRICTED_APPLICATION_PACKAGES_SID)?;
    apply_rx_access(
        dir,
        &[all_packages.0, restricted.0],
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        false,
    )
}

impl AppContainerSid {
    /// Lets the container read and execute `program`. AppContainers cannot open files that do
    /// not grant them access, and user-owned folders (a checkout, a per-user install) do not.
    /// Only the executable itself is opened up, never its folder. If the ACL cannot be changed
    /// (e.g. under Program Files, which already grants app packages access), this is a no-op.
    fn grant_read_execute(&self, program: &Path) -> io::Result<()> {
        apply_rx_access(program, &[self.0], NO_INHERITANCE, true)
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

    #[test]
    fn grants_inheritable_webview2_acls_on_directory() {
        let base = std::env::temp_dir().join(format!("sandbox-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        let child = base.join("sub").join("dummy.dll");
        std::fs::write(&child, b"test").unwrap();

        grant_webview2_appcontainer_acls(&base).unwrap();

        let pkg = ConvertedSid::parse(ALL_APPLICATION_PACKAGES_SID).unwrap();
        let restricted = ConvertedSid::parse(ALL_RESTRICTED_APPLICATION_PACKAGES_SID).unwrap();
        let dir_dacl = read_dacl(&base).unwrap();
        assert!(dacl_grants_rx(dir_dacl.dacl, pkg.0, true).unwrap());
        assert!(dacl_grants_rx(dir_dacl.dacl, restricted.0, true).unwrap());
        let child_dacl = read_dacl(&child).unwrap();
        assert!(dacl_grants_rx(child_dacl.dacl, pkg.0, false).unwrap());
        assert!(dacl_grants_rx(child_dacl.dacl, restricted.0, false).unwrap());

        let _ = std::fs::remove_dir_all(&base);
    }
}
