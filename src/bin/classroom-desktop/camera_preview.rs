use objc2::{
    ClassType, DeclaredClass, ProtocolType, class, declare_class, extern_protocol,
    msg_send, msg_send_id, mutability,
    encode::{Encoding, RefEncode},
    rc::Retained,
    runtime::{AnyObject, Bool, NSObjectProtocol},
};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSString};
use std::{ffi::{c_char, c_void}, ptr, slice, sync::{Arc, Mutex}};

const BGRA: u32 = u32::from_be_bytes(*b"BGRA");
const READ_ONLY: u64 = 1;

#[repr(C)]
struct SampleBuffer { _opaque: [u8; 0] }

unsafe impl RefEncode for SampleBuffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMSampleBuffer", &[]));
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(sample: *const SampleBuffer) -> *mut c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: &'static NSString;
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *const u8;
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetDataSize(buffer: *mut c_void) -> usize;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> *mut AnyObject;
    fn dispatch_sync_f(queue: *const AnyObject, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

pub(super) struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub(super) type Frames = Arc<Mutex<Option<Frame>>>;

extern_protocol!(
    unsafe trait AVCaptureVideoDataOutputSampleBufferDelegate: NSObjectProtocol {
        #[optional]
        #[method(captureOutput:didOutputSampleBuffer:fromConnection:)]
        unsafe fn frame(&self, output: &AnyObject, sample: *const SampleBuffer, connection: &AnyObject);
    }
    unsafe impl ProtocolType for dyn AVCaptureVideoDataOutputSampleBufferDelegate {}
);

declare_class!(
    struct PreviewDelegate;

    // The callback queue owns no UI objects. Its shared slot carries one frame.
    unsafe impl ClassType for PreviewDelegate {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "RustClassroomCameraPreviewDelegate";
    }

    impl DeclaredClass for PreviewDelegate {
        type Ivars = Frames;
    }

    unsafe impl NSObjectProtocol for PreviewDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for PreviewDelegate {
        #[method(captureOutput:didOutputSampleBuffer:fromConnection:)]
        unsafe fn frame(&self, _output: &AnyObject, sample: *const SampleBuffer, _connection: &AnyObject) {
            if let Ok(mut pending) = self.ivars().try_lock()
                && pending.is_none()
                && !sample.is_null()
            {
                *pending = unsafe { copy_frame(sample) };
            }
        }
    }
);

pub(super) struct PreviewOutput {
    output: Retained<AnyObject>,
    queue: Retained<AnyObject>,
    _delegate: Retained<PreviewDelegate>,
}

impl PreviewOutput {
    pub(super) fn install(session: &AnyObject, frames: Frames) -> Result<Self, String> {
        unsafe {
            let output: Retained<AnyObject> = msg_send_id![class!(AVCaptureVideoDataOutput), new];
            let allowed: Bool = msg_send![session, canAddOutput: &*output];
            if !allowed.as_bool() {
                return Err("macOS couldn't add a camera preview.".into());
            }
            let settings = NSDictionary::from_vec(&[kCVPixelBufferPixelFormatTypeKey],
                vec![NSNumber::numberWithUnsignedInt(BGRA)]);
            let delegate: Retained<PreviewDelegate> = msg_send_id![super(PreviewDelegate::alloc().set_ivars(frames)), init];
            let queue = Retained::from_raw(dispatch_queue_create(c"local.classroom.camera.preview".as_ptr(), ptr::null()))
                .ok_or("Couldn't open the camera preview queue.")?;
            let _: () = msg_send![&*output, setVideoSettings: &*settings];
            let _: () = msg_send![&*output, setAlwaysDiscardsLateVideoFrames: Bool::YES];
            let _: () = msg_send![&*output, setSampleBufferDelegate: &*delegate, queue: &*queue];
            let _: () = msg_send![session, addOutput: &*output];
            Ok(Self { output, queue, _delegate: delegate })
        }
    }
}

impl Drop for PreviewOutput {
    fn drop(&mut self) {
        extern "C" fn barrier(_: *mut c_void) {}
        unsafe {
            let _: () = msg_send![&*self.output, setSampleBufferDelegate: ptr::null::<AnyObject>(),
                queue: ptr::null::<AnyObject>()];
            dispatch_sync_f(Retained::as_ptr(&self.queue), ptr::null_mut(), barrier);
        }
    }
}

unsafe fn copy_frame(sample: *const SampleBuffer) -> Option<Frame> {
    unsafe { copy_pixel_buffer(CMSampleBufferGetImageBuffer(sample)) }
}

unsafe fn copy_pixel_buffer(buffer: *mut c_void) -> Option<Frame> {
    unsafe {
        if buffer.is_null() || CVPixelBufferGetPixelFormatType(buffer) != BGRA
            || CVPixelBufferLockBaseAddress(buffer, READ_ONLY) != 0
        {
            return None;
        }
        let frame = (|| {
            let width = CVPixelBufferGetWidth(buffer);
            let height = CVPixelBufferGetHeight(buffer);
            let stride = CVPixelBufferGetBytesPerRow(buffer);
            let base = CVPixelBufferGetBaseAddress(buffer);
            if base.is_null() || width > 4096 || height > 4096 || stride > 32768 {
                return None;
            }
            let size = stride.checked_mul(height)?;
            if size > CVPixelBufferGetDataSize(buffer) {
                return None;
            }
            convert_frame(slice::from_raw_parts(base, size), width, height, stride)
        })();
        let _ = CVPixelBufferUnlockBaseAddress(buffer, READ_ONLY);
        frame
    }
}

fn convert_frame(bytes: &[u8], width: usize, height: usize, stride: usize) -> Option<Frame> {
    if width == 0 || height == 0 || stride < width.checked_mul(4)? || bytes.len() < stride.checked_mul(height)? {
        return None;
    }
    let step = width.div_ceil(480).max(height.div_ceil(360));
    let (preview_width, preview_height) = (width.div_ceil(step), height.div_ceil(step));
    let mut pixels = Vec::with_capacity(preview_width * preview_height * 4);
    for row in bytes.chunks_exact(stride).take(height).step_by(step) {
        for chunk in row[..width * 4].chunks_exact(4).step_by(step) {
            pixels.extend([chunk[2], chunk[1], chunk[0], 255]);
        }
    }
    Some(Frame { width: preview_width as u32, height: preview_height as u32, pixels })
}

#[cfg(test)]
#[test]
fn preview_converts_padded_bgra_rows_and_rejects_truncated_data() {
    let row = [3, 2, 1, 0, 6, 5, 4, 0, 99, 99, 99, 99];
    let frame = convert_frame(&row, 2, 1, 12).unwrap();
    assert_eq!((frame.width, frame.height), (2, 1));
    assert_eq!(frame.pixels, [1, 2, 3, 255, 4, 5, 6, 255]);
    assert!(convert_frame(&row[..8], 2, 1, 12).is_none());
}

#[cfg(test)]
#[test]
fn reads_a_real_core_video_buffer_without_opening_the_camera() {
    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferCreate(allocator: *const c_void, width: usize, height: usize,
            format: u32, attributes: *const c_void, output: *mut *mut c_void) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }
    let mut buffer = ptr::null_mut();
    unsafe {
        assert_eq!(CVPixelBufferCreate(ptr::null(), 2, 2, BGRA, ptr::null(), &mut buffer), 0);
        assert!(!buffer.is_null());
        let frame = copy_pixel_buffer(buffer);
        CFRelease(buffer);
        let frame = frame.unwrap();
        assert_eq!((frame.width, frame.height, frame.pixels.len()), (2, 2, 16));
    }
}
