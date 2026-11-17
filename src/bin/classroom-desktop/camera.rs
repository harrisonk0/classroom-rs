use super::{audio, screen_recording::Draft};
use objc2::{ClassType, DeclaredClass, ProtocolType, declare_class, extern_protocol, msg_send, msg_send_id, class,
    mutability, rc::{autoreleasepool, Retained}, runtime::{AnyObject, Bool, NSObjectProtocol}};
use objc2_foundation::{NSObject, NSString};
use std::{ptr, sync::{Arc, Mutex, mpsc}, thread, time::{Duration, Instant}};
use tokio::sync::oneshot;

#[path = "camera_preview.rs"]
mod camera_preview;
use camera_preview::{Frames, PreviewOutput};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: &'static NSString;
    static AVMediaTypeAudio: &'static NSString;
    static AVCaptureSessionPresetMedium: &'static NSString;
}

enum Event {
    Begin,
    Stop,
    Cancel,
    FileFinished,
}

extern_protocol!(
    unsafe trait AVCaptureFileOutputRecordingDelegate: NSObjectProtocol {
        #[method(captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:)]
        fn finished(&self, output: &AnyObject, url: &AnyObject, connections: &AnyObject, error: Option<&AnyObject>);
    }

    unsafe impl ProtocolType for dyn AVCaptureFileOutputRecordingDelegate {}
);

declare_class!(
    struct MovieDelegate;

    // AVFoundation may invoke the delegate on a background thread. Its single
    // ivar is a thread-safe channel sender and doesn't access the capture UI.
    unsafe impl ClassType for MovieDelegate {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "RustClassroomMovieDelegate";
    }

    impl DeclaredClass for MovieDelegate {
        type Ivars = mpsc::Sender<Event>;
    }

    unsafe impl NSObjectProtocol for MovieDelegate {}

    unsafe impl AVCaptureFileOutputRecordingDelegate for MovieDelegate {
        #[method(captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:)]
        fn finished(&self, _output: &AnyObject, _url: &AnyObject, _connections: &AnyObject, _error: Option<&AnyObject>) {
            let _ = self.ivars().send(Event::FileFinished);
        }
    }
);

pub fn request_permission() -> oneshot::Receiver<bool> {
    audio::request_media_permission(unsafe { AVMediaTypeVideo })
}

pub struct Control {
    events: mpsc::Sender<Event>,
    frames: Frames,
}

impl Control {
    pub(super) fn screenshot_fixture() -> Self {
        let (events, _) = mpsc::channel();
        Self { events, frames: Arc::new(Mutex::new(None)) }
    }

    pub fn begin(&self) {
        let _ = self.events.send(Event::Begin);
    }

    pub fn stop(&self) {
        let _ = self.events.send(Event::Stop);
    }

    pub fn take_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.frames.try_lock().ok()?.take()?;
        Some((frame.width, frame.height, frame.pixels))
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Cancel);
    }
}

type RecordingResult = Result<Option<Draft>, String>;

pub fn start() -> Result<(Control, oneshot::Receiver<RecordingResult>), String> {
    let (events, incoming) = mpsc::channel();
    let (completed, result) = oneshot::channel();
    let delegate_sender = events.clone();
    let frames: Frames = Arc::new(Mutex::new(None));
    let worker_frames = frames.clone();
    thread::Builder::new().name("classroom-camera".into()).spawn(move || {
        // AVFoundation session startup can block. All configuration and
        // start/stop calls stay on this worker, away from the Iced event loop.
        let result = autoreleasepool(|_| record(delegate_sender, incoming, worker_frames));
        let _ = completed.send(result);
    }).map_err(|_| "Couldn't start the camera worker.")?;
    Ok((Control { events, frames }, result))
}

