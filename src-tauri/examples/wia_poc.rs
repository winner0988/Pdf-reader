//! #688 的 POC，不合併。
//!
//! Standalone WIA 2.0 proof-of-concept for Issue #688 (ADR 0031).
//! Tests device enumeration, property configuration, in-memory `IStream` transfers,
//! loaded module inspection, timeout/cancellation, and error handling without any UI,
//! Tauri commands, or temporary image files on disk.

// WIA 2.0 COM calls, PROPVARIANT inspection, and Psapi module enumeration;
// each unsafe block documents why it is sound.
#![allow(unsafe_code)]

use std::collections::BTreeSet;
use std::ffi::{OsString, c_void};
use std::fs;
use std::io::{self, Write as _};
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, size_of};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{copy_nonoverlapping, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Devices::ImageAcquisition::{
    ALL_PAGES, DUPLEX, FEEDER, FLATBED, IWiaDevMgr2, IWiaItem2, IWiaPropertyStorage, IWiaTransfer,
    IWiaTransferCallback, IWiaTransferCallback_Vtbl, WIA_CATEGORY_AUTO, WIA_CATEGORY_FEEDER,
    WIA_CATEGORY_FEEDER_BACK, WIA_CATEGORY_FEEDER_FRONT, WIA_CATEGORY_FLATBED, WIA_CATEGORY_ROOT,
    WIA_DATA_COLOR, WIA_DATA_GRAYSCALE, WIA_DATA_THRESHOLD, WIA_DEVINFO_ENUM_ALL,
    WIA_DEVINFO_ENUM_LOCAL, WIA_DIP_DEV_DESC, WIA_DIP_DEV_ID, WIA_DIP_DEV_NAME, WIA_DIP_DEV_TYPE,
    WIA_DIP_DRIVER_VERSION, WIA_DIP_PNP_ID, WIA_DIP_PORT_NAME, WIA_DIP_REMOTE_DEV_ID,
    WIA_DIP_SERVER_NAME, WIA_DIP_UI_CLSID, WIA_DIP_VEND_DESC, WIA_DIP_WIA_VERSION,
    WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES, WIA_DPS_DOCUMENT_HANDLING_SELECT,
    WIA_DPS_DOCUMENT_HANDLING_STATUS, WIA_DPS_HORIZONTAL_BED_SIZE,
    WIA_DPS_HORIZONTAL_SHEET_FEED_SIZE, WIA_DPS_OPTICAL_XRES, WIA_DPS_OPTICAL_YRES, WIA_DPS_PAGES,
    WIA_DPS_VERTICAL_BED_SIZE, WIA_DPS_VERTICAL_SHEET_FEED_SIZE, WIA_ERROR_BUSY,
    WIA_ERROR_COVER_OPEN, WIA_ERROR_DESTINATION, WIA_ERROR_DEVICE_COMMUNICATION,
    WIA_ERROR_DEVICE_LOCKED, WIA_ERROR_EXCEPTION_IN_DRIVER, WIA_ERROR_GENERAL_ERROR,
    WIA_ERROR_INCORRECT_HARDWARE_SETTING, WIA_ERROR_INVALID_COMMAND,
    WIA_ERROR_INVALID_DRIVER_RESPONSE, WIA_ERROR_ITEM_DELETED, WIA_ERROR_LAMP_OFF,
    WIA_ERROR_MULTI_FEED, WIA_ERROR_NETWORK_RESERVATION_FAILED, WIA_ERROR_OFFLINE,
    WIA_ERROR_PAPER_EMPTY, WIA_ERROR_PAPER_JAM, WIA_ERROR_PAPER_PROBLEM,
    WIA_ERROR_USER_INTERVENTION, WIA_ERROR_WARMING_UP, WIA_INTENT_IMAGE_TYPE_COLOR,
    WIA_INTENT_IMAGE_TYPE_GRAYSCALE, WIA_INTENT_IMAGE_TYPE_TEXT, WIA_IPA_BYTES_PER_LINE,
    WIA_IPA_CHANNELS_PER_PIXEL, WIA_IPA_DATATYPE, WIA_IPA_DEPTH, WIA_IPA_FORMAT,
    WIA_IPA_FULL_ITEM_NAME, WIA_IPA_ITEM_NAME, WIA_IPA_NUMBER_OF_LINES, WIA_IPA_PIXELS_PER_LINE,
    WIA_IPA_PREFERRED_FORMAT, WIA_IPA_TYMED, WIA_IPS_CUR_INTENT, WIA_IPS_MAX_HORIZONTAL_SIZE,
    WIA_IPS_MAX_VERTICAL_SIZE, WIA_IPS_PAGE_SIZE, WIA_IPS_XEXTENT, WIA_IPS_XPOS, WIA_IPS_XRES,
    WIA_IPS_YEXTENT, WIA_IPS_YPOS, WIA_IPS_YRES, WIA_PAGE_A4, WIA_PROP_FLAG, WIA_PROP_LIST,
    WIA_PROP_RANGE, WIA_S_NO_DEVICE_AVAILABLE, WIA_TRANSFER_MSG_DEVICE_STATUS,
    WIA_TRANSFER_MSG_END_OF_STREAM, WIA_TRANSFER_MSG_END_OF_TRANSFER, WIA_TRANSFER_MSG_NEW_PAGE,
    WIA_TRANSFER_MSG_STATUS, WiaDevMgr2, WiaImgFmt_BMP, WiaImgFmt_JPEG, WiaImgFmt_PNG,
    WiaImgFmt_TIFF, WiaItemTypeDevice, WiaItemTypeFolder, WiaItemTypeImage,
    WiaItemTypeProgrammableDataSource, WiaItemTypeRoot, WiaItemTypeTransfer, WiaTransferParams,
};
use windows::Win32::Foundation::{
    E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_POINTER, ERROR_SERVICE_NOT_ACTIVE, HGLOBAL,
    HMODULE, MAX_PATH, S_FALSE, S_OK, STG_E_INVALIDFUNCTION, STG_E_MEDIUMFULL,
};
use windows::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, PROPSPEC, PROPSPEC_0, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0,
    PROPVARIANT_0_0_0, PRSPEC_PROPID, PropVariantClear, STATPROPSTG,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc,
    CoTaskMemFree, CoUninitialize, IGlobalInterfaceTable, ISequentialStream,
    ISequentialStream_Vtbl, IStream, IStream_Vtbl, STATFLAG_NONAME, STATSTG, STGTY_STREAM,
    STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET, TYMED_FILE,
};
use windows::Win32::System::ProcessStatus::{
    EnumProcessModules, GetModuleFileNameExW, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Variant::{
    VT_BSTR, VT_CLSID, VT_EMPTY, VT_I4, VT_LPWSTR, VT_UI4, VT_VECTOR,
};
use windows::core::{
    BSTR, Error as WinError, GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR,
};

const DEFAULT_MAX_PAGE_BYTES: usize = 80 * 1024 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const CLSID_STD_GLOBAL_INTERFACE_TABLE: GUID =
    GUID::from_u128(0x00000323_0000_0000_c000_000000000046);
const STG_E_INVALIDPARAMETER: HRESULT = HRESULT(0x8003_0057_u32 as i32);

// Standard WIA constants not split into separate consts in windows-rs 0.62:
const WIA_DIP_SCANNER: u32 = 1;
const WIA_IPA_BITS_PER_PIXEL_COMPAT: u32 = WIA_IPA_DEPTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Flatbed,
    Feeder,
    Duplex,
    Auto,
}

impl Source {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "flatbed" => Ok(Self::Flatbed),
            "feeder" => Ok(Self::Feeder),
            "duplex" => Ok(Self::Duplex),
            "auto" => Ok(Self::Auto),
            _ => Err(format!(
                "invalid --source '{value}', expected flatbed|feeder|duplex|auto"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorMode {
    Color,
    Grayscale,
    Bw,
}

impl ColorMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "color" => Ok(Self::Color),
            "grayscale" | "gray" => Ok(Self::Grayscale),
            "bw" | "threshold" | "mono" => Ok(Self::Bw),
            _ => Err(format!(
                "invalid --color '{value}', expected color|grayscale|bw"
            )),
        }
    }

    fn wia_data_type(self) -> i32 {
        match self {
            Self::Color => WIA_DATA_COLOR as i32,
            Self::Grayscale => WIA_DATA_GRAYSCALE as i32,
            Self::Bw => WIA_DATA_THRESHOLD as i32,
        }
    }

    fn wia_depth(self) -> i32 {
        match self {
            Self::Color => 24,
            Self::Grayscale => 8,
            Self::Bw => 1,
        }
    }

    fn wia_intent(self) -> i32 {
        match self {
            Self::Color => WIA_INTENT_IMAGE_TYPE_COLOR as i32,
            Self::Grayscale => WIA_INTENT_IMAGE_TYPE_GRAYSCALE as i32,
            Self::Bw => WIA_INTENT_IMAGE_TYPE_TEXT as i32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageFormat {
    Auto,
    Bmp,
    Jpeg,
    Tiff,
    Png,
}

impl ImageFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "bmp" => Ok(Self::Bmp),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "tiff" | "tif" => Ok(Self::Tiff),
            "png" => Ok(Self::Png),
            _ => Err(format!(
                "invalid --format '{value}', expected auto|bmp|jpeg|tiff|png"
            )),
        }
    }

