//! Local time helpers for clock-like widgets. Formatting follows the user's locale.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::{PCWSTR, w};

pub fn now() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

/// Milliseconds until the next multiple of `period_ms` in the local day.
/// `period_ms = 60_000` gives the next minute boundary, `1_000` the next second.
pub fn ms_until_next(st: &SYSTEMTIME, period_ms: u32) -> u32 {
    let ms_of_day = ((st.wHour as u32 * 60 + st.wMinute as u32) * 60 + st.wSecond as u32) * 1000
        + st.wMilliseconds as u32;
    period_ms - ms_of_day % period_ms
}

/// Seconds into the current minute, with milliseconds.
pub fn seconds_in_minute(st: &SYSTEMTIME) -> f32 {
    st.wSecond as f32 + st.wMilliseconds as f32 / 1000.0
}

/// Time in the user's locale format, e.g. "13:05" or "1:05 PM".
pub fn format_time(st: &SYSTEMTIME, seconds: bool) -> Vec<u16> {
    let flags = if seconds { TIME_FORMAT_FLAGS(0) } else { TIME_NOSECONDS };
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(crate::i18n::locale(), flags, Some(st), PCWSTR::null(), Some(&mut buf)) };
    buf[..(n.max(1) - 1) as usize].to_vec()
}

/// Long date in the user's locale format, e.g. "Thursday, November 5, 2026".
pub fn format_date(st: &SYSTEMTIME) -> Vec<u16> {
    let mut buf = [0u16; 128];
    let n = unsafe {
        GetDateFormatEx(crate::i18n::locale(), DATE_LONGDATE, Some(st), PCWSTR::null(), Some(&mut buf), PCWSTR::null())
    };
    buf[..(n.max(1) - 1) as usize].to_vec()
}

/// Values of the built-in `clock` provider, for markup bindings like `{clock.time}`.
/// Angles are degrees clockwise from 12 o'clock, for analog hands.
pub(crate) fn clock_values(st: &SYSTEMTIME) -> Vec<(&'static str, String)> {
    let date = |f: PCWSTR| {
        let mut buf = [0u16; 64];
        let n = unsafe { GetDateFormatEx(crate::i18n::locale(), ENUM_DATE_FORMATS_FLAGS(0), Some(st), f, Some(&mut buf), PCWSTR::null()) };
        String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
    };
    let (h, m, s) = (st.wHour as f32, st.wMinute as f32, seconds_in_minute(st));
    vec![
        ("clock.time", String::from_utf16_lossy(&format_time(st, false))),
        ("clock.time_seconds", String::from_utf16_lossy(&format_time(st, true))),
        ("clock.date", String::from_utf16_lossy(&format_date(st))),
        ("clock.weekday", date(w!("dddd"))),
        ("clock.weekday_short", date(w!("ddd"))),
        ("clock.day", st.wDay.to_string()),
        ("clock.month", date(w!("MMMM"))),
        ("clock.month_short", date(w!("MMM"))),
        ("clock.month_day", month_day(st)),
        ("clock.year", st.wYear.to_string()),
        ("clock.hour", format!("{:02}", st.wHour)),
        ("clock.minute", format!("{:02}", st.wMinute)),
        ("clock.second", format!("{:02}", st.wSecond)),
        ("clock.hour_angle", format!("{}", (h % 12.0) * 30.0 + m * 0.5 + s / 120.0)),
        ("clock.minute_angle", format!("{}", m * 6.0 + s * 0.1)),
        ("clock.second_angle", format!("{}", s * 6.0)),
    ]
}

