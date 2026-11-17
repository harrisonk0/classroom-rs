use objc2::{class, msg_send, msg_send_id, rc::{Allocated, Retained, autoreleasepool}, runtime::{AnyObject, Bool}};
use objc2_foundation::{NSArray, NSString, NSURL};
use std::{path::{Path, PathBuf}, process::Stdio, ptr, sync::Arc};
use tempfile::TempDir;
use tokio::process::Command;

const NAME: &str = "Classroom screen recording.mov";

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: &'static NSString;
}

#[derive(Clone)]
pub struct Draft {
    directory: Arc<TempDir>,
    path: PathBuf,
}

impl Draft {
    pub(super) fn validate(directory: Arc<TempDir>, path: PathBuf) -> Result<Self, String> {
        if playable_video(&path) {
            Ok(Self { directory, path })
        } else {
            Err("The saved recording has no playable video.".into())
        }
    }

    pub fn file(&self) -> (Arc<TempDir>, PathBuf) {
        (self.directory.clone(), self.path.clone())
    }

    pub async fn preview(self) -> Result<(), String> {
        let opened = Command::new("/usr/bin/open").args(["-b", "com.apple.QuickTimePlayerX"])
            .arg(&self.path).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
        if opened.is_ok_and(|status| status.success()) {
            Ok(())
        } else {
            Err("QuickTime Player couldn't open the local recording.".into())
        }
    }
}

fn capture_command(path: &Path) -> Command {
    // Interactive macOS selection and the system Stop button stay in control.
    // No microphone or system-audio flags are passed.
    let mut command = Command::new("/usr/sbin/screencapture");
    command.args(["-i", "-U", "-v", "-J", "video", "-V", "300", "-x"]).arg(path)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    command
}

pub async fn capture() -> Result<Option<Draft>, String> {
    let directory = Arc::new(tempfile::Builder::new().prefix("classroom-screen-").tempdir()
        .map_err(|_| "Couldn't make a temporary folder for the recording.")?);
    let path = directory.path().join(NAME);
    let status = capture_command(&path).status().await.map_err(|_| "macOS couldn't open screen recording.")?;
    if !status.success() {
        return Err("No screen recording was saved. If macOS blocked it, check Screen Recording in Privacy & Security.".into());
    }
    if !path.is_file() {
        return Ok(None);
    }

    tokio::task::spawn_blocking(move || Draft::validate(directory, path).map(Some))
        .await.map_err(|_| "Couldn't verify the saved recording.")?
}

fn playable_video(path: &Path) -> bool {
    let Some(path) = path.to_str() else { return false };
    autoreleasepool(|_| unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let allocated: Allocated<AnyObject> = msg_send_id![class!(AVURLAsset), alloc];
        let asset: Option<Retained<AnyObject>> =
            msg_send_id![allocated, initWithURL: &*url, options: ptr::null::<AnyObject>()];
        let Some(asset) = asset else { return false };
        let playable: Bool = msg_send![&*asset, isPlayable];
        if !playable.as_bool() {
            return false;
        }
        let tracks: Option<Retained<NSArray<AnyObject>>> = msg_send_id![&*asset, tracksWithMediaType: AVMediaTypeVideo];
        tracks.is_some_and(|tracks| !tracks.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_recording_uses_explicit_selection_output_and_five_minute_limit() {
        let destination = Path::new("/tmp/local fixture.mov");
        let command = capture_command(destination);
        assert_eq!(command.as_std().get_program(), "/usr/sbin/screencapture");
        assert_eq!(command.as_std().get_args().collect::<Vec<_>>(),
            ["-i", "-U", "-v", "-J", "video", "-V", "300", "-x", "/tmp/local fixture.mov"]);
    }

    #[test]
    fn native_validation_rejects_an_unplayable_movie() {
        let fixture = tempfile::NamedTempFile::new().unwrap();
        assert!(!playable_video(fixture.path()));
        if let Some(valid) = std::env::var_os("CLASSROOM_VIDEO_FIXTURE") {
            assert!(playable_video(Path::new(&valid)));
        }
    }
}