    fn guid(self) -> Option<GUID> {
        match self {
            Self::Auto => None,
            Self::Bmp => Some(WiaImgFmt_BMP),
            Self::Jpeg => Some(WiaImgFmt_JPEG),
            Self::Tiff => Some(WiaImgFmt_TIFF),
            Self::Png => Some(WiaImgFmt_PNG),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageExtent {
    A4,
    DriverDefault,
}

impl PageExtent {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "a4" => Ok(Self::A4),
            "default" => Ok(Self::DriverDefault),
            _ => Err(format!("invalid --page '{value}', expected a4|default")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamKind {
    Bounded,
    HGlobal,
}

impl StreamKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "bounded" => Ok(Self::Bounded),
            "hglobal" => Ok(Self::HGlobal),
            _ => Err(format!(
                "invalid --stream '{value}', expected bounded|hglobal"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitPoint {
    BeforeCreate,
    BeforeDownload,
}

impl WaitPoint {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "before-create" => Ok(Self::BeforeCreate),
            "before-download" => Ok(Self::BeforeDownload),
            _ => Err(format!(
                "invalid --wait-enter '{value}', expected before-create|before-download"
            )),
        }
    }
}

enum Command {
    List {
        details: bool,
        local_only: bool,
        dump_props: bool,
    },
    Scan(ScanArgs),
}

struct ScanArgs {
    device: String,
    direct_id: bool,
    source: Source,
    color: ColorMode,
    dpi: u32,
    page: PageExtent,
    format: ImageFormat,
    stream: StreamKind,
    max_page_bytes: usize,
    timeout_secs: u64,
    cancel_after_ms: Option<u64>,
    cancel_on_progress: Option<i32>,
    check_modules: bool,
    wait_enter: Option<WaitPoint>,
    save_dir: Option<PathBuf>,
}

/// COM apartment guard (`COINIT_MULTITHREADED`) for this thread. Not `Send`.
struct ComGuard(PhantomData<*const ()>);

impl ComGuard {
    fn init_mta() -> windows::core::Result<Self> {
        // SAFETY: `pvReserved` is `None` as required; `CoUninitialize` balances `CoInitializeEx`
        // when this guard drops on the same thread (even on `S_FALSE` for nested init).
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Self(PhantomData))
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: `init_mta` succeeded on this thread.
        unsafe { CoUninitialize() };
    }
}

/// Owns a `PROPVARIANT` and clears any COM-allocated payload on drop.
struct PropVarGuard(PROPVARIANT);

impl PropVarGuard {
    fn new() -> Self {
        Self(PROPVARIANT::default())
    }

    fn as_mut_ptr(&mut self) -> *mut PROPVARIANT {
        &mut self.0
    }

    fn vt(&self) -> u16 {
        // SAFETY: Reading the discriminant field `vt` of an initialized `PROPVARIANT` is valid.
        unsafe { self.0.Anonymous.Anonymous.vt.0 }
    }

    fn as_i32(&self) -> Option<i32> {
        let vt = self.vt();
        // SAFETY: `vt` selects the active union field in `PROPVARIANT`.
        unsafe {
            if vt == VT_I4.0 {
                Some(self.0.Anonymous.Anonymous.Anonymous.lVal)
            } else if vt == VT_UI4.0 {
                i32::try_from(self.0.Anonymous.Anonymous.Anonymous.ulVal).ok()
            } else {
                None
            }
        }
    }

    fn as_u32(&self) -> Option<u32> {
        let vt = self.vt();
        // SAFETY: `vt` selects the active union field in `PROPVARIANT`.
        unsafe {
            if vt == VT_UI4.0 {
                Some(self.0.Anonymous.Anonymous.Anonymous.ulVal)
            } else if vt == VT_I4.0 {
                u32::try_from(self.0.Anonymous.Anonymous.Anonymous.lVal).ok()
            } else {
                None
            }
        }
    }

    fn as_string(&self) -> Option<String> {
        let vt = self.vt();
        // SAFETY: `vt` guarantees the union holds a valid `BSTR` or `PWSTR` until drop.
        unsafe {
            if vt == VT_BSTR.0 {
                Some(self.0.Anonymous.Anonymous.Anonymous.bstrVal.to_string())
            } else if vt == VT_LPWSTR.0 {
                let ptr = self.0.Anonymous.Anonymous.Anonymous.pwszVal;
                if ptr.is_null() {
                    None
                } else {
                    Some(ptr.to_string().unwrap_or_default())
                }
            } else {
                None
            }
        }
    }

    fn as_guid(&self) -> Option<GUID> {
        if self.vt() != VT_CLSID.0 {
            return None;
        }
        // SAFETY: `vt == VT_CLSID` guarantees `puuid` points to a valid `GUID` if non-null.
        unsafe {
            let ptr = self.0.Anonymous.Anonymous.Anonymous.puuid;
            if ptr.is_null() { None } else { Some(*ptr) }
        }
    }

    fn as_i32_vec(&self) -> Option<Vec<i32>> {
        let vt = self.vt();
        if vt == (VT_VECTOR.0 | VT_I4.0) {
            // SAFETY: `vt == VT_VECTOR | VT_I4` ensures `cal` holds `cElems` valid `i32` items.
            unsafe {
                let cal = self.0.Anonymous.Anonymous.Anonymous.cal;
                if cal.cElems == 0 || cal.pElems.is_null() {
                    return Some(Vec::new());
                }
                let slice = std::slice::from_raw_parts(cal.pElems, cal.cElems as usize);
                Some(slice.to_vec())
            }
        } else if vt == (VT_VECTOR.0 | VT_UI4.0) {
            // SAFETY: `vt == VT_VECTOR | VT_UI4` ensures `caul` holds `cElems` valid `u32` items.
            unsafe {
                let caul = self.0.Anonymous.Anonymous.Anonymous.caul;
                if caul.cElems == 0 || caul.pElems.is_null() {
                    return Some(Vec::new());
                }
                let slice = std::slice::from_raw_parts(caul.pElems, caul.cElems as usize);
                Some(slice.iter().map(|&v| v as i32).collect())
            }
        } else {
            None
        }
    }

    fn as_guid_vec(&self) -> Option<Vec<GUID>> {
        if self.vt() != (VT_VECTOR.0 | VT_CLSID.0) {
            return None;
        }
        // SAFETY: `vt == VT_VECTOR | VT_CLSID` ensures `cauuid` holds `cElems` valid `GUID`s.
        unsafe {
            let cauuid = self.0.Anonymous.Anonymous.Anonymous.cauuid;
            if cauuid.cElems == 0 || cauuid.pElems.is_null() {
                return Some(Vec::new());
            }
            let slice = std::slice::from_raw_parts(cauuid.pElems, cauuid.cElems as usize);
            Some(slice.to_vec())
        }
    }

    fn describe(&self) -> String {
        let vt = self.vt();
        if vt == VT_EMPTY.0 {
            return "<empty>".to_owned();
        }
        if let Some(v) = self.as_i32() {
            return format!("{v}");
        }
        if let Some(s) = self.as_string() {
            return s;
        }
        if let Some(g) = self.as_guid() {
            return format_guid(g);
        }
        if let Some(vec) = self.as_i32_vec() {
            return format!("{vec:?}");
        }
        if let Some(vec) = self.as_guid_vec() {
            let formatted: Vec<_> = vec.into_iter().map(format_guid).collect();
            return format!("{formatted:?}");
        }
        format!("<vt={vt}>")
    }
}

impl Drop for PropVarGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` is an initialized `PROPVARIANT`; `PropVariantClear` frees any COM
        // allocations owned by it and resets `vt` to `VT_EMPTY`.
        let _ = unsafe { PropVariantClear(&mut self.0) };
    }
}

fn propspec_id(propid: u32) -> PROPSPEC {
    PROPSPEC {
        ulKind: PRSPEC_PROPID,
        Anonymous: PROPSPEC_0 { propid },
    }
}

fn read_prop(props: &IWiaPropertyStorage, propid: u32) -> windows::core::Result<PropVarGuard> {
    let spec = propspec_id(propid);
    let mut guard = PropVarGuard::new();
    // SAFETY: `spec` is a valid `PROPSPEC` for 1 property and `guard.as_mut_ptr()` points to a
    // zero-initialized `PROPVARIANT` owned by `guard`.
    unsafe { props.ReadMultiple(1, &spec, guard.as_mut_ptr()) }?;
    Ok(guard)
}

fn read_prop_str(props: &IWiaPropertyStorage, propid: u32) -> Option<String> {
    read_prop(props, propid).ok().and_then(|g| g.as_string())
}

fn read_prop_i32(props: &IWiaPropertyStorage, propid: u32) -> Option<i32> {
    read_prop(props, propid).ok().and_then(|g| g.as_i32())
}

fn read_prop_u32(props: &IWiaPropertyStorage, propid: u32) -> Option<u32> {
    read_prop(props, propid).ok().and_then(|g| g.as_u32())
}

fn read_prop_guid(props: &IWiaPropertyStorage, propid: u32) -> Option<GUID> {
    read_prop(props, propid).ok().and_then(|g| g.as_guid())
}

fn write_prop_i32(
    props: &IWiaPropertyStorage,
    propid: u32,
    value: i32,
) -> windows::core::Result<()> {
    let spec = propspec_id(propid);
    let pv = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { lVal: value },
            }),
        },
    };
    // SAFETY: `pv` is a valid `VT_I4` `PROPVARIANT` and `spec` is a valid `PROPSPEC`.
    unsafe { props.WriteMultiple(1, &spec, &pv, 2) }
}

fn write_prop_guid(
    props: &IWiaPropertyStorage,
    propid: u32,
    guid: GUID,
) -> windows::core::Result<()> {
    let spec = propspec_id(propid);
    // SAFETY: Allocate `GUID` on the COM heap so `IWiaPropertyStorage` proxy/stub can copy or
    // `PropVariantClear` it safely if required; verify whether `WriteMultiple` frees it.
    let puuid = unsafe { CoTaskMemAlloc(size_of::<GUID>()) }.cast::<GUID>();
    if puuid.is_null() {
        return Err(WinError::from_hresult(E_FAIL));
    }
    // SAFETY: `puuid` is non-null and holds space for one `GUID`.
    unsafe { puuid.write(guid) };
    let pv = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_CLSID,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { puuid },
            }),
        },
    };
    // SAFETY: `pv` is a valid `VT_CLSID` `PROPVARIANT` with COM-allocated `puuid`.
    // `sti.dll`'s `IWiaPropertyStorage::WriteMultiple` frees `puuid` internally (passing a stack
    // pointer causes `STATUS_HEAP_CORRUPTION` 0xC0000374), so do not double-free `puuid` here.
    unsafe { props.WriteMultiple(1, &spec, &pv, 2) }
}

fn read_prop_attrs(props: &IWiaPropertyStorage, propid: u32) -> Option<(u32, PropVarGuard)> {
    let spec = propspec_id(propid);
    let mut flags = 0u32;
    let mut guard = PropVarGuard::new();
    // SAFETY: Valid COM call with 1 `PROPSPEC`, output `flags`, and output `PROPVARIANT`.
    unsafe { props.GetPropertyAttributes(1, &spec, &mut flags, guard.as_mut_ptr()) }.ok()?;
    Some((flags, guard))
}

fn describe_prop_attrs(props: &IWiaPropertyStorage, propid: u32) -> String {
    let Some((flags, guard)) = read_prop_attrs(props, propid) else {
        return String::new();
    };
    if (flags & WIA_PROP_LIST) != 0 {
        if let Some(vec) = guard.as_i32_vec()
            && vec.len() >= 2
        {
            let count = vec[0].max(0) as usize;
            let nom = vec[1];
            let end = (2 + count).min(vec.len());
            return format!(" List(nom={nom})={:?}", &vec[2..end]);
        }
        return format!(" List={}", guard.describe());
    }
    if (flags & WIA_PROP_RANGE) != 0 {
        if let Some(vec) = guard.as_i32_vec()
            && vec.len() >= 4
        {
            return format!(
                " Range[{}..{} step={} nom={}]",
                vec[0], vec[2], vec[3], vec[1]
            );
        }
        return format!(" Range={}", guard.describe());
    }
    if (flags & WIA_PROP_FLAG) != 0 {
        if let Some(vec) = guard.as_i32_vec()
            && vec.len() >= 2
        {
            return format!(" Flags(nom={:#x}, valid={:#x})", vec[0], vec[1]);
        }
        return format!(" Flags={}", guard.describe());
    }
    String::new()
}

fn format_guid(g: GUID) -> String {
    format!(
        "{{{0:08X}-{1:04X}-{2:04X}-{3:02X}{4:02X}-{5:02X}{6:02X}{7:02X}{8:02X}{9:02X}{10:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7],
    )
}

fn format_name(guid: GUID) -> &'static str {
    if guid == WiaImgFmt_BMP {
        "BMP"
    } else if guid == WiaImgFmt_JPEG {
        "JPEG"
    } else if guid == WiaImgFmt_PNG {
        "PNG"
    } else if guid == WiaImgFmt_TIFF {
        "TIFF"
    } else if guid == windows::Win32::Devices::ImageAcquisition::WiaImgFmt_EXIF {
        "EXIF"
    } else if guid == windows::Win32::Devices::ImageAcquisition::WiaImgFmt_PDFA {
        "PDFA"
    } else if guid == windows::Win32::Devices::ImageAcquisition::WiaImgFmt_RAW {
        "RAW"
    } else {
        "OTHER"
    }
}

fn category_name(guid: GUID) -> &'static str {
    if guid == WIA_CATEGORY_ROOT {
        "ROOT"
    } else if guid == WIA_CATEGORY_FLATBED {
        "FLATBED"
    } else if guid == WIA_CATEGORY_FEEDER {
        "FEEDER"
    } else if guid == WIA_CATEGORY_FEEDER_FRONT {
        "FEEDER_FRONT"
    } else if guid == WIA_CATEGORY_FEEDER_BACK {
        "FEEDER_BACK"
    } else if guid == WIA_CATEGORY_AUTO {
        "AUTO"
    } else if guid == GUID::zeroed() {
        "NONE"
    } else {
        "OTHER"
    }
}

fn format_wia_error(err: &WinError) -> String {
    let code = err.code();
    let label = match code {
        c if c == WIA_ERROR_PAPER_EMPTY => "WIA_ERROR_PAPER_EMPTY",
        c if c == WIA_ERROR_PAPER_JAM => "WIA_ERROR_PAPER_JAM",
        c if c == WIA_ERROR_PAPER_PROBLEM => "WIA_ERROR_PAPER_PROBLEM",
        c if c == WIA_ERROR_OFFLINE => "WIA_ERROR_OFFLINE",
        c if c == WIA_ERROR_BUSY => "WIA_ERROR_BUSY",
        c if c == WIA_ERROR_WARMING_UP => "WIA_ERROR_WARMING_UP",
        c if c == WIA_ERROR_USER_INTERVENTION => "WIA_ERROR_USER_INTERVENTION",
        c if c == WIA_ERROR_ITEM_DELETED => "WIA_ERROR_ITEM_DELETED",
        c if c == WIA_ERROR_DEVICE_COMMUNICATION => "WIA_ERROR_DEVICE_COMMUNICATION",
        c if c == WIA_ERROR_INVALID_COMMAND => "WIA_ERROR_INVALID_COMMAND",
        c if c == WIA_ERROR_INCORRECT_HARDWARE_SETTING => "WIA_ERROR_INCORRECT_HARDWARE_SETTING",
        c if c == WIA_ERROR_DEVICE_LOCKED => "WIA_ERROR_DEVICE_LOCKED",
        c if c == WIA_ERROR_EXCEPTION_IN_DRIVER => "WIA_ERROR_EXCEPTION_IN_DRIVER",
        c if c == WIA_ERROR_INVALID_DRIVER_RESPONSE => "WIA_ERROR_INVALID_DRIVER_RESPONSE",
        c if c == WIA_ERROR_COVER_OPEN => "WIA_ERROR_COVER_OPEN",
        c if c == WIA_ERROR_LAMP_OFF => "WIA_ERROR_LAMP_OFF",
        c if c == WIA_ERROR_DESTINATION => "WIA_ERROR_DESTINATION",
        c if c == WIA_ERROR_NETWORK_RESERVATION_FAILED => "WIA_ERROR_NETWORK_RESERVATION_FAILED",
        c if c == WIA_ERROR_MULTI_FEED => "WIA_ERROR_MULTI_FEED",
        c if c == WIA_ERROR_GENERAL_ERROR => "WIA_ERROR_GENERAL_ERROR",
        c if c == WIA_S_NO_DEVICE_AVAILABLE => "WIA_S_NO_DEVICE_AVAILABLE",
        c if c == HRESULT::from_win32(ERROR_SERVICE_NOT_ACTIVE.0) => "ERROR_SERVICE_NOT_ACTIVE",
        c if c == E_INVALIDARG => "E_INVALIDARG",
        c if c == STG_E_MEDIUMFULL => "STG_E_MEDIUMFULL",
        _ => "HRESULT",
    };
    format!(
        "{label} (0x{:08X}): {}",
        code.0 as u32,
        err.message().trim()
    )
}

#[derive(Debug, Clone)]
struct DeviceInfo {
    id: String,
    name: String,
    manufacturer: String,
    description: String,
    dev_type: u32,
    port: String,
    server: String,
    remote_id: String,
    ui_clsid: String,
    wia_version: String,
    driver_version: String,
    pnp_id: String,
}

impl DeviceInfo {
    fn from_props(props: &IWiaPropertyStorage) -> windows::core::Result<Self> {
        let id = read_prop_str(props, WIA_DIP_DEV_ID).unwrap_or_default();
        if id.is_empty() {
            return Err(WinError::new(E_FAIL, "device has empty WIA_DIP_DEV_ID"));
        }
        Ok(Self {
            id,
            name: read_prop_str(props, WIA_DIP_DEV_NAME).unwrap_or_default(),
            manufacturer: read_prop_str(props, WIA_DIP_VEND_DESC).unwrap_or_default(),
            description: read_prop_str(props, WIA_DIP_DEV_DESC).unwrap_or_default(),
            dev_type: read_prop_u32(props, WIA_DIP_DEV_TYPE).unwrap_or(0),
            port: read_prop_str(props, WIA_DIP_PORT_NAME).unwrap_or_default(),
            server: read_prop_str(props, WIA_DIP_SERVER_NAME).unwrap_or_default(),
            remote_id: read_prop_str(props, WIA_DIP_REMOTE_DEV_ID).unwrap_or_default(),
            ui_clsid: read_prop_str(props, WIA_DIP_UI_CLSID).unwrap_or_default(),
            wia_version: read_prop_str(props, WIA_DIP_WIA_VERSION).unwrap_or_default(),
            driver_version: read_prop_str(props, WIA_DIP_DRIVER_VERSION).unwrap_or_default(),
            pnp_id: read_prop_str(props, WIA_DIP_PNP_ID).unwrap_or_default(),
        })
    }

    fn sti_device_type(&self) -> u32 {
        self.dev_type & 0xFFFF
    }

    fn is_scanner(&self) -> bool {
        self.sti_device_type() == WIA_DIP_SCANNER
    }

    fn transport_classification(&self) -> &'static str {
        let server_lower = self.server.to_ascii_lowercase();
        let pnp_lower = self.pnp_id.to_ascii_lowercase();
        let port_lower = self.port.to_ascii_lowercase();
        if server_lower == "network"
            || pnp_lower.contains("dafwsdprovider")
            || port_lower.starts_with("urn:")
            || port_lower.starts_with("http://")
            || port_lower.starts_with("https://")
        {
            "network (WSD/network server)"
        } else if pnp_lower.contains("\\escl#") || port_lower.starts_with("swd\\escl\\") {
            "local (eSCL; may use IPP-USB or LAN)"
        } else if server_lower == "local" {
            "local"
        } else {
            "unknown"
        }
    }
}

fn create_dev_mgr() -> Result<IWiaDevMgr2, String> {
    // SAFETY: COM is initialized on this thread (`ComGuard`); `WiaDevMgr2` is the system WIA 2.0
    // manager class.
    unsafe { CoCreateInstance(&WiaDevMgr2, None, CLSCTX_LOCAL_SERVER) }.map_err(|e| {
        format!(
            "CoCreateInstance(WiaDevMgr2) failed: {}",
            format_wia_error(&e)
        )
    })
}

fn enumerate_devices(mgr: &IWiaDevMgr2, local_only: bool) -> Result<Vec<DeviceInfo>, String> {
    let flags = if local_only {
        WIA_DEVINFO_ENUM_LOCAL
    } else {
        WIA_DEVINFO_ENUM_ALL
    } as i32;
    // SAFETY: `mgr` is a live `IWiaDevMgr2` on this thread.
    let enum_info = unsafe { mgr.EnumDeviceInfo(flags) }
        .map_err(|e| format!("EnumDeviceInfo({flags}) failed: {}", format_wia_error(&e)))?;
    let mut devices = Vec::new();
    loop {
        let mut props = None;
        let mut fetched = 0u32;
        // SAFETY: `enum_info` is a live `IEnumWIA_DEV_INFO`; output slots are valid.
        let res = unsafe { enum_info.Next(1, &mut props, &mut fetched) };
        if let Err(e) = res {
            return Err(format!(
                "IEnumWIA_DEV_INFO::Next failed: {}",
                format_wia_error(&e)
            ));
        }
        if fetched == 0 {
            break;
        }
        if let Some(prop_storage) = props {
            match DeviceInfo::from_props(&prop_storage) {
                Ok(info) => devices.push(info),
                Err(e) => eprintln!(
                    "warning: skipping device without ID: {}",
                    format_wia_error(&e)
                ),
            }
        }
    }
    Ok(devices)
}

fn dump_all_properties(props: &IWiaPropertyStorage, indent: &str) {
    // SAFETY: `props` is a live `IWiaPropertyStorage`.
    let Ok(enum_props) = (unsafe { props.Enum() }) else {
        println!("{indent}(Enum failed)");
        return;
    };
    let mut stat = [STATPROPSTG::default(); 1];
    loop {
        let mut fetched = 0u32;
        // SAFETY: `enum_props` is live; `stat` has length 1.
        let hr = unsafe { enum_props.Next(&mut stat, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        let propid = stat[0].propid;
        let name = if stat[0].lpwstrName.is_null() {
            String::new()
        } else {
            // SAFETY: `lpwstrName` is a COM-allocated NUL-terminated UTF-16 string from `Next`.
            let s = unsafe { stat[0].lpwstrName.to_string() }.unwrap_or_default();
            // SAFETY: `STATPROPSTG::lpwstrName` must be freed with `CoTaskMemFree`.
            unsafe { CoTaskMemFree(Some(stat[0].lpwstrName.0 as *const c_void)) };
            stat[0].lpwstrName = windows::core::PWSTR::null();
            s
        };
        let val = read_prop(props, propid)
            .map(|g| g.describe())
            .unwrap_or_else(|e| format!("<err: {}>", format_wia_error(&e)));
        let attrs = describe_prop_attrs(props, propid);
        println!("{indent}Prop[{propid}] {name} = {val}{attrs}");
    }
}

fn collect_items(root: &IWiaItem2) -> windows::core::Result<Vec<IWiaItem2>> {
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(item) = stack.pop() {
        out.push(item.clone());
        // SAFETY: `item` is a live `IWiaItem2`.
        let flags = unsafe { item.GetItemType() }.unwrap_or(0);
        if (flags & (WiaItemTypeFolder | WiaItemTypeRoot | WiaItemTypeDevice) as i32) == 0 {
            continue;
        }
        // SAFETY: `EnumChildItems(None)` enumerates all direct child items.
        if let Ok(children) = unsafe { item.EnumChildItems(None) } {
            let mut batch = Vec::new();
            loop {
                let mut child = None;
                let mut fetched = 0u32;
                // SAFETY: `children` is a live `IEnumWiaItem2`.
                unsafe { children.Next(1, &mut child, &mut fetched) }?;
                if fetched == 0 {
                    break;
                }
                if let Some(c) = child {
                    batch.push(c);
                }
            }
            for c in batch.into_iter().rev() {
                stack.push(c);
            }
        }
    }
    Ok(out)
}

fn run_list(details: bool, local_only: bool, dump_props: bool) -> Result<(), String> {
    let mgr = create_dev_mgr()?;
    let devices = enumerate_devices(&mgr, local_only)?;
    println!(
        "Found {} WIA device(s) (scope={}):",
        devices.len(),
        if local_only { "LOCAL" } else { "ALL" }
    );
    for (idx, dev) in devices.iter().enumerate() {
        println!("[{idx}] Name: {}", dev.name);
        println!("    DeviceID: {}", dev.id);
        println!(
            "    Type: {:#x} (sti_type={}, scanner={})",
            dev.dev_type,
            dev.sti_device_type(),
            dev.is_scanner()
        );
        println!("    Manufacturer: {}", dev.manufacturer);
        println!("    Description: {}", dev.description);
        println!("    Server: {}", dev.server);
        println!("    Port: {}", dev.port);
        println!("    RemoteID: {}", dev.remote_id);
        println!("    UI CLSID: {}", dev.ui_clsid);
        println!(
            "    WIA Version: {} | Driver Version: {}",
            dev.wia_version, dev.driver_version
        );
        println!("    PnP ID: {}", dev.pnp_id);
        println!("    Classification: {}", dev.transport_classification());

        if details || dump_props {
            let id_bstr = BSTR::from(dev.id.as_str());
            // SAFETY: `mgr` is live and `id_bstr` is a valid `BSTR`.
            match unsafe { mgr.CreateDevice(0, &id_bstr) } {
                Ok(root) => {
                    let items = collect_items(&root)
                        .map_err(|e| format!("collect_items failed: {}", format_wia_error(&e)))?;
                    println!("    Item tree ({} item(s)):", items.len());
                    for item in items {
                        let props: IWiaPropertyStorage = item
                            .cast()
                            .map_err(|e| format!("cast IWiaPropertyStorage failed: {e}"))?;
                        // SAFETY: `item` is a live `IWiaItem2`.
                        let cat = unsafe { item.GetItemCategory() }.unwrap_or_default();
                        // SAFETY: `item` is a live `IWiaItem2`.
                        let item_type = unsafe { item.GetItemType() }.unwrap_or(0);
                        let name = read_prop_str(&props, WIA_IPA_ITEM_NAME).unwrap_or_default();
                        let full =
                            read_prop_str(&props, WIA_IPA_FULL_ITEM_NAME).unwrap_or_default();
                        println!(
                            "      - Item '{name}' full='{full}' category={} ({}) flags={item_type:#x}",
                            category_name(cat),
                            format_guid(cat)
                        );
                        if (item_type & WiaItemTypeRoot as i32) != 0 {
                            let caps =
                                read_prop_i32(&props, WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES);
                            let sel = read_prop_i32(&props, WIA_DPS_DOCUMENT_HANDLING_SELECT);
                            let status = read_prop_i32(&props, WIA_DPS_DOCUMENT_HANDLING_STATUS);
                            let bed_w = read_prop_i32(&props, WIA_DPS_HORIZONTAL_BED_SIZE);
                            let bed_h = read_prop_i32(&props, WIA_DPS_VERTICAL_BED_SIZE);
                            let adf_w = read_prop_i32(&props, WIA_DPS_HORIZONTAL_SHEET_FEED_SIZE);
                            let adf_h = read_prop_i32(&props, WIA_DPS_VERTICAL_SHEET_FEED_SIZE);
                            let opt_x = read_prop_i32(&props, WIA_DPS_OPTICAL_XRES);
                            let opt_y = read_prop_i32(&props, WIA_DPS_OPTICAL_YRES);
                            println!(
                                "        Handling: caps={caps:?} select={sel:?} status={status:?} | Bed={bed_w:?}x{bed_h:?} | ADF={adf_w:?}x{adf_h:?} | Optical={opt_x:?}x{opt_y:?}"
                            );
                        }
                        if (item_type & WiaItemTypeTransfer as i32) != 0 {
                            println!(
                                "        DataType={:?}{} | BPP={:?}{} | XRes={:?}{} | YRes={:?}{}",
                                read_prop_i32(&props, WIA_IPA_DATATYPE),
                                describe_prop_attrs(&props, WIA_IPA_DATATYPE),
                                read_prop_i32(&props, WIA_IPA_BITS_PER_PIXEL_COMPAT),
                                describe_prop_attrs(&props, WIA_IPA_BITS_PER_PIXEL_COMPAT),
                                read_prop_i32(&props, WIA_IPS_XRES),
                                describe_prop_attrs(&props, WIA_IPS_XRES),
                                read_prop_i32(&props, WIA_IPS_YRES),
                                describe_prop_attrs(&props, WIA_IPS_YRES),
                            );
                            println!(
                                "        XPos={:?}{} | YPos={:?}{} | XExt={:?}{} | YExt={:?}{}",
                                read_prop_i32(&props, WIA_IPS_XPOS),
                                describe_prop_attrs(&props, WIA_IPS_XPOS),
                                read_prop_i32(&props, WIA_IPS_YPOS),
                                describe_prop_attrs(&props, WIA_IPS_YPOS),
                                read_prop_i32(&props, WIA_IPS_XEXTENT),
                                describe_prop_attrs(&props, WIA_IPS_XEXTENT),
                                read_prop_i32(&props, WIA_IPS_YEXTENT),
                                describe_prop_attrs(&props, WIA_IPS_YEXTENT),
                            );
                            if let Ok(transfer) = item.cast::<IWiaTransfer>() {
                                // SAFETY: `transfer` is a live `IWiaTransfer`.
                                if let Ok(fmt_enum) = unsafe { transfer.EnumWIA_FORMAT_INFO() } {
                                    let mut formats = Vec::new();
                                    loop {
                                        let mut info = Default::default();
                                        let mut fetched = 0u32;
                                        // SAFETY: `fmt_enum` is live.
                                        if unsafe { fmt_enum.Next(1, &mut info, &mut fetched) }
                                            .is_err()
                                            || fetched == 0
                                        {
                                            break;
                                        }
                                        formats.push(format!(
                                            "{}({}, tymed={})",
                                            format_name(info.guidFormatID),
                                            format_guid(info.guidFormatID),
                                            info.lTymed
                                        ));
                                    }
                                    println!("        Transfer formats: {}", formats.join(", "));
                                }
                            }
                        }
                        if dump_props {
                            dump_all_properties(&props, "        ");
                        }
                    }
                }
                Err(e) => {
                    println!("    CreateDevice error: {}", format_wia_error(&e));
                }
            }
        }
    }
    Ok(())
}

/// In-memory `IStream` with an explicit byte ceiling checked during `Write`/`SetSize`.
struct BoundedStreamInner {
    state: Mutex<StreamState>,
    max_bytes: usize,
}

struct StreamState {
    buf: Vec<u8>,
    pos: usize,
}

impl BoundedStreamInner {
    fn new(max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(StreamState {
                buf: Vec::new(),
                pos: 0,
            }),
            max_bytes,
        })
    }

    fn into_com_stream(self: &Arc<Self>) -> IStream {
        let boxed = Box::new(RawBoundedStream {
            vtable: &RAW_BOUNDED_STREAM_VTBL,
            ref_count: AtomicU32::new(1),
            inner: Arc::clone(self),
        });
        let raw = Box::into_raw(boxed).cast::<c_void>();
        // SAFETY: `RawBoundedStream` starts with `*const IStream_Vtbl` and has `ref_count == 1`;
        // `IStream::from_raw` takes ownership of that reference and releases it on drop.
        unsafe { IStream::from_raw(raw) }
    }

    fn take_bytes(&self) -> Vec<u8> {
        let mut guard = self.state.lock().expect("stream mutex poisoned");
        guard.pos = 0;
        std::mem::take(&mut guard.buf)
    }
}

#[repr(C)]
struct RawBoundedStream {
    vtable: *const IStream_Vtbl,
    ref_count: AtomicU32,
    inner: Arc<BoundedStreamInner>,
}

const RAW_BOUNDED_STREAM_VTBL: IStream_Vtbl = IStream_Vtbl {
    base__: ISequentialStream_Vtbl {
        base__: IUnknown_Vtbl {
            QueryInterface: raw_stream_qi,
            AddRef: raw_stream_add_ref,
            Release: raw_stream_release,
        },
        Read: raw_stream_read,
        Write: raw_stream_write,
    },
    Seek: raw_stream_seek,
    SetSize: raw_stream_set_size,
    CopyTo: raw_stream_copy_to,
    Commit: raw_stream_commit,
    Revert: raw_stream_revert,
    LockRegion: raw_stream_lock_region,
    UnlockRegion: raw_stream_unlock_region,
    Stat: raw_stream_stat,
    Clone: raw_stream_clone,
};

unsafe extern "system" fn raw_stream_qi(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    if interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: `interface` was checked non-null; initialize out-param to null on error paths.
    unsafe { *interface = null_mut() };
    if iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: `iid` was checked non-null.
    let requested = unsafe { *iid };
    if requested == IUnknown::IID
        || requested == ISequentialStream::IID
        || requested == IStream::IID
    {
        // SAFETY: `this` points to a live `RawBoundedStream`; `AddRef` increments refcount.
        unsafe {
            raw_stream_add_ref(this);
            *interface = this;
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn raw_stream_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let obj = unsafe { &*this.cast::<RawBoundedStream>() };
    obj.ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn raw_stream_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` points to a heap-allocated `RawBoundedStream` with valid refcount.
    let obj = unsafe { &*this.cast::<RawBoundedStream>() };
    let prev = obj.ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        fence(Ordering::Acquire);
        // SAFETY: Last COM reference released; reconstruct `Box` once and drop it.
        unsafe { drop(Box::from_raw(this.cast::<RawBoundedStream>())) };
        0
    } else {
        prev - 1
    }
}

unsafe extern "system" fn raw_stream_read(
    this: *mut c_void,
    pv: *mut c_void,
    cb: u32,
    pcbread: *mut u32,
) -> HRESULT {
    if pv.is_null() && cb > 0 {
        return E_POINTER;
    }
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let inner = unsafe { &(*this.cast::<RawBoundedStream>()).inner };
    let mut guard = inner.state.lock().expect("stream mutex poisoned");
    let avail = guard.buf.len().saturating_sub(guard.pos);
    let to_read = (cb as usize).min(avail);
    if to_read > 0 {
        // SAFETY: Caller supplied writable `pv` for at least `cb >= to_read` bytes.
        unsafe {
            copy_nonoverlapping(guard.buf.as_ptr().add(guard.pos), pv.cast::<u8>(), to_read);
        }
    }
    guard.pos += to_read;
    if !pcbread.is_null() {
        // SAFETY: Caller provided `pcbread` pointer when non-null.
        unsafe { *pcbread = to_read as u32 };
    }
    if to_read < cb as usize { S_FALSE } else { S_OK }
}

unsafe extern "system" fn raw_stream_write(
    this: *mut c_void,
    pv: *const c_void,
    cb: u32,
    pcbwritten: *mut u32,
) -> HRESULT {
    if pv.is_null() && cb > 0 {
        return E_POINTER;
    }
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let inner = unsafe { &(*this.cast::<RawBoundedStream>()).inner };
    let mut guard = inner.state.lock().expect("stream mutex poisoned");
    let Some(end) = guard.pos.checked_add(cb as usize) else {
        return STG_E_MEDIUMFULL;
    };
    if end > inner.max_bytes {
        if !pcbwritten.is_null() {
            // SAFETY: Caller provided valid `pcbwritten` pointer when non-null.
            unsafe { *pcbwritten = 0 };
        }
        return STG_E_MEDIUMFULL;
    }
    if end > guard.buf.len() {
        guard.buf.resize(end, 0);
    }
    if cb > 0 {
        // SAFETY: Caller supplied readable `pv` of `cb` bytes; target slice has length `cb`.
        unsafe {
            copy_nonoverlapping(
                pv.cast::<u8>(),
                guard.buf.as_mut_ptr().add(guard.pos),
                cb as usize,
            );
        }
    }
    guard.pos = end;
    if !pcbwritten.is_null() {
        // SAFETY: Caller provided valid `pcbwritten` pointer when non-null.
        unsafe { *pcbwritten = cb };
    }
    S_OK
}

unsafe extern "system" fn raw_stream_seek(
    this: *mut c_void,
    dlibmove: i64,
    dworigin: STREAM_SEEK,
    plibnewposition: *mut u64,
) -> HRESULT {
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let inner = unsafe { &(*this.cast::<RawBoundedStream>()).inner };
    let mut guard = inner.state.lock().expect("stream mutex poisoned");
    let base: i128 = if dworigin == STREAM_SEEK_SET {
        0
    } else if dworigin == STREAM_SEEK_CUR {
        guard.pos as i128
    } else if dworigin == STREAM_SEEK_END {
        guard.buf.len() as i128
    } else {
        return STG_E_INVALIDFUNCTION;
    };
    let target = base + i128::from(dlibmove);
    if target < 0 || target > inner.max_bytes as i128 {
        return STG_E_INVALIDPARAMETER;
    }
    guard.pos = target as usize;
    if !plibnewposition.is_null() {
        // SAFETY: Caller provided valid `plibnewposition` when non-null.
        unsafe { *plibnewposition = guard.pos as u64 };
    }
    S_OK
}

unsafe extern "system" fn raw_stream_set_size(this: *mut c_void, libnewsize: u64) -> HRESULT {
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let inner = unsafe { &(*this.cast::<RawBoundedStream>()).inner };
    if libnewsize > inner.max_bytes as u64 {
        return STG_E_MEDIUMFULL;
    }
    let mut guard = inner.state.lock().expect("stream mutex poisoned");
    guard.buf.resize(libnewsize as usize, 0);
    if guard.pos > guard.buf.len() {
        guard.pos = guard.buf.len();
    }
    S_OK
}

unsafe extern "system" fn raw_stream_copy_to(
    _this: *mut c_void,
    _pstm: *mut c_void,
    _cb: u64,
    _pcbread: *mut u64,
    _pcbwritten: *mut u64,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn raw_stream_commit(_this: *mut c_void, _grfcommitflags: u32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn raw_stream_revert(_this: *mut c_void) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn raw_stream_lock_region(
    _this: *mut c_void,
    _liboffset: u64,
    _cb: u64,
    _dwlocktype: u32,
) -> HRESULT {
    STG_E_INVALIDFUNCTION
}

unsafe extern "system" fn raw_stream_unlock_region(
    _this: *mut c_void,
    _liboffset: u64,
    _cb: u64,
    _dwlocktype: u32,
) -> HRESULT {
    STG_E_INVALIDFUNCTION
}

unsafe extern "system" fn raw_stream_stat(
    this: *mut c_void,
    pstatstg: *mut STATSTG,
    _grfstatflag: u32,
) -> HRESULT {
    if pstatstg.is_null() {
        return E_POINTER;
    }
    // SAFETY: `this` points to a live `RawBoundedStream`.
    let inner = unsafe { &(*this.cast::<RawBoundedStream>()).inner };
    let guard = inner.state.lock().expect("stream mutex poisoned");
    // SAFETY: Caller provided valid writable `STATSTG`.
    unsafe {
        *pstatstg = STATSTG {
            r#type: STGTY_STREAM.0 as u32,
            cbSize: guard.buf.len() as u64,
            ..Default::default()
        };
    }
    S_OK
}

unsafe extern "system" fn raw_stream_clone(_this: *mut c_void, ppstm: *mut *mut c_void) -> HRESULT {
    if !ppstm.is_null() {
        // SAFETY: Initialize out-param to null.
        unsafe { *ppstm = null_mut() };
    }
    E_NOTIMPL
}

struct MtaStream(IStream);

// SAFETY: This binary initializes COM with `COINIT_MULTITHREADED` (MTA), and
// `CreateStreamOnHGlobal` returns an agile in-memory stream whose COM calls are
// thread-safe across MTA threads.
unsafe impl Send for MtaStream {}
// SAFETY: Shared access is only used through COM `IStream` methods synchronized via MTA/Mutex.
unsafe impl Sync for MtaStream {}

enum PageStream {
    Bounded(Arc<BoundedStreamInner>),
    HGlobal(MtaStream),
}

impl PageStream {
    fn into_bytes(self) -> windows::core::Result<Vec<u8>> {
        match self {
            Self::Bounded(inner) => Ok(inner.take_bytes()),
            Self::HGlobal(stream) => read_all_from_istream(&stream.0),
        }
    }
}

fn read_all_from_istream(stream: &IStream) -> windows::core::Result<Vec<u8>> {
    let mut stat = STATSTG::default();
    // SAFETY: `stream` is a valid `IStream` and `stat` is valid writable memory.
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }?;
    let total = usize::try_from(stat.cbSize).unwrap_or(0);
    // SAFETY: Seek back to start of stream before reading.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }?;
    let mut out = Vec::with_capacity(total);
    let mut chunk = [0u8; 65536];
    loop {
        let mut read = 0u32;
        // SAFETY: `chunk` is valid for `chunk.len()` bytes and `read` is valid.
        let hr = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        if hr.is_err() {
            return Err(WinError::from_hresult(hr));
        }
        if read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read as usize]);
        if hr == S_FALSE {
            break;
        }
    }
    Ok(out)
}

struct TransferCallbackState {
    stream_kind: StreamKind,
    max_page_bytes: usize,
    cancel_on_progress: Option<i32>,
    check_modules: bool,
    modules_checked: AtomicBool,
    pages: Mutex<Vec<PageRecord>>,
    transfer_git_cookie: u32,
    max_observed_bytes: AtomicU64,
    cancelled_in_callback: AtomicBool,
    last_logged_bucket: AtomicI32,
}

struct PageRecord {
    item_name: String,
    stream: PageStream,
}

impl TransferCallbackState {
    fn new(
        stream_kind: StreamKind,
        max_page_bytes: usize,
        cancel_on_progress: Option<i32>,
        check_modules: bool,
        transfer_git_cookie: u32,
    ) -> Arc<Self> {
        Arc::new(Self {
            stream_kind,
            max_page_bytes,
            cancel_on_progress,
            check_modules,
            modules_checked: AtomicBool::new(false),
            pages: Mutex::new(Vec::new()),
            transfer_git_cookie,
            max_observed_bytes: AtomicU64::new(0),
            cancelled_in_callback: AtomicBool::new(false),
            last_logged_bucket: AtomicI32::new(-1),
        })
    }

