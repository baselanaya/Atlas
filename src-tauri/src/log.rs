// Small append-only log — %LOCALAPPDATA%\Atlas\atlas.log on Windows,
// ~/.local/state/atlas/atlas.log on Linux. Nothing leaves the machine.
//
// Also the one place local time is computed: the log stamp and the settings
// backup names in hooks.rs both go through LocalTime::now().

use std::io::Write;

use crate::settings;

/// Broken-down local time. GetLocalTime on Windows, localtime_r on Unix: the
/// log and the backup names should read like the user's clock, not UTC.
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalTime {
    pub fn now() -> LocalTime {
        #[cfg(windows)]
        {
            let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
            LocalTime {
                year: t.wYear as i32,
                month: t.wMonth as u32,
                day: t.wDay as u32,
                hour: t.wHour as u32,
                minute: t.wMinute as u32,
                second: t.wSecond as u32,
            }
        }
        #[cfg(unix)]
        {
            let now = unsafe { libc::time(std::ptr::null_mut()) };
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
                return LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0 };
            }
            LocalTime {
                year: tm.tm_year + 1900,
                month: (tm.tm_mon + 1) as u32,
                day: tm.tm_mday as u32,
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
                second: tm.tm_sec as u32,
            }
        }
    }
}

pub fn line(message: impl AsRef<str>) {
    let t = LocalTime::now();
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let dir = settings::local_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("atlas.log");
    // Keep it from growing forever: start fresh past ~1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{stamp} {}", message.as_ref());
    }
}
