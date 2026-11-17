use super::https_host;

pub(super) fn calendar_url(value: &str) -> Option<String> {
    match https_host(value)?.as_str() {
        "calendar.google.com" => Some(value.to_owned()),
        "www.google.com" if value.starts_with("https://www.google.com/calendar/") =>
            Some(value.replacen("https://www.google.com", "https://calendar.google.com", 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_direct_and_legacy_google_calendar_links_only() {
        let direct = "https://calendar.google.com/calendar/u/0?cid=fixture";
        assert_eq!(calendar_url(direct).as_deref(), Some(direct));
        assert_eq!(calendar_url("https://www.google.com/calendar/embed?src=fixture").as_deref(),
            Some("https://calendar.google.com/calendar/embed?src=fixture"));
        for value in [
            "http://calendar.google.com/calendar",
            "https://calendar.google.com.example/calendar",
            "https://calendar.google.com@example.org/calendar",
            "https://www.google.com/url?calendar=fixture",
        ] {
            assert!(calendar_url(value).is_none());
        }
    }
}