    fn into_com_callback(self: &Arc<Self>) -> IWiaTransferCallback {
        let boxed = Box::new(RawTransferCallback {
            vtable: &RAW_TRANSFER_CALLBACK_VTBL,
            ref_count: AtomicU32::new(1),
            state: Arc::clone(self),
        });
        let raw = Box::into_raw(boxed).cast::<c_void>();
        // SAFETY: `RawTransferCallback` starts with `*const IWiaTransferCallback_Vtbl` and has
        // `ref_count == 1`; `IWiaTransferCallback::from_raw` owns that reference.
        unsafe { IWiaTransferCallback::from_raw(raw) }
    }
}

#[repr(C)]
struct RawTransferCallback {
    vtable: *const IWiaTransferCallback_Vtbl,
    ref_count: AtomicU32,
    state: Arc<TransferCallbackState>,
}

const RAW_TRANSFER_CALLBACK_VTBL: IWiaTransferCallback_Vtbl = IWiaTransferCallback_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: raw_cb_qi,
        AddRef: raw_cb_add_ref,
        Release: raw_cb_release,
    },
    TransferCallback: raw_cb_transfer,
    GetNextStream: raw_cb_get_next_stream,
};

unsafe extern "system" fn raw_cb_qi(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    if interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: `interface` was checked non-null.
    unsafe { *interface = null_mut() };
    if iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: `iid` was checked non-null.
    let requested = unsafe { *iid };
    if requested == IUnknown::IID || requested == IWiaTransferCallback::IID {
        // SAFETY: `this` points to a live `RawTransferCallback`.
        unsafe {
            raw_cb_add_ref(this);
            *interface = this;
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn raw_cb_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` points to a live `RawTransferCallback`.
    let obj = unsafe { &*this.cast::<RawTransferCallback>() };
    obj.ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn raw_cb_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` points to a heap-allocated `RawTransferCallback`.
    let obj = unsafe { &*this.cast::<RawTransferCallback>() };
    let prev = obj.ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        fence(Ordering::Acquire);
        // SAFETY: Last COM reference released; reconstruct `Box` once and drop it.
        unsafe { drop(Box::from_raw(this.cast::<RawTransferCallback>())) };
        0
    } else {
        prev - 1
    }
}

fn cancel_via_git(cookie: u32) -> Result<(), String> {
    // SAFETY: COM is initialized on the calling thread; `StdGlobalInterfaceTable` is built-in.
    let git: IGlobalInterfaceTable = unsafe {
        CoCreateInstance(
            &CLSID_STD_GLOBAL_INTERFACE_TABLE,
            None,
            windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|e| format!("CoCreateInstance(GIT) failed: {e}"))?;
    let mut raw_ptr: *mut c_void = null_mut();
    // SAFETY: `cookie` was registered in GIT for `IWiaTransfer`.
    unsafe { git.GetInterfaceFromGlobal(cookie, &IWiaTransfer::IID, &mut raw_ptr) }
        .map_err(|e| format!("GetInterfaceFromGlobal failed: {e}"))?;
    if raw_ptr.is_null() {
        return Err("GetInterfaceFromGlobal returned null".to_owned());
    }
    // SAFETY: `GetInterfaceFromGlobal` returned an owned ref-counted `IWiaTransfer`.
    let transfer = unsafe { IWiaTransfer::from_raw(raw_ptr) };
    // SAFETY: `transfer` is a valid COM proxy on this thread.
    unsafe { transfer.Cancel() }.map_err(|e| format_wia_error(&e))
}

unsafe extern "system" fn raw_cb_transfer(
    this: *mut c_void,
    _lflags: i32,
    pwiatransferparams: *const WiaTransferParams,
) -> HRESULT {
    // SAFETY: `this` points to a live `RawTransferCallback`.
    let state = unsafe { &(*this.cast::<RawTransferCallback>()).state };
    if state.check_modules && !state.modules_checked.swap(true, Ordering::SeqCst) {
        print_loaded_modules("during TransferCallback");
    }
    if pwiatransferparams.is_null() {
        return S_OK;
    }
    // SAFETY: WIA provides a valid `WiaTransferParams` pointer for the callback duration.
    let params = unsafe { *pwiatransferparams };
    state
        .max_observed_bytes
        .fetch_max(params.ulTransferredBytes, Ordering::Relaxed);
    let msg_name = match params.lMessage as u32 {
        WIA_TRANSFER_MSG_STATUS => "STATUS",
        WIA_TRANSFER_MSG_END_OF_STREAM => "END_OF_STREAM",
        WIA_TRANSFER_MSG_END_OF_TRANSFER => "END_OF_TRANSFER",
        WIA_TRANSFER_MSG_DEVICE_STATUS => "DEVICE_STATUS",
        WIA_TRANSFER_MSG_NEW_PAGE => "NEW_PAGE",
        _ => "OTHER",
    };
    let should_log = if params.lMessage as u32 != WIA_TRANSFER_MSG_STATUS {
        true
    } else {
        let bucket = params.lPercentComplete / 25;
        let prev = state.last_logged_bucket.swap(bucket, Ordering::Relaxed);
        bucket != prev
    };
    if should_log {
        println!(
            "  [cb] msg={msg_name} ({}) pct={}% bytes={} hr=0x{:08X}",
            params.lMessage,
            params.lPercentComplete,
            params.ulTransferredBytes,
            params.hrErrorStatus.0 as u32
        );
    }
    if params.ulTransferredBytes > state.max_page_bytes as u64 {
        eprintln!(
            "  [cb] page exceeded max_page_bytes ({} > {}), cancelling",
            params.ulTransferredBytes, state.max_page_bytes
        );
        let _ = cancel_via_git(state.transfer_git_cookie);
        return STG_E_MEDIUMFULL;
    }
    if let Some(threshold) = state.cancel_on_progress
        && params.lMessage as u32 == WIA_TRANSFER_MSG_STATUS
        && params.lPercentComplete >= threshold
        && !state.cancelled_in_callback.swap(true, Ordering::SeqCst)
    {
        println!(
            "  [cb] triggering IWiaTransfer::Cancel at {}% (>= {threshold}%)",
            params.lPercentComplete
        );
        let res = cancel_via_git(state.transfer_git_cookie);
        println!("  [cb] IWiaTransfer::Cancel returned {res:?}");
        return S_OK;
    }
    S_OK
}

unsafe extern "system" fn raw_cb_get_next_stream(
    this: *mut c_void,
    _lflags: i32,
    bstritemname: *mut c_void,
    bstrfullitemname: *mut c_void,
    ppdestination: *mut *mut c_void,
) -> HRESULT {
    if ppdestination.is_null() {
        return E_POINTER;
    }
    // SAFETY: `ppdestination` was checked non-null.
    unsafe { *ppdestination = null_mut() };
    // SAFETY: `this` points to a live `RawTransferCallback`.
    let state = unsafe { &(*this.cast::<RawTransferCallback>()).state };

    let name = if bstritemname.is_null() {
        String::new()
    } else {
        // SAFETY: Caller-owned `BSTR` is NUL-terminated UTF-16 while inside this callback.
        unsafe { PCWSTR(bstritemname.cast()).to_string() }.unwrap_or_default()
    };
    let full = if bstrfullitemname.is_null() {
        String::new()
    } else {
        // SAFETY: Caller-owned `BSTR` is NUL-terminated UTF-16 while inside this callback.
        unsafe { PCWSTR(bstrfullitemname.cast()).to_string() }.unwrap_or_default()
    };
    let page_num = state.pages.lock().expect("pages mutex poisoned").len() + 1;
    println!("  [cb] GetNextStream page #{page_num}: name='{name}' full='{full}'");

    let (com_stream, page_stream) = match state.stream_kind {
        StreamKind::Bounded => {
            let inner = BoundedStreamInner::new(state.max_page_bytes);
            let com = inner.into_com_stream();
            (com, PageStream::Bounded(inner))
        }
        StreamKind::HGlobal => {
            // SAFETY: `HGLOBAL::default()` allocates an expandable in-memory stream freed on
            // final release (`fdeleteonrelease = true`).
            let com = match unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) } {
                Ok(s) => s,
                Err(e) => return e.code(),
            };
            (com.clone(), PageStream::HGlobal(MtaStream(com)))
        }
    };

    state
        .pages
        .lock()
        .expect("pages mutex poisoned")
        .push(PageRecord {
            item_name: name,
            stream: page_stream,
        });

    // SAFETY: `ppdestination` was checked non-null; `into_raw` transfers one COM refcount to caller.
    unsafe { *ppdestination = com_stream.into_raw() };
    S_OK
}

