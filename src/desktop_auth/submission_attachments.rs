use super::{
    classwork, mutation, check_class, https_host, LiveSession, drive_files, PathBuf, BTreeSet,
    load_submissions, SubmissionAction, DriveFileAttachment, upload_drive_file, Message,
};

pub async fn remove_submission_attachment(
    session: &LiveSession, course_id: i64, item_id: i64, material_id: i64,
) -> Result<(), String> {
    check_class(course_id)?;
    if material_id <= 0 {
        return Err("Classroom did not provide an ID for this attachment.".into());
    }
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not provide your existing submission.")?;
    if SubmissionAction::for_submission(&mine) != Some(SubmissionAction::TurnIn)
        || !mine.attachments.iter().any(|item| item.remote_id == Some(material_id))
    {
        return Err("The attachment is no longer part of an editable submission.".into());
    }
    let target = classwork::SubmissionId {
        id: mine.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", mutation::WriteSubmissionRequest {
            request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
            submission_mutation: vec![mutation::SubmissionMutation {
                submission_id: Some(target.clone()),
                submission: Some(classwork::Submission {
                    submission_id: Some(target.clone()),
                    assignment_submission: Some(classwork::AssignmentSubmission {
                        material: vec![classwork::Material { id: Some(material_id), ..Default::default() }],
                    }),
                    ..Default::default()
                }),
                submission_update_mask: Some(mutation::SubmissionUpdateMask {
                    assignment_submission_mask: Some(mutation::AssignmentSubmissionUpdateMask {
                        update_material: Some(3),
                    }),
                    ..Default::default()
                }),
                add_materials: vec![],
            }],
            submission_read_mask: Some(classwork::SubmissionReadMask {
                assignment_submission_mask: Some(classwork::AssignmentSubmissionReadMask {
                    material_mask: Some(classwork::MaterialReadMask { include_name: Some(true), ..Default::default() }),
                }),
                ..Default::default()
            }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before trying again."))?;
    let confirmed = response.submission.into_iter()
        .find(|item| item.submission_id.as_ref() == Some(&target))
        .and_then(|item| item.assignment_submission)
        .is_some_and(|work| {
            let received: BTreeSet<_> = work.material.iter().filter_map(|item| item.id).collect();
            !received.contains(&material_id)
                && mine.attachments.iter().filter_map(|item| item.remote_id)
                    .filter(|id| *id != material_id).all(|id| received.contains(&id))
        });
    if confirmed { Ok(()) } else {
        Err("Classroom did not confirm removal. Refresh your work before trying again.".into())
    }
}

pub async fn attach_submission_link(
    session: &LiveSession, course_id: i64, item_id: i64, url: String,
) -> Result<(), String> {
    let url = url.trim();
    if https_host(url).is_none() {
        return Err("Enter a valid HTTPS link.".into());
    }
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    attach_material(session, course_id, target, link_material(url)).await
}

pub async fn attach_submission_drive_file(
    session: &LiveSession, course_id: i64, item_id: i64, file: DriveFileAttachment,
) -> Result<(), String> {
    let (target, mine) = editable_submission(session, course_id, item_id).await?;
    if mine.attachments.iter().any(|attachment| file.matches_attachment(attachment)) {
        return Ok(());
    }
    attach_material(session, course_id, target, file.0).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveDocumentKind {
    Docs,
    Sheets,
    Slides,
    Drawings,
}

impl DriveDocumentKind {
    pub const ALL: [Self; 4] = [Self::Docs, Self::Sheets, Self::Slides, Self::Drawings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Docs => "Docs",
            Self::Sheets => "Sheets",
            Self::Slides => "Slides",
            Self::Drawings => "Drawings",
        }
    }

    fn descriptor(self) -> (i32, &'static str, &'static str) {
        match self {
            Self::Docs => (1, "document", "application/vnd.google-apps.document"),
            Self::Sheets => (2, "spreadsheets", "application/vnd.google-apps.spreadsheet"),
            Self::Slides => (3, "presentation", "application/vnd.google-apps.presentation"),
            Self::Drawings => (6, "drawings", "application/vnd.google-apps.drawing"),
        }
    }
}

fn drive_document_request(course_id: i64, title: String, kind: DriveDocumentKind) -> classwork::DriveItemCreateRequest {
    classwork::DriveItemCreateRequest {
        course_id: Some(classwork::CourseId { id: Some(course_id) }),
        drive_item_type: Some(kind.descriptor().0),
        title: Some(title),
        material_read_mask: Some(classwork::MaterialReadMask {
            include_name: Some(true), include_mime_type: Some(true), include_open_url: Some(true),
            ..Default::default()
        }),
    }
}

fn created_drive_document(material: classwork::Material, kind: DriveDocumentKind) -> Option<classwork::Material> {
    let (_, editor, mime) = kind.descriptor();
    if material.reference_type.is_some_and(|value| value != 2)
        || material.mime_type.as_deref().is_some_and(|value| value != mime)
    {
        return None;
    }
    let link_id = material.open_url.as_deref().and_then(|url| {
        let host = https_host(url)?;
        let uri: tonic::codegen::http::Uri = url.parse().ok()?;
        let mut parts = uri.path().split('/').skip(1);
        match (host.as_str(), parts.next(), parts.next(), parts.next()) {
            ("docs.google.com", Some(category), Some("d"), Some(id))
                if category == editor && drive_files::valid_drive_id(id) => Some(id.to_owned()),
            ("drive.google.com", Some("file"), Some("d"), Some(id))
                if drive_files::valid_drive_id(id) => Some(id.to_owned()),
            _ => None,
        }
    });
    let id = match (material.reference, link_id) {
        (Some(reference), Some(link_id)) if reference != link_id => return None,
        (Some(reference), _) if drive_files::valid_drive_id(&reference) => reference,
        (None, Some(link_id)) => link_id,
        _ => return None,
    };
    Some(classwork::Material {
        name: material.name,
        open_url: Some(format!("https://docs.google.com/{editor}/d/{id}/edit")),
        reference: Some(id),
        reference_type: Some(2),
        mime_type: Some(mime.into()),
        ..Default::default()
    })
}

#[cfg(test)]
#[test]
fn create_drive_document_uses_the_recovered_request_tags_and_four_editor_types() {
    for (kind, tag) in DriveDocumentKind::ALL.into_iter().zip([1, 2, 3, 6]) {
        let request = drive_document_request(9, "Essay".into(), kind);
        assert_eq!(request.encode_to_vec(), [
            0x0a, 2, 0x08, 9, 0x10, tag, 0x22, 5, b'E', b's', b's', b'a', b'y',
            0x2a, 6, 0x10, 1, 0x20, 1, 0x30, 1,
        ]);
    }
}

#[cfg(test)]
#[test]
fn created_editor_material_binds_a_valid_drive_id_to_the_existing_submission_mutation() {
    let editor = "https://docs.google.com/document/d/fixture-doc/edit";
    let raw = classwork::Material { open_url: Some(editor.into()), ..Default::default() };
    let material = created_drive_document(raw.clone(), DriveDocumentKind::Docs).unwrap();
    let id = classwork::SubmissionId {
        id: Some(7),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(5), course_id: Some(classwork::CourseId { id: Some(9) }),
        }),
    };
    let request = attach_material_request(id.clone(), &material);
    let [entry] = request.submission_mutation.as_slice() else { panic!("Expected one attachment mutation") };
    assert_eq!(entry.submission_id.as_ref(), Some(&id));
    assert!(entry.submission.is_none() && entry.submission_update_mask.is_none());
    let decoded = classwork::Material::decode(entry.add_materials[0].as_slice()).unwrap();
    assert_eq!(decoded.reference.as_deref(), Some("fixture-doc"));
    assert_eq!(decoded.reference_type, Some(2));
    assert_eq!(decoded.open_url.as_deref(), Some(editor));
    assert_eq!(decoded.is_new_upload, None);
    assert!(created_drive_document(raw.clone(), DriveDocumentKind::Slides).is_none());
    assert!(created_drive_document(classwork::Material { reference: Some("wrong".into()), ..raw.clone() }, DriveDocumentKind::Docs).is_none());
    assert!(created_drive_document(classwork::Material {
        open_url: Some("https://docs.google.com.example.invalid/document/d/fixture-doc/edit".into()), ..raw
    }, DriveDocumentKind::Docs).is_none());
}

