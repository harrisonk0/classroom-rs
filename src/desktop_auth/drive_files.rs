use super::{DriveFileAttachment, LiveSession, run_drive_helper};
use serde::Deserialize;
use std::{collections::BTreeSet, time::Duration};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct DriveFilePage {
    pub files: Vec<DriveFileAttachment>,
    pub folders: Vec<DriveFolder>,
    pub next_page: Option<String>,
    pub incomplete: bool,
}

#[derive(Clone)]
pub struct DriveFolder {
    pub id: String,
    pub name: String,
}

#[derive(Clone)]
pub enum DriveLocation {
    Recent,
    Shared,
    Folder(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveListReply {
    #[serde(default)]
    files: Vec<DriveListEntry>,
    next_page_token: Option<String>,
    #[serde(default)]
    incomplete_search: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveListEntry {
    id: String,
    name: String,
    mime_type: String,
}

fn decode_page(reply: DriveListReply) -> DriveFilePage {
    let mut seen = BTreeSet::new();
    let mut folders = Vec::new();
    let files = reply.files.into_iter().filter_map(|entry| {
        if entry.mime_type == "application/vnd.google-apps.shortcut" || entry.name.trim().is_empty()
            || !valid_drive_id(&entry.id) || !seen.insert(entry.id.clone())
        {
            return None;
        }
        if entry.mime_type == "application/vnd.google-apps.folder" {
            folders.push(DriveFolder { id: entry.id, name: entry.name });
            return None;
        }
        DriveFileAttachment::existing(entry.id, entry.name, entry.mime_type)
    }).collect();
    DriveFilePage {
        files,
        folders,
        next_page: reply.next_page_token.filter(|token| !token.is_empty() && token.len() <= 8192),
        incomplete: reply.incomplete_search,
    }
}

pub(super) fn valid_drive_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256
        && id.bytes().all(|value| value.is_ascii_alphanumeric() || value == b'_' || value == b'-')
}

pub async fn list_drive_files(
    session: &LiveSession, location: DriveLocation, search: String, page_token: Option<String>,
) -> Result<DriveFilePage, String> {
    if search.len() > 1024 || page_token.as_ref().is_some_and(|token| token.len() > 8192) {
        return Err("The Drive search request is too long.".into());
    }
    let parent = match &location {
        DriveLocation::Folder(id) if valid_drive_id(id) => Some(id),
        DriveLocation::Folder(_) => return Err("Invalid Drive folder ID.".into()),
        _ => None,
    };
    let bearer = session.bearer_for_request(None).await?;
    let payload = Zeroizing::new(serde_json::to_vec(&serde_json::json!({
        "search": search.trim(),
        "page_token": page_token,
        "parent": parent,
        "shared_only": matches!(location, DriveLocation::Shared),
        "bearer": bearer.as_str(),
    })).map_err(|_| "Could not prepare the Drive search.")?);
    run_drive_helper(&["--list"], payload, Duration::from_secs(35), "Drive did not finish listing files.")
        .await.map(decode_page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_auth::{classwork, material_matches};

    #[test]
    fn list_metadata_preserves_page_and_marks_existing_items_as_not_uploaded() {
        let reply = serde_json::from_str(r#"{
            "nextPageToken":"cursor-fixture","incompleteSearch":true,"files":[
                {"id":"valid-file","name":"Course notes","mimeType":"application/vnd.google-apps.document"},
                {"id":"folder","name":"A folder","mimeType":"application/vnd.google-apps.folder"},
                {"id":"bad id","name":"Invalid","mimeType":"application/pdf"},
                {"id":"valid-file","name":"Duplicate","mimeType":"application/pdf"}
            ]
        }"#).unwrap();
        let page = decode_page(reply);
        assert_eq!(page.next_page.as_deref(), Some("cursor-fixture"));
        assert!(page.incomplete);
        assert_eq!(page.folders.iter().map(|folder| folder.name.as_str()).collect::<Vec<_>>(), ["A folder"]);
        let [file] = page.files.as_slice() else { panic!("Expected one attachable file") };
        assert_eq!(file.name(), "Course notes");
        assert_eq!(file.0.reference_type, Some(2));
        assert_eq!(file.0.is_new_upload, None);
        let received = classwork::Material {
            open_url: Some("https://docs.google.com/document/d/valid-file/edit".into()),
            ..Default::default()
        };
        assert!(material_matches(&file.0, &received));
    }

    #[test]
    fn existing_file_is_added_to_one_submission_without_an_upload_marker() {
        use crate::desktop_auth::{attach_material_request, mutation};
        use prost::Message;
        let file = DriveFileAttachment::existing("fixture-file".into(), "Notes".into(), "application/pdf".into()).unwrap();
        let key = classwork::SubmissionId {
            id: Some(9),
            stream_item_id: Some(classwork::StreamItemId {
                id: Some(17),
                course_id: Some(classwork::CourseId { id: Some(21) }),
            }),
        };
        let request = attach_material_request(key.clone(), &file.0);
        let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let [target] = decoded.submission_mutation.as_slice() else { panic!("Expected exactly one target") };
        assert_eq!(target.submission_id, Some(key));
        assert!(target.submission_update_mask.is_none());
        let material = classwork::Material::decode(target.add_materials[0].as_slice()).unwrap();
        assert_eq!(material.reference.as_deref(), Some("fixture-file"));
        assert_eq!(material.is_new_upload, None);
    }
}