fn find_device(devices: &[DeviceInfo], query: &str) -> Result<DeviceInfo, String> {
    if let Some(exact) = devices.iter().find(|d| d.id == query) {
        return Ok(exact.clone());
    }
    let query_lower = query.to_ascii_lowercase();
    let matches: Vec<_> = devices
        .iter()
        .filter(|d| {
            d.id.to_ascii_lowercase().contains(&query_lower)
                || d.name.to_ascii_lowercase().contains(&query_lower)
        })
        .collect();
    match matches.len() {
        0 => Err(format!(
            "device matching '{query}' not found among {} enumerated device(s)",
            devices.len()
        )),
        1 => Ok(matches[0].clone()),
        _ => {
            let names: Vec<_> = matches
                .iter()
                .map(|d| format!("'{}' ({})", d.name, d.id))
                .collect();
            Err(format!(
                "device query '{query}' is ambiguous: {}",
                names.join(", ")
            ))
        }
    }
}

fn select_scan_item(root: &IWiaItem2, source: Source) -> Result<IWiaItem2, String> {
    let items = collect_items(root)
        .map_err(|e| format!("enumerate child items failed: {}", format_wia_error(&e)))?;
    let wanted_cat = match source {
        Source::Flatbed => Some(WIA_CATEGORY_FLATBED),
        Source::Feeder | Source::Duplex => Some(WIA_CATEGORY_FEEDER),
        Source::Auto => Some(WIA_CATEGORY_AUTO),
    };
    let transfer_items: Vec<_> = items
        .into_iter()
        .filter(|item| {
            // SAFETY: `item` is a live `IWiaItem2`.
            let flags = unsafe { item.GetItemType() }.unwrap_or(0);
            (flags & WiaItemTypeRoot as i32) == 0
                && (flags
                    & (WiaItemTypeTransfer | WiaItemTypeImage | WiaItemTypeProgrammableDataSource)
                        as i32)
                    != 0
        })
        .collect();

    if let Some(cat) = wanted_cat {
        for item in &transfer_items {
            // SAFETY: `item` is a live `IWiaItem2`.
            if unsafe { item.GetItemCategory() }.ok() == Some(cat) {
                return Ok(item.clone());
            }
        }
    }
    transfer_items
        .into_iter()
        .next()
        .ok_or_else(|| "scanner device exposes no transferable child item".to_owned())
}

