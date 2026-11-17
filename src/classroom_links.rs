use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use tonic::codegen::http::Uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassroomLink {
    Course { course: i64, classwork: bool },
    Topic { course: i64, topic: i64 },
    YourWork { course: i64, student: i64 },
    Item { course: i64, item: i64, kind: ItemLinkKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLinkKind {
    Assignment,
    ShortAnswer,
    MultipleChoice,
    Material,
    Announcement,
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
            ["w", course, "t", "all"] => Some(Self::Course { course: link_id(course)?, classwork: true }),
            ["w", course, "tc", topic] => Some(Self::Topic {
                course: link_id(course)?, topic: link_id(topic)?,
            }),
            ["c", course, "sp", student, "all", "default"] =>
                Some(Self::YourWork { course: link_id(course)?, student: link_id(student)? }),
            ["c", course, kind, item, "details" | "dw"] => Some(Self::Item {
                course: link_id(course)?,
                item: link_id(item)?,
                kind: match *kind {
                    "a" => ItemLinkKind::Assignment,
                    "sa" => ItemLinkKind::ShortAnswer,
                    "mc" => ItemLinkKind::MultipleChoice,
                    "m" => ItemLinkKind::Material,
                    "p" => ItemLinkKind::Announcement,
                    _ => return None,
                },
            }),
            _ => None,
        }
    }

    pub fn course(self) -> i64 {
        match self {
            Self::Course { course, .. } | Self::Topic { course, .. } | Self::YourWork { course, .. }
                | Self::Item { course, .. } => course,
        }
    }

    pub fn web_url(self) -> Option<String> {
        let course = encoded_id(self.course())?;
        Some(match self {
            Self::Course { classwork, .. } =>
                format!("https://classroom.google.com/{}/{course}", if classwork { "w" } else { "c" }),
            Self::Topic { topic, .. } => {
                let topic = encoded_id(topic)?;
                format!("https://classroom.google.com/w/{course}/tc/{topic}")
            }
            Self::YourWork { student, .. } => {
                let student = encoded_id(student)?;
                format!("https://classroom.google.com/c/{course}/sp/{student}/all/default")
            }
            Self::Item { item, kind, .. } => {
                let item = encoded_id(item)?;
                let segment = match kind {
                    ItemLinkKind::Assignment => "a",
                    ItemLinkKind::ShortAnswer => "sa",
                    ItemLinkKind::MultipleChoice => "mc",
                    ItemLinkKind::Material => "m",
                    ItemLinkKind::Announcement => "p",
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
        let decoded = std::str::from_utf8(&bytes).ok()?;
        let numeric = decoded.trim_end_matches('Z');
        let padding = decoded.len() - numeric.len();
        if padding > 0 && (padding > 2 || decoded.len() % 3 != 0) {
            return None;
        }
        decimal(numeric)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_course_classwork_and_all_observed_assignment_routes() {
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/u/1/w/MTIz"),
            Some(ClassroomLink::Course { course: 123, classwork: true }));
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/u/1/w/MTIz/t/all"),
            Some(ClassroomLink::Course { course: 123, classwork: true }));
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/c/MVpa/a/NDVa/dw"),
            Some(ClassroomLink::Item { course: 1, item: 45, kind: ItemLinkKind::Assignment }));
        for kind in [ItemLinkKind::Assignment, ItemLinkKind::ShortAnswer, ItemLinkKind::MultipleChoice,
            ItemLinkKind::Material, ItemLinkKind::Announcement] {
            let target = ClassroomLink::Item { course: 123, item: 456, kind };
            let url = target.web_url().unwrap();
            assert_eq!(ClassroomLink::parse(&url), Some(target));
            assert_eq!(ClassroomLink::parse(&url.replace("/details", "/dw?authuser=1")), Some(target));
        }
        let topic = ClassroomLink::Topic { course: 123, topic: 456 };
        assert_eq!(ClassroomLink::parse(&topic.web_url().unwrap()), Some(topic));
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/u/1/w/MTIz/tc/NDVa"),
            Some(ClassroomLink::Topic { course: 123, topic: 45 }));
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
            "https://classroom.google.com/c/ MVo=",
            "https://classroom.google.com/c/MTIzWlpa",
        ] {
            assert!(ClassroomLink::parse(value).is_none(), "{value}");
        }
    }

    #[test]
    fn parses_only_the_observed_student_work_route() {
        let target = ClassroomLink::YourWork { course: 123, student: 456 };
        assert_eq!(ClassroomLink::parse(&target.web_url().unwrap()), Some(target));
        assert_eq!(ClassroomLink::parse("https://classroom.google.com/u/1/c/MTIz/sp/NDU2/all/default"), Some(target));
        assert!(ClassroomLink::parse("https://classroom.google.com/c/MTIz/sp/NDU2/all/grades").is_none());
        assert!(ClassroomLink::parse("https://classroom.google.com/c/MTIz/sp/0/all/default").is_none());
    }
}
