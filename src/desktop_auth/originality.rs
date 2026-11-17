use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

/// Link to the original assignment's Your work page only when reports are enabled.
pub fn originality_report_url(course_id: i64, item_id: i64, enabled: bool) -> Option<String> {
    if !enabled || course_id <= 0 || item_id <= 0 {
        return None;
    }
    // Android passes displayOnWeb=true to its link builder, selecting /dw.
    let course = encoded_id(course_id);
    let item = encoded_id(item_id);
    Some(format!("https://classroom.google.com/c/{course}/a/{item}/dw"))
}

fn encoded_id(id: i64) -> String {
    let mut id = id.to_string();
    // Android appends Z before encoding so the result never needs '=' padding.
    while id.len() % 3 != 0 {
        id.push('Z');
    }
    URL_SAFE_NO_PAD.encode(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{classroom_links::{ClassroomLink, ItemLinkKind}, desktop_auth::classwork};
    use prost::Message;

    #[test]
    fn enabled_assignment_uses_exact_classroom_web_handoff() {
        let url = originality_report_url(123, 456, true).unwrap();
        assert_eq!(url, "https://classroom.google.com/c/MTIz/a/NDU2/dw");
        assert_eq!(
            ClassroomLink::parse(&url),
            Some(ClassroomLink::Item { course: 123, item: 456, kind: ItemLinkKind::Assignment }),
        );
        assert_eq!(originality_report_url(123, 456, false), None);
        assert_eq!(originality_report_url(0, 456, true), None);
        assert_eq!(originality_report_url(123, -456, true), None);
        assert_eq!(
            originality_report_url(1234, 12, true).as_deref(),
            Some("https://classroom.google.com/c/MTIzNFpa/a/MTJa/dw"),
        );
    }

    #[test]
    fn assignment_settings_use_nested_tag_five_and_only_request_state() {
        let read_mask = classwork::AssignmentReadMask {
            originality_settings_mask: Some(classwork::OriginalitySettingsReadMask {
                include_state: Some(true),
            }),
            ..Default::default()
        };
        assert_eq!(read_mask.encode_to_vec(), [0x2a, 0x02, 0x08, 0x01]);
        for state in [0, 1, 2] {
            let bytes = [0x2a, 0x02, 0x08, state];
            let assignment = classwork::Assignment::decode(bytes.as_slice()).unwrap();
            assert_eq!(assignment.originality_settings.unwrap().state, Some(i32::from(state)));
        }
        assert!(classwork::Assignment::decode([].as_slice()).unwrap().originality_settings.is_none());
    }
}