fn configure_source(root: &IWiaItem2, item: &IWiaItem2, source: Source) -> Result<(), String> {
    let root_props: IWiaPropertyStorage = root
        .cast()
        .map_err(|e| format!("root cast IWiaPropertyStorage failed: {e}"))?;
    let item_props: IWiaPropertyStorage = item
        .cast()
        .map_err(|e| format!("item cast IWiaPropertyStorage failed: {e}"))?;

    let caps = read_prop_i32(&root_props, WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES).unwrap_or(0);
    let status = read_prop_i32(&root_props, WIA_DPS_DOCUMENT_HANDLING_STATUS).unwrap_or(0);
    println!("Root handling before scan: caps={caps:#x} status={status:#x}");

    let select_val = match source {
        Source::Flatbed => Some(FLATBED as i32),
        Source::Feeder => Some(FEEDER as i32),
        Source::Duplex => Some((FEEDER | DUPLEX) as i32),
        Source::Auto => None,
    };

    if let Some(sel) = select_val {
        let supported = match source {
            Source::Flatbed => (caps as u32 & FLATBED) != 0,
            Source::Feeder => (caps as u32 & FEEDER) != 0,
            Source::Duplex => (caps as u32 & (FEEDER | DUPLEX)) == (FEEDER | DUPLEX),
            Source::Auto => true,
        };
        if caps != 0 && !supported {
            return Err(format!(
                "requested source {source:?} is not in WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES ({caps:#x})"
            ));
        }
        // Try setting on root (WIA 1.0/WSD/eSCL style) and if supported on item, set there too.
        if let Err(e) = write_prop_i32(&root_props, WIA_DPS_DOCUMENT_HANDLING_SELECT, sel) {
            println!(
                "  note: setting WIA_DPS_DOCUMENT_HANDLING_SELECT={sel} on root failed ({}), trying item",
                format_wia_error(&e)
            );
            write_prop_i32(&item_props, WIA_DPS_DOCUMENT_HANDLING_SELECT, sel).map_err(|e2| {
                format!(
                    "failed to set document handling select={sel}: {}",
                    format_wia_error(&e2)
                )
            })?;
        }
        if matches!(source, Source::Feeder | Source::Duplex) {
            // Set WIA_DPS_PAGES = ALL_PAGES (0) on root or item if supported.
            let _ = write_prop_i32(&root_props, WIA_DPS_PAGES, ALL_PAGES as i32)
                .or_else(|_| write_prop_i32(&item_props, WIA_DPS_PAGES, ALL_PAGES as i32));
        }
    }
    Ok(())
}

