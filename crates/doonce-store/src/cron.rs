//! Minimal 5-field cron parser with timezone support.

use chrono::{Datelike, Duration, NaiveDateTime, SecondsFormat, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use crate::{Result, StoreError};

pub fn next_run(cron: &str, timezone: &str) -> Result<String> {
    let tz: Tz = timezone
        .trim()
        .parse()
        .map_err(|_| StoreError::Invalid(format!("unknown timezone \"{timezone}\"")))?;
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(StoreError::Invalid(
            "cron must have exactly 5 fields".into(),
        ));
    }
    let minutes = parse_field(fields[0], 0, 59)?;
    let hours = parse_field(fields[1], 0, 23)?;
    let days_of_month = parse_field(fields[2], 1, 31)?;
    let months = parse_field(fields[3], 1, 12)?;
    let days_of_week = parse_field(fields[4], 0, 7)?
        .into_iter()
        .map(|day| if day == 7 { 0 } else { day })
        .collect::<Vec<_>>();

    let dom_restricted = fields[2] != "*";
    let dow_restricted = fields[4] != "*";
    let start = Utc::now()
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .unwrap_or_else(Utc::now)
        + Duration::minutes(1);
    let mut current: NaiveDateTime = start.naive_utc();
    let limit = current + Duration::days(366);

    while current <= limit {
        let local = tz.from_utc_datetime(&current);
        let day_match = if dom_restricted && dow_restricted {
            days_of_month.contains(&local.day())
                || days_of_week.contains(&local.weekday().num_days_from_sunday())
        } else if dom_restricted {
            days_of_month.contains(&local.day())
        } else if dow_restricted {
            days_of_week.contains(&local.weekday().num_days_from_sunday())
        } else {
            true
        };
        if months.contains(&local.month())
            && day_match
            && hours.contains(&local.hour())
            && minutes.contains(&local.minute())
        {
            return Ok(current
                .and_utc()
                .to_rfc3339_opts(SecondsFormat::Millis, true));
        }
        current += Duration::minutes(1);
    }
    Err(StoreError::Invalid(format!(
        "cron expression never matches within a year: {cron}"
    )))
}

fn parse_field(field: &str, low: u32, high: u32) -> Result<Vec<u32>> {
    let mut output = Vec::new();
    for part in field.split(',') {
        let (base, step) = part.split_once('/').unwrap_or((part, "1"));
        let step: u32 = step
            .parse()
            .map_err(|_| StoreError::Invalid(format!("invalid cron step: {part}")))?;
        if step == 0 {
            return Err(StoreError::Invalid(format!("invalid cron step: {part}")));
        }
        let values: Vec<u32> = if base == "*" {
            (low..=high).collect()
        } else if let Some((start, end)) = base.split_once('-') {
            let start: u32 = start
                .parse()
                .map_err(|_| StoreError::Invalid(format!("invalid cron value: {start}")))?;
            let end: u32 = end
                .parse()
                .map_err(|_| StoreError::Invalid(format!("invalid cron value: {end}")))?;
            if start > end {
                return Err(StoreError::Invalid(format!(
                    "invalid cron range: {start}-{end}"
                )));
            }
            (start..=end).collect()
        } else {
            let value: u32 = base
                .parse()
                .map_err(|_| StoreError::Invalid(format!("invalid cron value: {base}")))?;
            if value < low || value > high {
                return Err(StoreError::Invalid(format!(
                    "cron value out of range: {value}"
                )));
            }
            vec![value]
        };
        for (index, value) in values.iter().enumerate() {
            if index % step as usize == 0 {
                output.push(*value);
            }
        }
    }
    output.sort_unstable();
    output.dedup();
    if output.is_empty() {
        return Err(StoreError::Invalid(format!(
            "cron field matches nothing: {field}"
        )));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_the_next_matching_minute() {
        let next = next_run("0 9 * * *", "UTC").unwrap();
        assert!(next.contains('T'));
        assert!(next.ends_with('Z'));
        let next = next_run("*/15 * * * *", "Europe/Istanbul").unwrap();
        assert!(next.contains('T'));
    }

    #[test]
    fn rejects_invalid_expressions() {
        assert!(next_run("* * * *", "UTC").is_err());
        assert!(next_run("* * * * *", "Not/AZone").is_err());
        assert!(next_run("99 * * * *", "UTC").is_err());
        assert!(next_run("*/0 * * * *", "UTC").is_err());
    }
}
