use std::time::Duration;

use regex::Regex;
use chrono::prelude::*;

const DAY_SECONDS: u64 = (60 * 60 * 24);

pub fn get_start_delay_from_next(now: DateTime<Local>, next: &str) -> Result<Duration, String> {
    let re = Regex::new(r"(?P<interval>hour|minute)(\+(?P<after>\d{1,2}))?$").unwrap();
    match re.captures(next) {
        Some(time) => {
            let after = match time.name("after") {
                Some(num) => num.as_str().parse::<u32>().unwrap(),
                None => 0,
            };

            let start_at = if &time["interval"] == "hour" {
                // Start some number of minutes after the hour.
                let start_minute = now.minute();
                if start_minute <= after {
                    // The start minute has not been reached within the current hour
                    // so the difference the requested minute and now is the delay.
                    now + chrono::Duration::minutes((after - start_minute) as i64)
                } else {
                    // The start minute has already been reached in the current hour, so
                    // advance to the next hour after the present one, then add the "after"
                    // value as minutes.
                    let next = now + chrono::Duration::hours(1);
                    Local
                        .ymd(next.year(), next.month(), next.day())
                        .and_hms(next.hour(), after, 0)
                }
            } else {
                // Start some number of seconds after the minute.
                let start_second = now.second();
                if start_second <= after {
                    // The start second has not been reached within the current minute
                    // so the difference the requested second and now is the delay.
                    now + chrono::Duration::seconds((after - start_second) as i64)
                } else {
                    // The start second has already been reached in the current second, so
                    // advance to the next minute after the present one, then add the "after"
                    // value as seconds.
                    let next = now + chrono::Duration::minutes(1);
                    Local.ymd(next.year(), next.month(), next.day()).and_hms(
                        next.hour(),
                        next.minute(),
                        after,
                    )
                }
            };
            Ok(Duration::from_secs(
                start_at.signed_duration_since(now).num_seconds().abs() as u64,
            ))
        }
        None => Err(String::from(format!(
            "invalid format for start time: {}",
            next
        ))),
    }
}

pub fn get_start_delay_from_hh_mm(start_at: &str) -> Result<Duration, String> {
    let re = Regex::new(r"(?P<hour>\d{2}):(?P<minute>\d{2})").unwrap();
    match re.captures(start_at) {
        Some(time) => {
            let now = Utc::now();
            if let Some(start_at) = Local::today().and_hms_opt(
                (&time["hour"]).parse::<u32>().unwrap(),
                (&time["minute"]).parse::<u32>().unwrap(),
                0,
            ) {
                let start_delay = match start_at.signed_duration_since(now).num_seconds() {
                    diff if diff >= 0 => diff as u64,
                    diff => DAY_SECONDS - (diff.abs() as u64),
                };
                Ok(Duration::from_secs(start_delay))
            } else {
                Err(String::from(format!(
                    "invalid values found in start time: {}",
                    start_at
                )))
            }
        }
        None => Err(String::from(format!(
            "invalid format for start time: {}",
            start_at
        ))),
    }
}