fn configure_scan_item(
    root: &IWiaItem2,
    item: &IWiaItem2,
    args: &ScanArgs,
) -> Result<(GUID, i32, i32, i32, i32), String> {
    let root_props: IWiaPropertyStorage = root
        .cast()
        .map_err(|e| format!("root cast IWiaPropertyStorage failed: {e}"))?;
    let props: IWiaPropertyStorage = item
        .cast()
        .map_err(|e| format!("item cast IWiaPropertyStorage failed: {e}"))?;

    // 1. Transfer type and format: in WIA 2.0 `IWiaTransfer::Download`, drivers advertise
    //    `TYMED_FILE` (2) for single-page streams via `GetNextStream` (not `TYMED_CALLBACK`).
    let mut supported_formats: Vec<(GUID, i32)> = Vec::new();
    if let Ok(transfer) = item.cast::<IWiaTransfer>() {
        // SAFETY: `transfer` is a live `IWiaTransfer`.
        if let Ok(fmt_enum) = unsafe { transfer.EnumWIA_FORMAT_INFO() } {
            loop {
                let mut info = Default::default();
                let mut fetched = 0u32;
                // SAFETY: `fmt_enum` is live.
                if unsafe { fmt_enum.Next(1, &mut info, &mut fetched) }.is_err() || fetched == 0 {
                    break;
                }
                supported_formats.push((info.guidFormatID, info.lTymed));
            }
        }
    }

    let (chosen_format, chosen_tymed) = if let Some(req_fmt) = args.format.guid() {
        let tymed = supported_formats
            .iter()
            .find(|(g, _)| *g == req_fmt)
            .map(|(_, t)| *t)
            .unwrap_or(TYMED_FILE.0);
        (Some(req_fmt), tymed)
    } else {
        // Prefer uncompressed/lossless raster formats first (`PNG`, `BMP`, `TIFF`, `JPEG`) rather
        // than `WIA_IPA_PREFERRED_FORMAT`, because eSCL defaults to `PDFA` (`%PDF-1.4`).
        let preferred_raster = [WiaImgFmt_PNG, WiaImgFmt_BMP, WiaImgFmt_TIFF, WiaImgFmt_JPEG];
        if let Some((g, t)) = preferred_raster
            .iter()
            .find_map(|wanted| supported_formats.iter().find(|(g, _)| g == wanted).copied())
        {
            (Some(g), t)
        } else if let Some(pref) = read_prop_guid(&props, WIA_IPA_PREFERRED_FORMAT) {
            let tymed = supported_formats
                .iter()
                .find(|(g, _)| *g == pref)
                .map(|(_, t)| *t)
                .unwrap_or(TYMED_FILE.0);
            (Some(pref), tymed)
        } else {
            (None, TYMED_FILE.0)
        }
    };

    write_prop_i32(&props, WIA_IPA_TYMED, chosen_tymed).map_err(|e| {
        format!(
            "failed to set WIA_IPA_TYMED={chosen_tymed}: {}",
            format_wia_error(&e)
        )
    })?;

    if let Some(fmt_guid) = chosen_format {
        write_prop_guid(&props, WIA_IPA_FORMAT, fmt_guid).map_err(|e| {
            format!(
                "failed to set WIA_IPA_FORMAT={}: {}",
                format_name(fmt_guid),
                format_wia_error(&e)
            )
        })?;
    }

    // 3. Color mode (try Current Intent first if supported, then explicit Data Type / Depth)
    let _ = write_prop_i32(&props, WIA_IPS_CUR_INTENT, args.color.wia_intent());
    write_prop_i32(&props, WIA_IPA_DATATYPE, args.color.wia_data_type()).map_err(|e| {
        format!(
            "failed to set WIA_IPA_DATATYPE={}: {}",
            args.color.wia_data_type(),
            format_wia_error(&e)
        )
    })?;
    // Some drivers update WIA_IPA_DEPTH automatically; set explicitly if needed.
    let current_depth = read_prop_i32(&props, WIA_IPA_DEPTH).unwrap_or(-1);
    if current_depth != args.color.wia_depth() {
        write_prop_i32(&props, WIA_IPA_DEPTH, args.color.wia_depth()).map_err(|e| {
            format!(
                "failed to set WIA_IPA_DEPTH={}: {}",
                args.color.wia_depth(),
                format_wia_error(&e)
            )
        })?;
    }

    // 4. Resolution (must be set BEFORE extents, because extents are in pixels at current DPI)
    let dpi = i32::try_from(args.dpi).map_err(|_| "dpi too large")?;
    write_prop_i32(&props, WIA_IPS_XRES, dpi)
        .map_err(|e| format!("failed to set WIA_IPS_XRES={dpi}: {}", format_wia_error(&e)))?;
    write_prop_i32(&props, WIA_IPS_YRES, dpi)
        .map_err(|e| format!("failed to set WIA_IPS_YRES={dpi}: {}", format_wia_error(&e)))?;

    let actual_xres = read_prop_i32(&props, WIA_IPS_XRES).unwrap_or(dpi);
    let actual_yres = read_prop_i32(&props, WIA_IPS_YRES).unwrap_or(dpi);
    if actual_xres != dpi || actual_yres != dpi {
        return Err(format!(
            "driver adjusted DPI to {actual_xres}x{actual_yres} instead of requested {dpi}"
        ));
    }

    // 5. Page bounds (A4: 210 mm x 297 mm)
    if args.page == PageExtent::A4 {
        let _ = write_prop_i32(&root_props, WIA_IPS_PAGE_SIZE, WIA_PAGE_A4 as i32);
        let _ = write_prop_i32(&props, WIA_IPS_PAGE_SIZE, WIA_PAGE_A4 as i32);

        let a4_x_px = ((210.0_f64 / 25.4_f64) * f64::from(actual_xres)).round() as i32;
        let a4_y_px = ((297.0_f64 / 25.4_f64) * f64::from(actual_yres)).round() as i32;

        // Clamp to driver's reported valid extent range (at current DPI).
        let max_x = match read_prop_attrs(&props, WIA_IPS_XEXTENT).and_then(|(_, g)| g.as_i32_vec())
        {
            Some(v) if v.len() >= 3 => v[2],
            _ => read_prop_i32(&props, WIA_IPS_MAX_HORIZONTAL_SIZE)
                .map(|m| ((f64::from(m) / 1000.0) * f64::from(actual_xres)).round() as i32)
                .unwrap_or(a4_x_px),
        };
        let max_y = match read_prop_attrs(&props, WIA_IPS_YEXTENT).and_then(|(_, g)| g.as_i32_vec())
        {
            Some(v) if v.len() >= 3 => v[2],
            _ => read_prop_i32(&props, WIA_IPS_MAX_VERTICAL_SIZE)
                .map(|m| ((f64::from(m) / 1000.0) * f64::from(actual_yres)).round() as i32)
                .unwrap_or(a4_y_px),
        };

        let target_x = a4_x_px.min(max_x);
        let target_y = a4_y_px.min(max_y);
        let _ = write_prop_i32(&props, WIA_IPS_XPOS, 0);
        let _ = write_prop_i32(&props, WIA_IPS_YPOS, 0);
        write_prop_i32(&props, WIA_IPS_XEXTENT, target_x).map_err(|e| {
            format!(
                "failed to set WIA_IPS_XEXTENT={target_x}: {}",
                format_wia_error(&e)
            )
        })?;
        write_prop_i32(&props, WIA_IPS_YEXTENT, target_y).map_err(|e| {
            format!(
                "failed to set WIA_IPS_YEXTENT={target_y}: {}",
                format_wia_error(&e)
            )
        })?;
    }

    let fmt = read_prop_guid(&props, WIA_IPA_FORMAT).unwrap_or_default();
    let xext = read_prop_i32(&props, WIA_IPS_XEXTENT).unwrap_or(0);
    let yext = read_prop_i32(&props, WIA_IPS_YEXTENT).unwrap_or(0);
    let px = read_prop_i32(&props, WIA_IPA_PIXELS_PER_LINE).unwrap_or(0);
    let lines = read_prop_i32(&props, WIA_IPA_NUMBER_OF_LINES).unwrap_or(0);
    let bpl = read_prop_i32(&props, WIA_IPA_BYTES_PER_LINE).unwrap_or(0);
    let ch = read_prop_i32(&props, WIA_IPA_CHANNELS_PER_PIXEL).unwrap_or(0);
    let bpp = read_prop_i32(&props, WIA_IPA_DEPTH).unwrap_or(0);
    println!(
        "Configured: format={} ({}) res={}x{} extent={}x{} (reported {}x{} bpl={} ch={} bpp={})",
        format_name(fmt),
        format_guid(fmt),
        actual_xres,
        actual_yres,
        xext,
        yext,
        px,
        lines,
        bpl,
        ch,
        bpp
    );
    Ok((fmt, actual_xres, actual_yres, xext, yext))
}

