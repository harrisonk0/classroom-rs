use block2::RcBlock;
use objc2::{class, msg_send, msg_send_id, rc::{Allocated, Retained}, runtime::{AnyObject, Bool}};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::{path::{Path, PathBuf}, ptr, sync::{Arc, Mutex}, time::Duration};
use tempfile::TempDir;
use tokio::sync::oneshot;

pub const MAX_DURATION: Duration = Duration::from_secs(300);
const AUDIO_NAME: &str = "Classroom audio recording.m4a";

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: &'static NSString;
}

#[link(name = "AVFAudio", kind = "framework")]
unsafe extern "C" {
    static AVFormatIDKey: &'static NSString;
    static AVSampleRateKey: &'static NSString;
    static AVNumberOfChannelsKey: &'static NSString;
}

pub fn request_permission() -> oneshot::Receiver<bool> {
    request_media_permission(unsafe { AVMediaTypeAudio })
}

pub(super) fn request_media_permission(media_type: &NSString) -> oneshot::Receiver<bool> {
    let (sender, receiver) = oneshot::channel();
    // AVCaptureDevice copies the block and may call it on a background thread.
    let sender = Mutex::new(Some(sender));
    let completion = RcBlock::new(move |granted: Bool| {
        if let Ok(mut sender) = sender.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(granted.as_bool());
        }
    });
    unsafe {
        let _: () = msg_send![class!(AVCaptureDevice),
            requestAccessForMediaType: media_type, completionHandler: &*completion];
    }
    receiver
}

pub struct Recording {
    recorder: Option<Retained<AnyObject>>,
    directory: Arc<TempDir>,
}

impl Recording {
    pub fn start() -> Result<Self, String> {
        let directory = Arc::new(tempfile::Builder::new().prefix("classroom-audio-").tempdir()
            .map_err(|_| "Could not create a temporary recording folder.")?);
        let url = file_url(&directory.path().join(AUDIO_NAME))?;
        let settings = unsafe { NSDictionary::from_vec(
            &[AVFormatIDKey, AVSampleRateKey, AVNumberOfChannelsKey],
            vec![NSNumber::numberWithUnsignedInt(u32::from_be_bytes(*b"aac ")),
                NSNumber::numberWithDouble(44_100.0), NSNumber::numberWithUnsignedInt(1)],
        ) };
        // The allocated receiver is consumed by init. macOS owns the optional NSError.
        let recorder: Retained<AnyObject> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send_id![class!(AVAudioRecorder), alloc];
            let initialized: Option<Retained<AnyObject>> = msg_send_id![allocated,
                initWithURL: &*url, settings: &*settings, error: ptr::null_mut::<*mut AnyObject>()];
            initialized.ok_or("macOS couldn't open the microphone recorder.")?
        };
        let started: Bool = unsafe { msg_send![&*recorder, recordForDuration: MAX_DURATION.as_secs_f64()] };
        if !started.as_bool() {
            return Err("Couldn't start recording. Check that your microphone is available.".into());
        }
        Ok(Self { recorder: Some(recorder), directory })
    }

    pub fn finish(mut self) -> Result<Draft, String> {
        self.stop();
        Draft::load(self.directory.clone(), self.directory.path().join(AUDIO_NAME))
    }

    fn stop(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            unsafe { let _: () = msg_send![&*recorder, stop]; }
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct Draft {
    player: Retained<AnyObject>,
    directory: Arc<TempDir>,
    path: PathBuf,
    pub duration: Duration,
}

impl Draft {
    fn load(directory: Arc<TempDir>, path: PathBuf) -> Result<Self, String> {
        let url = file_url(&path)?;
        let player: Retained<AnyObject> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send_id![class!(AVAudioPlayer), alloc];
            let initialized: Option<Retained<AnyObject>> = msg_send_id![allocated,
                initWithContentsOfURL: &*url, error: ptr::null_mut::<*mut AnyObject>()];
            initialized.ok_or("Couldn't open the recording. Try recording again.")?
        };
        let seconds: f64 = unsafe { msg_send![&*player, duration] };
        let duration = Duration::try_from_secs_f64(seconds).map_err(|_| "Invalid recording duration.")?;
        if duration < Duration::from_millis(80) {
            return Err("The recording is too short. Try recording again.".into());
        }
        Ok(Self { player, directory, path, duration })
    }

    pub fn play_preview(&self) -> Result<(), String> {
        unsafe {
            let _: () = msg_send![&*self.player, setCurrentTime: 0.0_f64];
            let started: Bool = msg_send![&*self.player, play];
            if !started.as_bool() {
                return Err("macOS couldn't play the recording.".into());
            }
        }
        Ok(())
    }

    pub fn is_playing(&self) -> bool {
        let playing: Bool = unsafe { msg_send![&*self.player, isPlaying] };
        playing.as_bool()
    }

    pub fn stop_preview(&self) {
        unsafe { let _: () = msg_send![&*self.player, stop]; }
    }

    pub fn file(&self) -> (Arc<TempDir>, PathBuf) {
        (self.directory.clone(), self.path.clone())
    }
}

impl Drop for Draft {
    fn drop(&mut self) {
        self.stop_preview();
    }
}

pub(super) fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
    let path = path.to_str().ok_or("The recording path is not valid UTF-8.")?;
    Ok(unsafe { NSURL::fileURLWithPath(&NSString::from_str(path)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_preview_loads_a_local_fixture_without_microphone_access() {
        let temporary = Arc::new(tempfile::tempdir().unwrap());
        let path = temporary.path().join("fixture.wav");
        let mut wav = Vec::new();
        let size = 16_000_u32;
        wav.extend(b"RIFF");
        wav.extend((36 + size).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(8_000_u32.to_le_bytes());
        wav.extend(16_000_u32.to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(size.to_le_bytes());
        wav.resize(44 + size as usize, 0);
        std::fs::write(&path, wav).unwrap();
        let draft = Draft::load(temporary, path.clone()).unwrap();
        assert_eq!(draft.duration, Duration::from_secs(1));
        draft.play_preview().unwrap();
        draft.stop_preview();
        let (keep_alive, _) = draft.file();
        drop(draft);
        assert!(path.is_file());
        drop(keep_alive);
        assert!(!path.exists());
    }
}
