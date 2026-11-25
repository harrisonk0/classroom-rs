use super::{
    wire, classwork, https_host, artwork_host_is_allowed, class_calendar, class_order, Course, CourseTheme,
    Snapshot, Message,
};

pub(super) fn course_details_request() -> wire::QueryCourseRequest {
    let mut request = crate::course_list_candidate();
    if let Some(filter) = &mut request.filter_options {
        filter.latest_enrollment_event = vec![2]; // ADDED
    }
    if let Some(mask) = &mut request.course_read_mask {
        mask.include_course_state = Some(true);
        mask.include_owner_user_id = Some(true);
        mask.include_description = Some(true);
        mask.include_room = Some(true);
        mask.include_calendar_url = Some(true);
        mask.include_reordering_sort_key = Some(true);
        mask.current_user_setting_read_mask = Some(wire::CourseUserSettingReadMask {
            include_course_visibility_setting: Some(true),
            ..Default::default()
        });
        mask.video_call_info_read_mask = Some(wire::VideoCallInfoReadMask {
            include_video_call_url_state: Some(true),
            include_video_call_url: Some(true),
            include_video_call_url_type: None,
        });
        mask.course_setting_read_mask = Some(wire::CourseSettingReadMask {
            include_student_stream_posting_policy: None,
            include_classwork_in_stream_display_type: Some(true),
            course_theme_setting_read_mask: Some(wire::CourseThemeSettingReadMask {
                include_color: Some(true),
                include_color_gm3: Some(true),
                include_secure_photo_url: Some(true),
                include_secure_photo_url_gm3: Some(true),
                include_photo_render_mode: Some(true),
                include_use_gm3_stock_theme: Some(true),
            }),
        });
    }
    request
}

pub(super) fn decode_courses(response: wire::QueryCourseResponse) -> Snapshot {
    let mut courses = response.course.into_iter().filter_map(|mut wire_course| {
        let style = wire_course.course_setting.as_mut()
            .and_then(|settings| settings.course_theme_setting.take())
            .and_then(course_theme);
        let meet = wire_course.video_call_info.take().and_then(student_meet_url);
        let calendar = wire_course.calendar_url.as_deref().and_then(class_calendar::calendar_url);
        let owners = wire_course.owner_user_id.iter()
            .filter_map(|bytes| classwork::UserId::decode(bytes.as_slice()).ok()?.id).filter(|id| *id > 0).collect();
        let mut course = Course::from_wire(wire_course)?;
        course.theme = style;
        course.meet_url = meet;
        course.calendar_url = calendar;
        course.owner_ids = owners;
        Some(course)
    }).collect::<Vec<_>>();
    class_order::sort_visible_classes(&mut courses);
    Snapshot::live(courses)
}

fn course_theme(theme: wire::CourseThemeSetting) -> Option<CourseTheme> {
    let artwork_url = theme.secure_photo_url_gm3
        .filter(|url| artwork_host_is_allowed(url))
        .or_else(|| theme.secure_photo_url.filter(|url| artwork_host_is_allowed(url)));
    let color_rgb = theme.color_gm3.iter().chain(theme.color.iter()).find_map(|value| parse_color(value));
    if artwork_url.is_none() && color_rgb.is_none() {
        return None;
    }
    Some(CourseTheme { color_rgb, artwork_url })
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..], 16).ok()?,
    ])
}

pub(super) fn student_meet_url(info: wire::VideoCallInfo) -> Option<String> {
    // ENABLED_VISIBLE is the Android protocol value for student-visible links.
    (info.video_call_url_state == Some(3)).then_some(info.video_call_url).flatten()
        .filter(|url| https_host(url).as_deref() == Some("meet.google.com"))
}