fn list_loaded_modules() -> Result<Vec<String>, String> {
    // SAFETY: `GetCurrentProcess()` returns a pseudo-handle that does not need closing.
    let proc_handle = unsafe { GetCurrentProcess() };
    let mut needed = 0u32;
    let mut modules = vec![HMODULE::default(); 256];
    loop {
        let cb = u32::try_from(modules.len() * size_of::<HMODULE>())
            .map_err(|_| "module buffer size overflow")?;
        // SAFETY: `proc_handle` is current process, `modules` is writable for `cb` bytes.
        unsafe { EnumProcessModules(proc_handle, modules.as_mut_ptr(), cb, &mut needed) }
            .map_err(|e| format!("EnumProcessModules failed: {}", format_wia_error(&e)))?;
        let count = needed as usize / size_of::<HMODULE>();
        if count <= modules.len() {
            modules.truncate(count);
            break;
        }
        modules.resize(count, HMODULE::default());
    }

    let mut paths = Vec::with_capacity(modules.len());
    let mut buf = vec![0u16; MAX_PATH as usize * 4];
    for hmod in modules {
        // SAFETY: `proc_handle` and `hmod` come from `EnumProcessModules`; `buf` is writable.
        let len = unsafe { GetModuleFileNameExW(Some(proc_handle), Some(hmod), &mut buf) };
        if len > 0 {
            let s = OsString::from_wide(&buf[..len as usize])
                .to_string_lossy()
                .into_owned();
            paths.push(s);
        }
    }
    Ok(paths)
}

fn print_loaded_modules(phase: &str) {
    match list_loaded_modules() {
        Ok(mods) => {
            let unique: BTreeSet<String> = mods.into_iter().collect();
            let suspect: Vec<_> = unique
                .iter()
                .filter(|p| {
                    let low = p.to_ascii_lowercase();
                    low.contains("epson")
                        || (!low.contains("\\windows\\system32\\")
                            && !low.contains("\\windows\\winsxs\\")
                            && !low.ends_with(".exe"))
                })
                .cloned()
                .collect();
            println!("=== Loaded modules ({phase}): {} total ===", unique.len());
            for m in &unique {
                println!("  {m}");
            }
            println!(
                "=== Non-System32 / vendor-matching modules ({phase}): {} ===",
                suspect.len()
            );
            for m in suspect {
                println!("  [CHECK] {m}");
            }
        }
        Err(e) => eprintln!("module listing failed ({phase}): {e}"),
    }
}

