use std::fmt::Write as _;

// A stale marker can be edited outside Phyro. Persist only numeric enums
// and short model/version numbers; never copy arbitrary marker text into a report.
const VENDORS: &[&str] = &[
    "Unrecognized",
    "NVIDIA",
    "AMD",
    "Intel",
    "Apple",
    "Microsoft",
    "Qualcomm",
    "Mesa",
];
const FAMILIES: &[&str] = &[
    "Unrecognized",
    "RTX",
    "GTX",
    "RX",
    "UHD",
    "Iris",
    "Arc",
    "Adreno",
    "Radeon",
    "llvmpipe",
];

pub(super) struct Graphics {
    vendor: usize,
    family: usize,
    model: u32,
    gl_major: u8,
    gl_minor: u8,
}
impl Graphics {
    pub(super) fn from_driver(vendor: &str, renderer: &str, version: &str) -> Self {
        let vendor_name = vendor.to_ascii_uppercase();
        let vendor = if vendor_name.contains("ADVANCED MICRO DEVICES")
            || vendor_name.starts_with("ATI ")
            || vendor_name == "ATI"
        {
            2
        } else {
            VENDORS
                .iter()
                .skip(1)
                .position(|name| vendor_name.contains(&name.to_ascii_uppercase()))
                .map_or(0, |i| i + 1)
        };
        let words: Vec<_> = renderer
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| !s.is_empty())
            .take(32)
            .collect();
        let (family, model) = FAMILIES
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(family, name)| {
                let index = words
                    .iter()
                    .position(|word| word.eq_ignore_ascii_case(name))?;
                let model = words
                    .iter()
                    .skip(index + 1)
                    .take(3)
                    .find_map(|word| {
                        (word.len() <= 5)
                            .then(|| word.parse::<u32>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                Some((family, model))
            })
            .unwrap_or((0, 0));
        let vendor = if vendor == 0 {
            match FAMILIES[family] {
                "RTX" | "GTX" => 1,
                "RX" | "Radeon" => 2,
                "UHD" | "Iris" | "Arc" => 3,
                "Adreno" => 6,
                _ => 0,
            }
        } else {
            vendor
        };
        let (gl_major, gl_minor) = version
            .split_ascii_whitespace()
            .find_map(|word| {
                let mut parts = word.split('.');
                let major = parts.next()?.parse::<u8>().ok()?;
                let minor = parts.next()?.parse::<u8>().ok()?;
                (major <= 9 && minor <= 99).then_some((major, minor))
            })
            .unwrap_or((0, 0));
        Self {
            vendor,
            family,
            model,
            gl_major,
            gl_minor,
        }
    }
    pub(super) fn encode(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.vendor, self.family, self.model, self.gl_major, self.gl_minor
        )
    }
    pub(super) fn decode(raw: &str) -> Option<Self> {
        let mut words = raw.split_ascii_whitespace();
        let vendor = words.next()?.parse().ok()?;
        let family = words.next()?.parse().ok()?;
        let model = words.next()?.parse().ok()?;
        let gl_major = words.next()?.parse().ok()?;
        let gl_minor = words.next()?.parse().ok()?;
        if words.next().is_some()
            || vendor >= VENDORS.len()
            || family >= FAMILIES.len()
            || model > 99_999
            || gl_major > 9
            || gl_minor > 99
        {
            return None;
        }
        Some(Self {
            vendor,
            family,
            model,
            gl_major,
            gl_minor,
        })
    }
    pub(super) fn describe(&self) -> String {
        let mut result = format!(
            "GPU vendor: {}\nGPU family: {}",
            VENDORS[self.vendor], FAMILIES[self.family]
        );
        if self.model != 0 {
            let _ = write!(result, " {}", self.model);
        }
        result.push('\n');
        if self.gl_major != 0 {
            let _ = writeln!(result, "OpenGL: {}.{}", self.gl_major, self.gl_minor);
        }
        result
    }
}
#[cfg(windows)]
pub(super) fn os_version() -> String {
    use windows_sys::{
        Wdk::System::SystemServices::RtlGetVersion,
        Win32::System::SystemInformation::OSVERSIONINFOW,
    };
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion requires a correctly sized, writable OSVERSIONINFOW.
    if unsafe { RtlGetVersion(&mut version) } == 0 {
        format!(
            "{}.{}.{}",
            version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
        )
    } else {
        "unavailable".into()
    }
}

#[cfg(target_os = "linux")]
pub(super) fn os_version() -> String {
    // A custom kernel build suffix may contain a host name. Keep only the version.
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|s| {
            let version = s
                .trim()
                .split(|c: char| !c.is_ascii_digit() && c != '.')
                .next()?
                .trim_matches('.');
            (!version.is_empty() && version.len() <= 32).then(|| version.to_owned())
        })
        .unwrap_or_else(|| "unavailable".into())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn os_version() -> String {
    "unavailable".into()
}

#[cfg(windows)]
pub(super) fn module_offsets() -> String {
    use windows_sys::Win32::System::{
        Diagnostics::Debug::RtlCaptureStackBackTrace,
        LibraryLoader::GetModuleHandleW,
        ProcessStatus::{K32GetModuleInformation, MODULEINFO},
        Threading::GetCurrentProcess,
    };
    let mut frames = [std::ptr::null_mut(); 64];
    let mut module = MODULEINFO::default();
    // Both APIs copy into fixed local arrays/structs. GetModuleHandleW(NULL)
    // selects this executable; addresses outside its loaded image are discarded.
    let count = unsafe {
        let handle = GetModuleHandleW(std::ptr::null());
        if handle.is_null()
            || K32GetModuleInformation(
                GetCurrentProcess(),
                handle,
                &mut module,
                size_of::<MODULEINFO>() as u32,
            ) == 0
        {
            return String::new();
        }
        RtlCaptureStackBackTrace(
            1,
            frames.len() as u32,
            frames.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    let base = module.lpBaseOfDll as usize;
    let mut out = String::from("Executable frame offsets (RVA):");
    for &p in &frames[..usize::from(count)] {
        if let Some(offset) = (p as usize)
            .checked_sub(base)
            .filter(|n| *n < module.SizeOfImage as usize)
        {
            let _ = write!(out, " +0x{offset:x}");
        }
    }
    out.push('\n');
    out
}

#[cfg(not(windows))]
pub(super) fn module_offsets() -> String {
    String::new()
}
