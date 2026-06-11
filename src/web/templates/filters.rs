use chrono::{DateTime, Utc};

pub fn fmt_long_date(d: &DateTime<Utc>) -> ::askama::Result<String> {
    Ok(d.format("%B %d, %Y").to_string())
}

pub fn fmt_short_date(d: &DateTime<Utc>) -> ::askama::Result<String> {
    Ok(d.format("%b %d, %Y").to_string())
}

#[allow(dead_code)]
pub fn fmt_long_date_opt(d: &Option<DateTime<Utc>>) -> ::askama::Result<String> {
    Ok(d.map(|x| x.format("%B %d, %Y").to_string())
        .unwrap_or_default())
}

#[allow(dead_code)]
pub fn fmt_short_date_opt(d: &Option<DateTime<Utc>>) -> ::askama::Result<String> {
    Ok(d.map(|x| x.format("%b %d, %Y").to_string())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 9, 12, 14, 30, 0).unwrap()
    }

    #[test]
    fn fmt_long_date_renders_full_month_name() {
        assert_eq!(fmt_long_date(&fixture()).unwrap(), "September 12, 2025");
    }

    #[test]
    fn fmt_short_date_renders_abbreviated_month() {
        assert_eq!(fmt_short_date(&fixture()).unwrap(), "Sep 12, 2025");
    }

    #[test]
    fn fmt_long_date_opt_renders_some() {
        assert_eq!(
            fmt_long_date_opt(&Some(fixture())).unwrap(),
            "September 12, 2025"
        );
    }

    #[test]
    fn fmt_long_date_opt_returns_empty_for_none() {
        assert_eq!(fmt_long_date_opt(&None).unwrap(), "");
    }

    #[test]
    fn fmt_short_date_opt_renders_some() {
        assert_eq!(
            fmt_short_date_opt(&Some(fixture())).unwrap(),
            "Sep 12, 2025"
        );
    }

    #[test]
    fn fmt_short_date_opt_returns_empty_for_none() {
        assert_eq!(fmt_short_date_opt(&None).unwrap(), "");
    }
}