fn wait_for_enter(prompt: &str) {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

struct WatchdogHandle {
    cookie: u32,
    done_tx: mpsc::Sender<()>,
    timed_out: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl WatchdogHandle {
    fn spawn(
        transfer: &IWiaTransfer,
        timeout: Duration,
        cancel_after: Option<Duration>,
    ) -> Result<Self, String> {
        // Marshal `IWiaTransfer` via the Global Interface Table so the watchdog thread and
        // any MTA callback thread can call `Cancel()` safely across COM apartments.
        // SAFETY: COM is initialized (`ComGuard`); `StdGlobalInterfaceTable` is built-in.
        let git: IGlobalInterfaceTable = unsafe {
            CoCreateInstance(
                &CLSID_STD_GLOBAL_INTERFACE_TABLE,
                None,
                windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|e| format!("CoCreateInstance(GlobalInterfaceTable) failed: {e}"))?;

        // SAFETY: `transfer` is a live `IWiaTransfer`; `RegisterInterfaceInGlobal` returns cookie.
        let cookie = unsafe { git.RegisterInterfaceInGlobal(transfer, &IWiaTransfer::IID) }
            .map_err(|e| format!("RegisterInterfaceInGlobal failed: {e}"))?;

        let (done_tx, done_rx) = mpsc::channel();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_clone = Arc::clone(&timed_out);

        let join = thread::Builder::new()
            .name("wia-watchdog".to_owned())
            .spawn(move || {
                let Ok(_com) = ComGuard::init_mta() else {
                    return;
                };
                let wait_dur = match cancel_after {
                    Some(c) if c < timeout => c,
                    _ => timeout,
                };
                let is_forced_cancel = cancel_after.is_some_and(|c| c < timeout);
                if done_rx.recv_timeout(wait_dur) == Err(mpsc::RecvTimeoutError::Timeout) {
                    if !is_forced_cancel {
                        timed_out_clone.store(true, Ordering::SeqCst);
                        eprintln!(
                            "  [watchdog] scan exceeded timeout ({:?}), calling IWiaTransfer::Cancel",
                            timeout
                        );
                    } else {
                        println!(
                            "  [watchdog] cancel-after-ms ({:?}) elapsed, calling IWiaTransfer::Cancel",
                            wait_dur
                        );
                    }
                    let res = cancel_via_git(cookie);
                    println!("  [watchdog] IWiaTransfer::Cancel returned {res:?}");
                }
            })
            .map_err(|e| format!("failed to spawn watchdog thread: {e}"))?;

        Ok(Self {
            cookie,
            done_tx,
            timed_out,
            join: Some(join),
        })
    }

    fn cookie(&self) -> u32 {
        self.cookie
    }

    fn finish(mut self) -> bool {
        let _ = self.done_tx.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        // SAFETY: `cookie` was registered in GIT on this process; revoke after watchdog joins.
        if let Ok(git) = unsafe {
            CoCreateInstance::<_, IGlobalInterfaceTable>(
                &CLSID_STD_GLOBAL_INTERFACE_TABLE,
                None,
                windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
            )
        } {
            // SAFETY: Revoke our GIT registration cookie once no thread uses it anymore.
            let _ = unsafe { git.RevokeInterfaceFromGlobal(self.cookie) };
        }
        self.timed_out.load(Ordering::SeqCst)
    }
}

fn extension_for_bytes(bytes: &[u8], fmt: GUID) -> &'static str {
    if bytes.starts_with(b"BM") || fmt == WiaImgFmt_BMP {
        "bmp"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || fmt == WiaImgFmt_JPEG {
        "jpg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) || fmt == WiaImgFmt_PNG {
        "png"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") || fmt == WiaImgFmt_TIFF {
        "tif"
    } else {
        "bin"
    }
}

fn run_scan(args: ScanArgs) -> Result<(), String> {
    if args.check_modules {
        print_loaded_modules("before WIA init");
    }

    let mgr = create_dev_mgr()?;
    let device_id = if args.direct_id {
        println!("Using direct DeviceID: '{}'", args.device);
        args.device.clone()
    } else {
        let devices = enumerate_devices(&mgr, false)?;
        let dev = find_device(&devices, &args.device)?;
        println!(
            "Selected device: '{}' | id={} | classification={}",
            dev.name,
            dev.id,
            dev.transport_classification()
        );
        dev.id
    };

    if args.wait_enter == Some(WaitPoint::BeforeCreate) {
        wait_for_enter("Paused before CreateDevice. Press Enter to continue...");
    }

    let id_bstr = BSTR::from(device_id.as_str());
    // SAFETY: `mgr` is live and `id_bstr` is a valid `BSTR`.
    let root = unsafe { mgr.CreateDevice(0, &id_bstr) }
        .map_err(|e| format!("CreateDevice failed: {}", format_wia_error(&e)))?;

    let item = select_scan_item(&root, args.source)?;
    configure_source(&root, &item, args.source)?;
    let (fmt, xres, yres, _xext, _yext) = configure_scan_item(&root, &item, &args)?;

    let transfer: IWiaTransfer = item
        .cast()
        .map_err(|e| format!("item cast IWiaTransfer failed: {e}"))?;

    if args.wait_enter == Some(WaitPoint::BeforeDownload) {
        wait_for_enter("Paused before Download. Press Enter to start scanning...");
    }

    let watchdog = WatchdogHandle::spawn(
        &transfer,
        Duration::from_secs(args.timeout_secs),
        args.cancel_after_ms.map(Duration::from_millis),
    )?;

    let cb_state = TransferCallbackState::new(
        args.stream,
        args.max_page_bytes,
        args.cancel_on_progress,
        args.check_modules,
        watchdog.cookie(),
    );
    let callback_com = cb_state.into_com_callback();

    let started = Instant::now();
    // SAFETY: `transfer` and `callback_com` are live COM objects on this thread.
    let download_res = unsafe { transfer.Download(0, &callback_com) };
    let elapsed = started.elapsed();
    let timed_out = watchdog.finish();
    drop(callback_com);

    let cb_cancelled = cb_state.cancelled_in_callback.load(Ordering::SeqCst);
    let max_progress_bytes = cb_state.max_observed_bytes.load(Ordering::Relaxed);

    if args.check_modules {
        print_loaded_modules("after Download");
    }

    let pages = {
        let mut guard = cb_state.pages.lock().expect("pages mutex poisoned");
        std::mem::take(&mut *guard)
    };

    println!(
        "Download finished in {:.3}s (result={}, timed_out={timed_out}, cb_cancelled={cb_cancelled}, max_progress_bytes={max_progress_bytes}, pages_received={})",
        elapsed.as_secs_f64(),
        match &download_res {
            Ok(()) => "S_OK".to_owned(),
            Err(e) => format_wia_error(e),
        },
        pages.len()
    );

    for (idx, page) in pages.into_iter().enumerate() {
        let page_num = idx + 1;
        let bytes = page
            .stream
            .into_bytes()
            .map_err(|e| format!("reading page #{page_num} stream failed: {e}"))?;
        let magic_hex: String = bytes
            .iter()
            .take(8)
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let ext = extension_for_bytes(&bytes, fmt);
        println!(
            "  Page #{page_num} ('{}'): {} bytes ({:.2} MiB) dpi={}x{} detected_ext={ext} header=[{magic_hex}]",
            page.item_name,
            bytes.len(),
            bytes.len() as f64 / (1024.0 * 1024.0),
            xres,
            yres,
        );
        if let Some(dir) = &args.save_dir {
            fs::create_dir_all(dir)
                .map_err(|e| format!("create save dir {} failed: {e}", dir.display()))?;
            let out_path = dir.join(format!("page_{page_num}_{xres}dpi.{ext}"));
            fs::write(&out_path, &bytes)
                .map_err(|e| format!("write {} failed: {e}", out_path.display()))?;
            println!("    Saved raw page to {}", out_path.display());
        }
    }

    let mut pmc = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: `GetCurrentProcess()` is valid and `pmc` has `cb` initialized.
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) }.is_ok() {
        println!(
            "Process memory (GetProcessMemoryInfo): PeakPagefileUsage={} ({:.2} MiB), PeakWorkingSetSize={} ({:.2} MiB)",
            pmc.PeakPagefileUsage,
            pmc.PeakPagefileUsage as f64 / (1024.0 * 1024.0),
            pmc.PeakWorkingSetSize,
            pmc.PeakWorkingSetSize as f64 / (1024.0 * 1024.0),
        );
    }

    if timed_out {
        return Err(format!("scan timed out after {}s", args.timeout_secs));
    }
    download_res.map_err(|e| format!("IWiaTransfer::Download failed: {}", format_wia_error(&e)))
}

fn parse_args() -> Result<Command, String> {
    let mut args = std::env::args().skip(1);
    let Some(sub) = args.next() else {
        return Err(usage());
    };
    match sub.as_str() {
        "list" => {
            let mut details = false;
            let mut local_only = false;
            let mut dump_props = false;
            for arg in args {
                match arg.as_str() {
                    "--details" => details = true,
                    "--local-only" => local_only = true,
                    "--dump-props" => dump_props = true,
                    _ => return Err(format!("unknown option for list: {arg}\n{}", usage())),
                }
            }
            Ok(Command::List {
                details,
                local_only,
                dump_props,
            })
        }
        "scan" => {
            let mut device = None;
            let mut direct_id = false;
            let mut source = Source::Flatbed;
            let mut color = ColorMode::Color;
            let mut dpi = 300u32;
            let mut page = PageExtent::A4;
            let mut format = ImageFormat::Auto;
            let mut stream = StreamKind::Bounded;
            let mut max_page_bytes = DEFAULT_MAX_PAGE_BYTES;
            let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
            let mut cancel_after_ms = None;
            let mut cancel_on_progress = None;
            let mut check_modules = false;
            let mut wait_enter = None;
            let mut save_dir = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--device" => {
                        device = Some(args.next().ok_or("missing value for --device")?);
                    }
                    "--direct-id" => direct_id = true,
                    "--source" => {
                        let v = args.next().ok_or("missing value for --source")?;
                        source = Source::parse(&v)?;
                    }
                    "--color" => {
                        let v = args.next().ok_or("missing value for --color")?;
                        color = ColorMode::parse(&v)?;
                    }
                    "--dpi" => {
                        let v = args.next().ok_or("missing value for --dpi")?;
                        dpi = v.parse().map_err(|_| format!("invalid --dpi '{v}'"))?;
                    }
                    "--page" => {
                        let v = args.next().ok_or("missing value for --page")?;
                        page = PageExtent::parse(&v)?;
                    }
                    "--format" => {
                        let v = args.next().ok_or("missing value for --format")?;
                        format = ImageFormat::parse(&v)?;
                    }
                    "--stream" => {
                        let v = args.next().ok_or("missing value for --stream")?;
                        stream = StreamKind::parse(&v)?;
                    }
                    "--max-page-bytes" => {
                        let v = args.next().ok_or("missing value for --max-page-bytes")?;
                        max_page_bytes = v
                            .parse()
                            .map_err(|_| format!("invalid --max-page-bytes '{v}'"))?;
                    }
                    "--timeout-secs" => {
                        let v = args.next().ok_or("missing value for --timeout-secs")?;
                        timeout_secs = v
                            .parse()
                            .map_err(|_| format!("invalid --timeout-secs '{v}'"))?;
                    }
                    "--cancel-after-ms" => {
                        let v = args.next().ok_or("missing value for --cancel-after-ms")?;
                        cancel_after_ms = Some(
                            v.parse()
                                .map_err(|_| format!("invalid --cancel-after-ms '{v}'"))?,
                        );
                    }
                    "--cancel-on-progress" => {
                        let v = args
                            .next()
                            .ok_or("missing value for --cancel-on-progress")?;
                        cancel_on_progress = Some(
                            v.parse()
                                .map_err(|_| format!("invalid --cancel-on-progress '{v}'"))?,
                        );
                    }
                    "--modules" => check_modules = true,
                    "--wait-enter" => {
                        let v = args.next().ok_or("missing value for --wait-enter")?;
                        wait_enter = Some(WaitPoint::parse(&v)?);
                    }
                    "--save-dir" => {
                        let v = args.next().ok_or("missing value for --save-dir")?;
                        save_dir = Some(PathBuf::from(v));
                    }
                    _ => return Err(format!("unknown option for scan: {arg}\n{}", usage())),
                }
            }

            let device = device.ok_or_else(|| format!("--device is required\n{}", usage()))?;
            Ok(Command::Scan(ScanArgs {
                device,
                direct_id,
                source,
                color,
                dpi,
                page,
                format,
                stream,
                max_page_bytes,
                timeout_secs,
                cancel_after_ms,
                cancel_on_progress,
                check_modules,
                wait_enter,
                save_dir,
            }))
        }
        "-h" | "--help" | "help" => Err(usage()),
        other => Err(format!("unknown subcommand '{other}'\n{}", usage())),
    }
}

fn usage() -> String {
    "\
Usage:
  wia_poc list [--details] [--local-only] [--dump-props]
  wia_poc scan --device <id-or-name> [--direct-id]
               [--source flatbed|feeder|duplex|auto]
               [--color color|grayscale|bw] [--dpi <N>] [--page a4|default]
               [--format auto|bmp|jpeg|tiff|png] [--stream bounded|hglobal]
               [--max-page-bytes <N>] [--timeout-secs <N>]
               [--cancel-after-ms <N>] [--cancel-on-progress <pct>]
               [--modules] [--wait-enter before-create|before-download]
               [--save-dir <dir>]"
        .to_owned()
}

fn main() {
    let cmd = match parse_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    let _com = match ComGuard::init_mta() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("CoInitializeEx failed: {}", format_wia_error(&e));
            std::process::exit(1);
        }
    };
    let res = match cmd {
        Command::List {
            details,
            local_only,
            dump_props,
        } => run_list(details, local_only, dump_props),
        Command::Scan(scan) => run_scan(scan),
    };
    if let Err(err) = res {
        eprintln!("ERROR: {err}");
        std::process::exit(1);
    }
}
