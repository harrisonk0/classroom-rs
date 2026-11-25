use super::{
    classwork, Runtime, LiveSession, drive_files, Duration, timeout, Stdio, Zeroizing, Command,
    PathBuf, Attachment, Deserialize, material_matches, AsyncWriteExt,
};

#[derive(Deserialize)]
struct DriveUploadReply {
    id: Option<String>,
    name: Option<String>,
    mime_type: Option<String>,
}

fn drive_helper_path() -> Result<(PathBuf, PathBuf), String> {
    let runtime = Runtime::resolve()?;
    let script = runtime.sign_in.parent().ok_or("Drive upload runtime is missing.")?
        .join("drive_upload.py");
    if !script.is_file() {
        return Err("Drive upload runtime is missing.".into());
    }
    Ok((runtime.python, script))
}

#[derive(Clone)]
pub struct DriveFileAttachment(pub(super) classwork::Material);

impl DriveFileAttachment {
    pub fn name(&self) -> &str {
        self.0.name.as_deref().unwrap_or("Drive file")
    }

    pub fn id(&self) -> &str {
        self.0.reference.as_deref().unwrap_or_default()
    }

    pub fn matches_attachment(&self, attachment: &Attachment) -> bool {
        attachment.url.as_ref().is_some_and(|url| material_matches(&self.0, &classwork::Material {
            open_url: Some(url.clone()),
            ..Default::default()
        }))
    }

    pub fn existing(id: String, name: String, mime: String) -> Option<Self> {
        Self::from_drive(id, Some(name), Some(mime), false)
    }

    fn from_drive(id: String, name: Option<String>, mime: Option<String>, uploaded: bool) -> Option<Self> {
        if !drive_files::valid_drive_id(&id) {
            return None;
        }
        let open_url = format!("https://drive.google.com/file/d/{id}/view");
        Some(Self(classwork::Material {
            name,
            reference: Some(id),
            reference_type: Some(2),
            mime_type: mime,
            open_url: Some(open_url),
            is_new_upload: uploaded.then_some(true),
            ..Default::default()
        }))
    }
}

pub(super) async fn upload_drive_file(session: &LiveSession, path: PathBuf) -> Result<DriveFileAttachment, String> {
    let bearer = session.bearer_for_request(None).await?;
    let payload = Zeroizing::new(serde_json::to_vec(&serde_json::json!({
        "path": path.to_str().ok_or("This file path is not valid UTF-8.")?,
        "bearer": bearer.as_str(),
    })).map_err(|_| "Could not prepare the upload.")?);
    let reply: DriveUploadReply = run_drive_helper(&[], payload, Duration::from_secs(600),
        "Upload did not finish. Check Google Drive before trying again.").await?;
    reply.id.and_then(|id| DriveFileAttachment::from_drive(id, reply.name, reply.mime_type, true))
        .ok_or_else(|| "Drive did not confirm the file ID.".into())
}

pub(super) async fn run_drive_helper<R: serde::de::DeserializeOwned>(
    args: &[&str], payload: Zeroizing<Vec<u8>>, deadline: Duration, failure: &'static str,
) -> Result<R, String> {
    let (python, script) = drive_helper_path()?;
    let mut child = Command::new(python).args(["-I", "-B"]).arg(script).args(args)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true).spawn().map_err(|_| "Could not open the Drive helper.")?;
    let mut input = child.stdin.take().ok_or("Drive helper pipe unavailable.")?;
    input.write_all(payload.as_slice()).await.map_err(|_| "Could not send Drive request details.")?;
    drop(input);
    drop(payload);
    let output = timeout(deadline, child.wait_with_output()).await
        .map_err(|_| failure)?.map_err(|_| failure)?;
    if !output.status.success() {
        return Err(serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
            .and_then(|reply| reply.get("error")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| failure.into()));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "Drive returned an unreadable result.".into())
}
