//! Read local Windows hang evidence only while the recovered report is being viewed.
use super::{millis, utc, Report, MAX_BYTES, MAX_REPORTS};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::System::EventLog::*;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct Session {
    marker_ms: u64,
    pid: u32,
}

impl Session {
    fn from_report(report: &Report, body: &str) -> Option<Self> {
        let key = report
            .filename
            .strip_prefix("crash-")?
            .strip_suffix("-exit.txt")?;
        let mut parts = key.split('-');
        let timestamp = parts.next()?.parse::<u64>().ok()?;
        let marker_ms = parts.next()?.parse::<u64>().ok()?;
        let pid = parts.next()?.parse::<u32>().ok()?;
        let _sequence = parts.next()?.parse::<u64>().ok()?;
        let header = format!(
            "Unexpected exit\nSession start (UTC): {}\n",
            report.timestamp_utc()
        );
        (parts.next().is_none()
            && timestamp == marker_ms
            && millis(report.timestamp) == marker_ms
            && pid != 0
            && super::report_body(body)?.starts_with(&header))
        .then_some(Self { marker_ms, pid })
    }

    fn matches(self, name: &str, pid: u64, started: SystemTime) -> bool {
        // Marker creation follows process startup. Allow the marker's millisecond rounding.
        let marker = UNIX_EPOCH + Duration::from_millis(self.marker_ms);
        (name.eq_ignore_ascii_case("phyro.exe") || name.eq_ignore_ascii_case("loopline.exe"))
            && pid == u64::from(self.pid)
            && started <= marker + Duration::from_millis(1)
            && marker.duration_since(started).unwrap_or_default() <= Duration::from_secs(5)
    }
}

type ResultReceiver = Receiver<Option<SystemTime>>;

#[derive(Default)]
pub(crate) struct Lookup {
    selected: Option<Session>,
    running: Option<(Session, ResultReceiver)>,
    results: HashMap<Session, Option<SystemTime>>,
}

impl Lookup {
    pub(crate) fn select(&mut self, report: &Report, body: &str) {
        self.selected = Session::from_report(report, body);
    }

    pub(crate) fn update(&mut self, ctx: &eframe::egui::Context, body: &mut String) -> bool {
        let Some(selected) = self.selected else {
            return false;
        };
        if let Some((session, receiver)) = &self.running {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => None,
            };
            if self.results.len() >= MAX_REPORTS {
                self.results.clear();
            }
            self.results.insert(*session, result);
            self.running = None;
        }
        if let Some(result) = self.results.get(&selected) {
            self.selected = None;
            if let Some(at) = result {
                let line = format!(
                    "\nWindows Application log: Application Hang (Event ID 1002) at {}.\n",
                    utc(*at)
                );
                if body.len() + line.len() <= MAX_BYTES {
                    body.push_str(&line);
                    return true;
                }
            }
        } else {
            let ctx = ctx.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            self.running = Some((selected, rx));
            if std::thread::Builder::new()
                .name("phyro-hang-report".into())
                .spawn(move || {
                    let _ = tx.send(find(selected));
                    ctx.request_repaint();
                })
                .is_err()
            {
                self.running = None;
                self.selected = None;
            }
        }
        false
    }
}

