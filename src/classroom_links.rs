use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use tonic::codegen::http::Uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassroomLink {
    Course { course: i64, classwork: bool },
    Item { course: i64, item: i64, kind: ItemLinkKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLinkKind {
    Assignment,
    ShortAnswer,
    MultipleChoice,
}

impl ClassroomLink {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.len() > 4096 {
            return None;
        }
        let uri = value.split('#').next()?.parse::<Uri>().ok()?;
        if uri.scheme_str() != Some("https") || !uri.host()?.eq_ignore_ascii_case("classroom.google.com")
            || uri.authority()?.as_str().contains('@') || !matches!(uri.port_u16(), None | Some(443))
        {
            return None;
        }
        let parts: Vec<_> = uri.path().strip_prefix('/')?.trim_end_matches('/').split('/').collect();
        let path = match parts.as_slice() {
            ["u", account, rest @ ..] if !account.is_empty() && account.bytes().all(|byte| byte.is_ascii_digit()) => rest,
            path => path,
        };
        match path {
            ["c", course] | ["w", course] => Some(Self::Course {
                course: link_id(course)?,
                classwork: path[0] == "w",
            }),
            ["c", course, kind, item, "details" | "dw"] => Some(Self::Item {
                course: link_id(course)?,
                item: link_id(item)?,
                kind: match *kind {
                    "a" => ItemLinkKind::Assignment,
                    "sa" => ItemLinkKind::ShortAnswer,
                    "mc" => ItemLinkKind::MultipleChoice,
                    _ => return None,
                },
            }),
            _ => None,
        }
    }

    pub fn course(self) -> i64 {
        match self {
            Self::Course { course, .. } | Self::Item { course, .. } => course,
        }
    }

    pub fn web_url(self) -> Option<String> {
        let course = encoded_id(self.course())?;
        Some(match self {
            Self::Course { classwork, .. } =>
                format!("https://classroom.google.com/{}/{course}", if classwork { "w" } else { "c" }),
            Self::Item { item, kind, .. } => {
                let item = encoded_id(item)?;
                let segment = match kind {
                    ItemLinkKind::Assignment => "a",
                    ItemLinkKind::ShortAnswer => "sa",
                    ItemLinkKind::MultipleChoice => "mc",
                };
                format!("https://classroom.google.com/c/{course}/{segment}/{item}/details")
            }
        })
    }
}

fn encoded_id(id: i64) -> Option<String> {
    (id > 0).then(|| URL_SAFE_NO_PAD.encode(id.to_string()))
}

fn link_id(value: &str) -> Option<i64> {
    fn decimal(value: &str) -> Option<i64> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value.parse::<i64>().ok().filter(|id| *id > 0)
    }
    decimal(value).or_else(|| {
        if value.len() > 28 {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
        decimal(std::str::from_utf8(&bytes).ok()?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_course_classwork_and_all_observed_assignment_routes() {
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/u/1/w/MTIz"),
            Some(ClassroomLink::Course { course: 123, classwork: true }));
        for kind in [ItemLinkKind::Assignment, ItemLinkKind::ShortAnswer, ItemLinkKind::MultipleChoice] {
            let target = ClassroomLink::Item { course: 123, item: 456, kind };
            let url = target.web_url().unwrap();
            assert_eq!(ClassroomLink::parse(&url), Some(target));
            assert_eq!(ClassroomLink::parse(&url.replace("/details", "/dw?authuser=1")), Some(target));
        }
    }

    #[test]
    fn refuses_unrelated_hosts_and_routes() {
        for value in [
            "https://classroom.google.com.example/c/123",
            "https://example.org@classroom.google.com/c/123",
            "http://classroom.google.com/c/123",
            "https://classroom.google.com:8080/c/123",
            "https://classroom.google.com/c/123/a/456/submissions",
            "https://classroom.google.com/c/0",
            "https://classroom.google.com/c/123/a/%34%35%36/details",
        ] {
            assert!(ClassroomLink::parse(value).is_none(), "{value}");
        }
    }
}
