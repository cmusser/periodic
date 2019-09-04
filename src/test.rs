#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use chrono::prelude::*;

#[cfg(test)]
use time::get_start_delay_from_next;

#[test]
fn test_start_delay_next_hour_16_45_hour15() {
    let cur_time = Local
        .datetime_from_str("2019-09-02T16:45:00", "%Y-%m-%dT%H:%M:%S")
        .unwrap();
    match get_start_delay_from_next(cur_time, "hour+15") {
        Ok(delay) => assert_eq!(delay, Duration::from_secs(1800)),
        Err(e) => panic!(e),
    }
}

#[test]
fn test_start_delay_next_hour_22_45_hour15() {
    let cur_time = Local
        .datetime_from_str("2019-09-02T22:45:00", "%Y-%m-%dT%H:%M:%S")
        .unwrap();
    match get_start_delay_from_next(cur_time, "hour+15") {
        Ok(delay) => assert_eq!(delay, Duration::from_secs(1800)),
        Err(e) => panic!(e),
    }
}

#[test]
fn test_start_delay_next_hour_23_01_hour() {
    let cur_time = Local
        .datetime_from_str("2019-09-02T23:01:00", "%Y-%m-%dT%H:%M:%S")
        .unwrap();
    match get_start_delay_from_next(cur_time, "hour") {
        Ok(delay) => assert_eq!(delay, Duration::from_secs(3540)),
        Err(e) => panic!(e),
    }
}

#[test]
fn test_start_delay_next_hour_23_45_hour20() {
    let cur_time = Local
        .datetime_from_str("2019-09-02T23:45:00", "%Y-%m-%dT%H:%M:%S")
        .unwrap();
    match get_start_delay_from_next(cur_time, "hour+20") {
        Ok(delay) => assert_eq!(delay, Duration::from_secs(2100)),
        Err(e) => panic!(e),
    }
}

#[test]
fn test_start_delay_next_hour_23_45_hour() {
    let cur_time = Local
        .datetime_from_str("2019-09-02T23:45:00", "%Y-%m-%dT%H:%M:%S")
        .unwrap();
    match get_start_delay_from_next(cur_time, "hour") {
        Ok(delay) => assert_eq!(delay, Duration::from_secs(900)),
        Err(e) => panic!(e),
    }
}