struct Handle(EVT_HANDLE);
impl Handle {
    fn new(raw: EVT_HANDLE) -> Option<Self> {
        (raw != 0).then(|| Self(raw))
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // Each nonzero handle returned by wevtapi is owned by one Handle on this thread.
        unsafe {
            EvtClose(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn find(session: Session) -> Option<SystemTime> {
    let start = UNIX_EPOCH.checked_add(Duration::from_millis(session.marker_ms))?;
    let end = start
        .checked_add(Duration::from_secs(7 * 86400))?
        .min(SystemTime::now());
    if start > end {
        return None;
    }
    let iso = |t| utc(t).replace(" UTC", "Z").replace(' ', "T");
    let filter = wide(&format!(
        "*[System[Provider[@Name='Application Hang'] and EventID=1002 and TimeCreated[@SystemTime >= '{}' and @SystemTime <= '{}']]]",
        iso(start), iso(end + Duration::from_secs(1))
    ));
    let channel = wide("Application");
    // Only the local Application channel and these four fields are accessed.
    let query = Handle::new(unsafe {
        EvtQuery(
            0,
            channel.as_ptr(),
            filter.as_ptr(),
            EvtQueryChannelPath | EvtQueryForwardDirection,
        )
    })?;
    let paths = [
        "Event/EventData/Data[@Name='AppName']",
        "Event/EventData/Data[@Name='ProcessId']",
        "Event/EventData/Data[@Name='StartTime']",
        "Event/System/TimeCreated/@SystemTime",
    ]
    .map(wide);
    let ptrs = paths.each_ref().map(|s| s.as_ptr());
    let context = Handle::new(unsafe {
        EvtCreateRenderContext(ptrs.len() as u32, ptrs.as_ptr(), EvtRenderContextValues)
    })?;
    let deadline = Instant::now() + Duration::from_millis(500);
    for _ in 0..32 {
        if Instant::now() >= deadline {
            break;
        }
        let mut raw = 0;
        let mut count = 0;
        if unsafe { EvtNext(query.0, 1, &mut raw, 50, 0, &mut count) } == 0 {
            break;
        }
        let event = Handle::new(raw)?;
        if count == 1 {
            if let Some(at) = matched_event(context.0, event.0, session) {
                return Some(at);
            }
        }
    }
    None
}

fn matched_event(context: EVT_HANDLE, event: EVT_HANDLE, session: Session) -> Option<SystemTime> {
    // EVT_VARIANT gives the buffer native alignment. Oversized or incomplete records are skipped.
    let mut buffer = [EVT_VARIANT::default(); 64];
    let capacity = std::mem::size_of_val(&buffer);
    let mut used = 0;
    let mut count = 0;
    if unsafe {
        EvtRender(
            context,
            event,
            EvtRenderEventValues,
            capacity as u32,
            buffer.as_mut_ptr().cast(),
            &mut used,
            &mut count,
        )
    } == 0
        || count != 4
        || used as usize > capacity
        || (used as usize) < 4 * std::mem::size_of::<EVT_VARIANT>()
    {
        return None;
    }
    let string = |value: &EVT_VARIANT| {
        if value.Type != EvtVarTypeString as u32 {
            return None;
        }
        let ptr = unsafe { value.Anonymous.StringVal };
        let offset = (ptr as usize).checked_sub(buffer.as_ptr() as usize)?;
        let remaining = (used as usize).checked_sub(offset)?;
        if offset % 2 != 0 || remaining < 2 {
            return None;
        }
        // EvtRender stores strings in this caller-owned buffer. Bound the pointer and terminator.
        let start = unsafe { buffer.as_ptr().cast::<u8>().add(offset).cast::<u16>() };
        let chars = unsafe { std::slice::from_raw_parts(start, (remaining / 2).min(64)) };
        String::from_utf16(&chars[..chars.iter().position(|c| *c == 0)?]).ok()
    };
    #[allow(non_upper_case_globals)]
    let number = |value: &EVT_VARIANT| match value.Type as i32 {
        EvtVarTypeHexInt32 | EvtVarTypeUInt32 => Some(unsafe { value.Anonymous.UInt32Val } as u64),
        EvtVarTypeHexInt64 | EvtVarTypeUInt64 | EvtVarTypeFileTime => {
            Some(unsafe { value.Anonymous.UInt64Val })
        }
        _ => {
            let s = string(value)?;
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).ok()
            } else {
                s.parse().ok()
            }
        }
    };
    let name = string(&buffer[0])?;
    let pid = number(&buffer[1])?;
    let started = from_filetime(number(&buffer[2])?)?;
    session.matches(&name, pid, started).then_some(())?;
    let at = from_filetime(number(&buffer[3])?)?;
    (at >= started).then_some(at)
}

fn from_filetime(ticks: u64) -> Option<SystemTime> {
    let since_unix = ticks.checked_sub(116_444_736_000_000_000)?;
    UNIX_EPOCH.checked_add(Duration::new(
        since_unix / 10_000_000,
        ((since_unix % 10_000_000) * 100) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(ms: u64, pid: u32) -> (Report, String) {
        let report = Report {
            filename: format!("crash-{ms}-{ms}-{pid}-0-exit.txt"),
            timestamp: UNIX_EPOCH + Duration::from_millis(ms),
        };
        let body = format!("Phyro crash report\nUnexpected exit\nSession start (UTC): {}\nPrevious build: unknown\n",report.timestamp_utc());
        (report, body)
    }

    #[test]
    fn requires_a_valid_recovered_session_and_both_windows_process_identifiers() {
        let start = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        let (r, body) = report(millis(start) + 20, 0x9218);
        let s = Session::from_report(&r, &body).unwrap();
        assert!(s.matches("PhYrO.exe", 0x9218, start));
        assert!(s.matches("Loopline.exe", 0x9218, start));
        assert!(!s.matches("phyro-cli.exe", 0x9218, start));
        assert!(!s.matches("phyro.exe", 0x9219, start));
        assert!(!s.matches("phyro.exe", 0x9218, start - Duration::from_secs(6)));
        assert!(!s.matches("phyro.exe", 0x9218, start + Duration::from_secs(1)));
        assert!(
            Session::from_report(&r, &body.replace("Unexpected exit", "Main thread panic"))
                .is_none()
        );
        let mut renamed = r;
        renamed.filename = renamed
            .filename
            .replacen("1700000000020", "1700000000021", 1);
        assert!(Session::from_report(&renamed, &body).is_none());
        assert_eq!(from_filetime(116_444_736_000_000_000), Some(UNIX_EPOCH));
        assert!(from_filetime(0).is_none());
    }

    #[test]
    fn viewing_an_old_report_adds_only_the_event_and_utc_once_and_never_crosses_sessions() {
        let (r, mut body) = report(1_700_000_000_000, 12);
        let (other, mut other_text) = report(1_700_000_001_000, 12);
        let mut lookup = Lookup::default();
        let session = Session::from_report(&r, &body).unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        let at = r.timestamp + Duration::from_secs(91);
        lookup.running = Some((session, receive));
        lookup
            .results
            .insert(Session::from_report(&other, &other_text).unwrap(), None);
        lookup.select(&other, &other_text);
        send.send(Some(at)).unwrap();
        let ctx = eframe::egui::Context::default();
        assert!(!lookup.update(&ctx, &mut other_text));
        assert!(!other_text.contains("Application Hang"));
        let original = body.clone();
        lookup.select(&r, &body);
        assert!(lookup.update(&ctx, &mut body));
        assert!(!lookup.update(&ctx, &mut body));
        assert_eq!(body.strip_prefix(&original).unwrap(), "\nWindows Application log: Application Hang (Event ID 1002) at 2023-11-14 22:14:51 UTC.\n");
    }
}