pub async fn create_submission_document(
    session: &LiveSession, course_id: i64, item_id: i64, title: String, kind: DriveDocumentKind,
) -> Result<String, String> {
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    let title = title.trim().chars().take(200).collect::<String>();
    let response: classwork::DriveItemCreateResponse = session.write_rpc(
        "DriveItemCreate", drive_document_request(course_id, title, kind), Some(course_id),
    ).await.map_err(|error| format!("{error} Check Drive for the new file before trying again."))?;
    let material = response.material.and_then(|material| created_drive_document(material, kind))
        .ok_or("Classroom did not return a usable Drive file. Check Drive before trying again.")?;
    let url = material.open_url.clone().ok_or("The new Drive file did not include an editor link.")?;

    // Recheck ownership and editability between creation and attachment.
    let attachment = async {
        let (current, _) = editable_submission(session, course_id, item_id).await?;
        if current != target {
            return Err("Your submission changed.".to_owned());
        }
        attach_material(session, course_id, target, material).await
    }.await;
    attachment.map_err(|error| format!("The new file is in Google Drive. Classroom could not confirm attachment. {error}"))?;
    Ok(url)
}

pub(super) fn link_material(url: &str) -> classwork::Material {
    classwork::Material {
        reference: Some(url.into()), open_url: Some(url.into()), reference_type: Some(4),
        ..Default::default()
    }
}

