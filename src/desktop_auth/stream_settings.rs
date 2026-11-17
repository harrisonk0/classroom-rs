use super::{LiveSession, classwork, mutation, require_class_moderator, wire};
use crate::desktop_data::StreamClassworkDisplay;
use prost::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPostingPolicy {
    PostAndComment,
    CommentOnly,
    TeachersOnly,
}

impl StreamPostingPolicy {
    pub const ALL: [Self; 3] = [Self::PostAndComment, Self::CommentOnly, Self::TeachersOnly];

    fn from_wire(value: i32) -> Option<Self> {
        match value {
            2 => Some(Self::PostAndComment),
            3 => Some(Self::CommentOnly),
            4 => Some(Self::TeachersOnly),
            _ => None,
        }
    }

    fn wire(self) -> i32 {
        match self {
            Self::PostAndComment => 2,
            Self::CommentOnly => 3,
            Self::TeachersOnly => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PostAndComment => "Students can post and comment",
            Self::CommentOnly => "Students can only comment",
            Self::TeachersOnly => "Only teachers can post or comment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourseStreamSettings {
    pub posting: Option<StreamPostingPolicy>,
    pub classwork: Option<StreamClassworkDisplay>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassDetails {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub room: String,
}

impl ClassDetails {
    pub fn valid(&self) -> bool {
        !self.title.trim().is_empty() && self.title.chars().count() <= 750
            && self.subtitle.chars().count() <= 2_800
            && self.description.chars().count() <= 30_000
            && self.room.chars().count() <= 650
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeacherClassSettings {
    pub details: ClassDetails,
    pub stream: CourseStreamSettings,
}

pub async fn load_teacher_class_settings(session: &LiveSession, course: i64) -> Result<TeacherClassSettings, String> {
    require_class_moderator(session, course).await?;
    read_active_class_settings(session, course).await
}

pub async fn save_teacher_class_settings(
    session: &LiveSession, course: i64, expected: TeacherClassSettings, desired: TeacherClassSettings,
) -> Result<TeacherClassSettings, String> {
    let current = load_teacher_class_settings(session, course).await?;
    if current != expected {
        return Err("Class settings changed. Review the current settings before saving again.".into());
    }
    let request = class_settings_update(course, &expected, &desired)?;
    let _: mutation::WriteCourseResponse = session.write_rpc("WriteCourse", request, Some(course)).await
        .map_err(|error| format!("{error} Reopen Class settings before trying again."))?;
    let updated = read_active_class_settings(session, course).await?;
    if updated != desired {
        return Err("Classroom did not confirm the requested class settings.".into());
    }
    Ok(updated)
}

async fn read_active_class_settings(session: &LiveSession, course: i64) -> Result<TeacherClassSettings, String> {
    let response: wire::QueryCourseResponse = session.rpc("QueryCourse", wire::QueryCourseRequest {
        course_read_mask: Some(class_settings_mask()),
        filter_options: Some(wire::CourseFilterOptions {
            course_id: vec![wire::CourseId { id: Some(course) }], ..Default::default()
        }),
    }, Some(course)).await?;
    let key = wire::CourseId { id: Some(course) };
    let mut matches = response.course.into_iter().filter(|entry| entry.course_id.as_ref() == Some(&key));
    let Some(row) = matches.next().filter(|row| row.course_state == Some(1)) else {
        return Err("Classroom did not confirm an active class for these settings.".into());
    };
    if matches.next().is_some() {
        return Err("Classroom returned conflicting settings for this class.".into());
    }
    let setting = row.course_setting.unwrap_or_default();
    Ok(TeacherClassSettings {
        details: ClassDetails {
            title: row.title.filter(|title| !title.trim().is_empty()).ok_or("Classroom did not return a class name.")?,
            subtitle: row.subtitle.unwrap_or_default(), description: row.description.unwrap_or_default(),
            room: row.room.unwrap_or_default(),
        },
        stream: CourseStreamSettings {
            posting: setting.student_stream_posting_policy.and_then(StreamPostingPolicy::from_wire),
            classwork: setting.classwork_in_stream_display_type.and_then(StreamClassworkDisplay::from_wire),
        },
    })
}

fn class_settings_mask() -> wire::CourseReadMask {
    wire::CourseReadMask {
        include_course_state: Some(true),
        include_title: Some(true), include_subtitle: Some(true), include_description: Some(true), include_room: Some(true),
        course_setting_read_mask: Some(wire::CourseSettingReadMask {
            include_student_stream_posting_policy: Some(true),
            include_classwork_in_stream_display_type: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn class_settings_update(
    course: i64, expected: &TeacherClassSettings, desired: &TeacherClassSettings,
) -> Result<mutation::WriteCourseRequest, String> {
    if !desired.details.valid() {
        return Err("Enter a class name and check the field lengths.".into());
    }
    if expected == desired {
        return Err("There are no class settings changes to save.".into());
    }
    let title = (expected.details.title != desired.details.title).then(|| desired.details.title.clone());
    let subtitle = (expected.details.subtitle != desired.details.subtitle).then(|| desired.details.subtitle.clone());
    let description = (expected.details.description != desired.details.description).then(|| desired.details.description.clone());
    let room = (expected.details.room != desired.details.room).then(|| desired.details.room.clone());
    let posting = if expected.stream.posting != desired.stream.posting {
        expected.stream.posting.ok_or("The posting policy was not returned.")?;
        Some(desired.stream.posting.ok_or("Choose a posting policy.")?.wire())
    } else { None };
    let classwork = if expected.stream.classwork != desired.stream.classwork {
        expected.stream.classwork.ok_or("The Classwork setting was not returned.")?;
        Some(match desired.stream.classwork.ok_or("Choose a Classwork display option.")? {
            StreamClassworkDisplay::Condensed => 1,
            StreamClassworkDisplay::Expanded => 2,
            StreamClassworkDisplay::Hidden => 3,
        })
    } else { None };
    let stream_changed = posting.is_some() || classwork.is_some();
    let mask = mutation::CourseUpdateMask {
        update_title: title.as_ref().map(|_| true), update_subtitle: subtitle.as_ref().map(|_| true),
        update_description: description.as_ref().map(|_| true), update_room: room.as_ref().map(|_| true),
        course_setting_update_mask: stream_changed.then_some(mutation::CourseSettingUpdateMask {
            update_student_stream_posting_policy: posting.map(|_| true),
            update_classwork_in_stream_display_type: classwork.map(|_| true),
        }),
        ..Default::default()
    };
    Ok(mutation::WriteCourseRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        course_mutation: vec![mutation::CourseMutation {
            course_id: Some(classwork::CourseId { id: Some(course) }),
            course: Some(wire::Course {
                course_id: Some(wire::CourseId { id: Some(course) }),
                title, subtitle, description, room,
                course_setting: stream_changed.then_some(wire::CourseSetting {
                    student_stream_posting_policy: posting, classwork_in_stream_display_type: classwork,
                    ..Default::default()
                }),
                ..Default::default()
            }.encode_to_vec()),
            course_update_mask: Some(mask.encode_to_vec()),
            ..Default::default()
        }],
        course_read_mask: Some(class_settings_mask().encode_to_vec()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_settings_use_independent_update_flags_and_preserve_unavailable_fields() {
        let original = CourseStreamSettings {
            posting: Some(StreamPostingPolicy::PostAndComment), classwork: Some(StreamClassworkDisplay::Condensed),
        };
        let cases = [
            (CourseStreamSettings { posting: Some(StreamPostingPolicy::CommentOnly), ..original }, Some(3), None),
            (CourseStreamSettings { classwork: Some(StreamClassworkDisplay::Hidden), ..original }, None, Some(3)),
            (CourseStreamSettings { posting: Some(StreamPostingPolicy::TeachersOnly), classwork: Some(StreamClassworkDisplay::Expanded) }, Some(4), Some(2)),
        ];
        let settings = |stream| TeacherClassSettings {
            details: ClassDetails { title: "Poetry".into(), ..Default::default() }, stream,
        };
        for (desired, posting, classwork) in cases {
            let decoded = mutation::WriteCourseRequest::decode(class_settings_update(17, &settings(original), &settings(desired))
                .unwrap().encode_to_vec().as_slice()).unwrap();
            assert_eq!(decoded.request_header.unwrap().operation, Some(3));
            let [item] = decoded.course_mutation.as_slice() else { panic!("Expected one class update") };
            assert_eq!(item.course_id.as_ref().unwrap().id, Some(17));
            let payload = wire::Course::decode(item.course.as_deref().unwrap()).unwrap();
            assert_eq!(payload.course_id.unwrap().id, Some(17));
            assert!(payload.course_state.is_none());
            let settings = payload.course_setting.unwrap();
            assert_eq!(settings.student_stream_posting_policy, posting);
            assert_eq!(settings.classwork_in_stream_display_type, classwork);
            let mask = mutation::CourseUpdateMask::decode(item.course_update_mask.as_deref().unwrap()).unwrap()
                .course_setting_update_mask.unwrap();
            assert_eq!(mask.update_student_stream_posting_policy, posting.map(|_| true));
            assert_eq!(mask.update_classwork_in_stream_display_type, classwork.map(|_| true));
        }
        assert!(class_settings_update(17, &settings(original), &settings(original)).is_err());
        assert!(class_settings_update(17, &settings(CourseStreamSettings { posting: None, ..original }), &settings(original)).is_err());
    }

    #[test]
    fn class_details_write_only_changed_text_and_support_clearing_an_optional_field() {
        let original = TeacherClassSettings {
            details: ClassDetails { title: "Literature".into(), subtitle: "Year 12".into(),
                description: "Read poetry.".into(), room: "B14".into() },
            stream: CourseStreamSettings { posting: None, classwork: None },
        };
        let mut desired = original.clone();
        desired.details.description = "Read poetry.\nCompare two sources.".into();
        desired.details.room.clear();
        let request = class_settings_update(17, &original, &desired).unwrap();
        let row = &request.course_mutation[0];
        let payload = wire::Course::decode(row.course.as_deref().unwrap()).unwrap();
        let mask = mutation::CourseUpdateMask::decode(row.course_update_mask.as_deref().unwrap()).unwrap();
        assert_eq!(payload.description, Some(desired.details.description.clone()));
        assert_eq!(payload.room, Some(String::new()));
        assert!(payload.title.is_none() && payload.subtitle.is_none() && payload.course_setting.is_none());
        assert_eq!((mask.update_description, mask.update_room), (Some(true), Some(true)));
        assert!(mask.update_title.is_none() && mask.update_subtitle.is_none() && mask.course_setting_update_mask.is_none());
        desired.details.title.clear();
        assert!(class_settings_update(17, &original, &desired).is_err());
    }
}