fn record(events: mpsc::Sender<Event>, incoming: mpsc::Receiver<Event>, frames: Frames) -> Result<Option<Draft>, String> {
    let directory = Arc::new(tempfile::Builder::new().prefix("classroom-camera-").tempdir()
        .map_err(|_| "Couldn't make a temporary folder for the video.")?);
    let path = directory.path().join("Classroom camera recording.mov");
    let url = audio::file_url(&path)?;
    if incoming.try_recv().is_ok() {
        return Ok(None);
    }
    unsafe {
        let session: Retained<AnyObject> = msg_send_id![class!(AVCaptureSession), new];
        let output: Retained<AnyObject> = msg_send_id![class!(AVCaptureMovieFileOutput), new];
        let _: () = msg_send![&*session, beginConfiguration];
        let medium: Bool = msg_send![&*session, canSetSessionPreset: AVCaptureSessionPresetMedium];
        if medium.as_bool() {
            let _: () = msg_send![&*session, setSessionPreset: AVCaptureSessionPresetMedium];
        }
        let setup = add_input(&session, AVMediaTypeVideo, "camera").and_then(|()| {
            let preview = PreviewOutput::install(&session, frames)?;
                let supported: Bool = msg_send![&*session, canAddOutput: &*output];
                if !supported.as_bool() {
                    return Err("macOS could not add the movie recorder.".into());
                }
                let _: () = msg_send![&*session, addOutput: &*output];
                Ok(preview)
            });
        let _: () = msg_send![&*session, commitConfiguration];
        let _preview = setup?;
        if incoming.try_recv().is_ok() {
            return Ok(None);
        }
        let delegate: Retained<MovieDelegate> = msg_send_id![super(MovieDelegate::alloc().set_ivars(events)), init];
        let _: () = msg_send![&*session, startRunning];
        let running: Bool = msg_send![&*session, isRunning];
        if !running.as_bool() {
            return Err("macOS could not start the camera.".into());
        }
        if !matches!(incoming.recv(), Ok(Event::Begin)) {
            let _: () = msg_send![&*session, stopRunning];
            return Ok(None);
        }
        let _: () = msg_send![&*session, beginConfiguration];
        let microphone = add_input(&session, AVMediaTypeAudio, "microphone");
        let _: () = msg_send![&*session, commitConfiguration];
        if let Err(error) = microphone {
            let _: () = msg_send![&*session, stopRunning];
            return Err(error);
        }
        if incoming.try_recv().is_ok() {
            let _: () = msg_send![&*session, stopRunning];
            return Ok(None);
        }
        let _: () = msg_send![&*output, startRecordingToOutputFileURL: &*url, recordingDelegate: &*delegate];
        let finished = match incoming.recv_timeout(audio::MAX_DURATION) {
            Ok(Event::FileFinished) => Ok(true),
            Ok(Event::Cancel) => {
                let _: () = msg_send![&*output, stopRecording];
                let _ = wait_for_movie(&incoming);
                Ok(false)
            }
            Ok(Event::Stop | Event::Begin) | Err(_) => {
                let _: () = msg_send![&*output, stopRecording];
                wait_for_movie(&incoming)
            }
        };
        let _: () = msg_send![&*session, stopRunning];
        if finished? {
            Draft::validate(directory, path).map(Some)
        } else {
            Ok(None)
        }
    }
}

fn wait_for_movie(incoming: &mpsc::Receiver<Event>) -> Result<bool, String> {
    let until = Instant::now() + Duration::from_secs(10);
    let mut cancelled = false;
    loop {
        match incoming.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(Event::FileFinished) => return Ok(!cancelled),
            Ok(Event::Cancel) => cancelled = true,
            Ok(Event::Stop | Event::Begin) => {}
            Err(_) => return Err("macOS did not finish saving the camera recording.".into()),
        }
    }
}

fn add_input(session: &AnyObject, media_type: &NSString, label: &str) -> Result<(), String> {
    unsafe {
        let device: Option<Retained<AnyObject>> = msg_send_id![class!(AVCaptureDevice), defaultDeviceWithMediaType: media_type];
        let device = device.ok_or_else(|| format!("No {label} was found."))?;
        let input: Option<Retained<AnyObject>> = msg_send_id![class!(AVCaptureDeviceInput),
            deviceInputWithDevice: &*device, error: ptr::null_mut::<*mut AnyObject>()];
        let input = input.ok_or_else(|| format!("macOS couldn't open the {label}."))?;
        let supported: Bool = msg_send![session, canAddInput: &*input];
        if !supported.as_bool() {
            return Err(format!("Couldn't use the {label} for this recording."));
        }
        let _: () = msg_send![session, addInput: &*input];
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_movie_waits_for_delegate_completion_and_discard_wins() {
        let (sender, incoming) = mpsc::channel();
        sender.send(Event::Stop).unwrap();
        sender.send(Event::FileFinished).unwrap();
        assert_eq!(wait_for_movie(&incoming), Ok(true));
        sender.send(Event::Cancel).unwrap();
        sender.send(Event::FileFinished).unwrap();
        assert_eq!(wait_for_movie(&incoming), Ok(false));
    }

    #[test]
    fn objc_movie_delegate_delivers_completion_with_no_camera() {
        let (sender, incoming) = mpsc::channel();
        let object = NSObject::new();
        let object = object.as_ref();
        let delegate: Retained<MovieDelegate> =
            unsafe { msg_send_id![super(MovieDelegate::alloc().set_ivars(sender)), init] };
        AVCaptureFileOutputRecordingDelegate::finished(&*delegate, object, object, object, None);
        assert!(matches!(incoming.try_recv(), Ok(Event::FileFinished)));
    }
}