async fn editable_submission(
    session: &LiveSession, course_id: i64, item_id: i64,
) -> Result<(classwork::SubmissionId, crate::desktop_data::Submission), String> {
    check_class(course_id)?;
    if item_id <= 0 {
        return Err("Choose an assignment first.".into());
    }
    let key = item_id.to_string();
    let (_, mine) = load_submissions(session, course_id).await?
        .into_iter().find(|(id, _)| id == &key)
        .ok_or("Classroom did not provide an existing submission.")?;
    if SubmissionAction::for_submission(&mine) != Some(SubmissionAction::TurnIn) {
        return Err("Unsubmit your work before adding attachments.".into());
    }
    Ok((classwork::SubmissionId {
        id: mine.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    }, mine))
}

pub(super) fn attach_material_request(target: classwork::SubmissionId, material: &classwork::Material) -> mutation::WriteSubmissionRequest {
    mutation::WriteSubmissionRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        submission_mutation: vec![mutation::SubmissionMutation {
            submission_id: Some(target),
            submission: None,
            submission_update_mask: None,
            add_materials: vec![material.encode_to_vec()],
        }],
        submission_read_mask: Some(classwork::SubmissionReadMask {
            assignment_submission_mask: Some(classwork::AssignmentSubmissionReadMask {
                material_mask: Some(classwork::MaterialReadMask {
                    include_open_url: Some(true), ..Default::default()
                }),
            }),
            ..Default::default()
        }),
    }
}

async fn attach_material(
    session: &LiveSession, course_id: i64, target: classwork::SubmissionId,
    material: classwork::Material,
) -> Result<(), String> {
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", attach_material_request(target.clone(), &material), Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before adding the attachment again."))?;
    if response.submission.into_iter().any(|item| item.submission_id.as_ref() == Some(&target)
        && item.assignment_submission.is_some_and(|work| work.material.iter().any(|existing| {
            material_matches(&material, existing)
        }))) {
        Ok(())
    } else {
        Err("Classroom did not confirm the attachment. Refresh your work before trying again.".into())
    }
}

pub(super) fn material_matches(sent: &classwork::Material, received: &classwork::Material) -> bool {
    if sent.reference.is_some() && sent.reference == received.reference
        || sent.open_url.is_some() && sent.open_url == received.open_url {
        return true;
    }
    if sent.reference_type != Some(2) {
        return false;
    }
    let (Some(id), Some(url)) = (sent.reference.as_deref(), received.open_url.as_deref()) else {
        return false;
    };
    let Ok(uri) = url.parse::<tonic::codegen::http::Uri>() else { return false };
    if uri.scheme_str() != Some("https") {
        return false;
    }
    if uri.host() == Some("docs.google.com") {
        let mut parts = uri.path().trim_start_matches('/').split('/');
        return matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some("document" | "spreadsheets" | "presentation" | "forms" | "drawings"), Some("d"), Some(found))
                if found == id
        );
    }
    if uri.host() != Some("drive.google.com") {
        return false;
    }
    let mut parts = uri.path().trim_start_matches('/').split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some("file"), Some("d"), Some(found)) if found == id)
        || (uri.path() == "/open" && uri.query().is_some_and(|query| {
            form_urlencoded::parse(query.as_bytes()).any(|(key, value)| key == "id" && value == id)
        }))
}

pub async fn attach_submission_file(
    session: &LiveSession, course_id: i64, item_id: i64, path: PathBuf,
) -> Result<(), String> {
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    let file = upload_drive_file(session, path).await?;
    attach_material(session, course_id, target, file.0).await.map_err(|error| {
        format!("Your file was uploaded to Google Drive, but attachment is unconfirmed. {error}")
    })
}