/// Day and month in the user's locale format, e.g. "September 22" or "22 сентября".
fn month_day(st: &SYSTEMTIME) -> String {
    let mut buf = [0u16; 64];
    // The locale's month-day pattern, "MMMM d" or "d MMMM" (DATE_MONTHDAY fails without a format).
    let pattern = windows::core::HSTRING::from(locale_string(LOCALE_SMONTHDAY));
    let n = unsafe { GetDateFormatEx(crate::i18n::locale(), ENUM_DATE_FORMATS_FLAGS(0), Some(st), &pattern, Some(&mut buf), PCWSTR::null()) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// Values of the built-in `cal` provider: a month grid for calendar widgets.
/// - `cal.wd0`..`cal.wd6`: weekday initials, starting on the user's first day of the week.
/// - `cal.d0`..`cal.d41`: day numbers of the current month in a 6x7 grid, "" outside the month.
/// - `cal.t0`..`cal.t41`: "on" for today.
/// - `cal.c0`..`cal.c41`: text color token: "#FFFFFF" for today (drawn on a highlight),
///   "secondary" for weekends, "foreground" otherwise.
/// - `cal.we0`..`cal.we6`: "on" for weekend columns. `cal.row5`: "on" when the 6th row is used.
/// - `cal.month`: month name in capitals.
pub(crate) fn calendar_values(st: &SYSTEMTIME) -> Vec<(String, String)> {
    let first = first_day_of_week();
    let grid = month_grid(st.wYear as i32, st.wMonth as u32, first);
    let mut v = Vec::with_capacity(140);
    for col in 0..7 {
        // 0 = Monday, as LOCALE_SSHORTESTDAYNAME1 is Monday.
        let wd = (first + col) % 7;
        v.push((format!("cal.wd{col}"), weekday_initial(wd)));
        v.push((format!("cal.we{col}"), if wd >= 5 { "on" } else { "off" }.into()));
    }
    for (i, &d) in grid.iter().enumerate() {
        let today = d != 0 && d as u16 == st.wDay;
        let weekend = (first + i as u32 % 7) % 7 >= 5;
        v.push((format!("cal.d{i}"), if d == 0 { String::new() } else { d.to_string() }));
        v.push((format!("cal.t{i}"), if today { "on" } else { "off" }.into()));
        let color = if today { "#FFFFFF" } else if weekend { "secondary" } else { "foreground" };
        v.push((format!("cal.c{i}"), color.into()));
    }
    v.push(("cal.row5".into(), if grid[35] != 0 { "on" } else { "off" }.into()));
    // Nominative month name, uppercase: "NOVEMBER", "НОЯБРЬ".
    v.push(("cal.month".into(), locale_string(LOCALE_SMONTHNAME1 + st.wMonth as u32 - 1).to_uppercase()));
    v
}

/// Day numbers of `month` laid out in 6 weeks of 7 days starting on `first` (0 = Monday).
/// 0 marks cells outside the month.
pub(crate) fn month_grid(year: i32, month: u32, first: u32) -> [u8; 42] {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(month - 1) as usize];
    let offset = (weekday(year, month, 1) + 7 - first) % 7;
    let mut grid = [0u8; 42];
    for d in 1..=days {
        grid[(offset + d - 1) as usize] = d as u8;
    }
    grid
}

/// Day of the week, 0 = Monday (Sakamoto's method).
fn weekday(year: i32, month: u32, day: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let sunday0 = (y + y / 4 - y / 100 + y / 400 + T[(month - 1) as usize] + day as i32).rem_euclid(7);
    ((sunday0 + 6) % 7) as u32
}

/// The user's first day of the week, 0 = Monday.
fn first_day_of_week() -> u32 {
    // The user's region, not the display language, decides where the week starts.
    let mut buf = [0u16; 8];
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_IFIRSTDAYOFWEEK, Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]).trim().parse().unwrap_or(0) % 7
}

/// First letter of the shortest localized weekday name, uppercase: "M", "T", "П".
fn weekday_initial(wd: u32) -> String {
    let name = locale_string(LOCALE_SSHORTESTDAYNAME1 + wd);
    name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

fn locale_string(kind: u32) -> String {
    let mut buf = [0u16; 32];
    let n = unsafe { GetLocaleInfoEx(crate::i18n::locale(), kind, Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u16, m: u16, s: u16, ms: u16) -> SYSTEMTIME {
        SYSTEMTIME { wHour: h, wMinute: m, wSecond: s, wMilliseconds: ms, ..Default::default() }
    }

    #[test]
    fn next_boundaries() {
        assert_eq!(ms_until_next(&at(12, 34, 56, 789), 60_000), 3_211);
        assert_eq!(ms_until_next(&at(12, 34, 56, 789), 1_000), 211);
        assert_eq!(ms_until_next(&at(12, 0, 0, 0), 60_000), 60_000);
        assert_eq!(ms_until_next(&at(23, 59, 59, 999), 60_000), 1);
    }

    #[test]
    fn hand_angles() {
        let v: std::collections::BTreeMap<_, _> = clock_values(&at(15, 30, 0, 0)).into_iter().collect();
        assert_eq!(v["clock.hour_angle"], "105");
        assert_eq!(v["clock.minute_angle"], "180");
        assert_eq!(v["clock.second_angle"], "0");
        let mut day = at(15, 30, 0, 0);
        (day.wYear, day.wMonth, day.wDay) = (2026, 9, 22);
        let v: std::collections::BTreeMap<_, _> = clock_values(&day).into_iter().collect();
        assert!(v["clock.month_day"].contains("22"), "{}", v["clock.month_day"]);
    }

    #[test]
    fn calendar_grid() {
        // 2026-11-01 is a Sunday.
        assert_eq!(weekday(2026, 11, 1), 6);
        assert_eq!(weekday(2024, 2, 29), 3);
        let monday = month_grid(2026, 11, 0);
        assert_eq!((monday[5], monday[6], monday[35], monday[36]), (0, 1, 30, 0));
        let sunday = month_grid(2026, 11, 6);
        assert_eq!((sunday[0], sunday[29], sunday[30]), (1, 30, 0));
        // Leap February starting on Thursday.
        assert_eq!(month_grid(2024, 2, 0)[3 + 28], 29);
    }

    #[test]
    fn seconds() {
        assert_eq!(seconds_in_minute(&at(0, 0, 30, 500)), 30.5);
    }
}
