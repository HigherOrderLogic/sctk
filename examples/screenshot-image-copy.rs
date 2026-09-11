use image::{save_buffer, ColorType};
use smithay_client_toolkit::{
    delegate_dispatch2, delegate_registry,
    globals::GlobalData,
    image_copy_capture::{
        CaptureFrame, CaptureSession, CaptureSource, Formats, Frame, ImageCopyCaptureFrameData,
        ImageCopyCaptureFrameDataExt, ImageCopyCaptureHandler, ImageCopyCaptureSessionData,
        ImageCopyCaptureSessionDataExt, ImageCopyCaptureState,
    },
    output::{OutputHandler, OutputState},
    reexports::{
        client::{
            delegate_dispatch, delegate_noop,
            globals::registry_queue_init,
            protocol::{wl_buffer, wl_shm},
            Connection, QueueHandle, WEnum,
        },
        protocols::ext::{
            image_capture_source::v1::client::{
                ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
                ext_image_capture_source_v1::ExtImageCaptureSourceV1,
                ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
            },
            image_copy_capture::v1::client::{
                ext_image_copy_capture_frame_v1::{ExtImageCopyCaptureFrameV1, FailureReason},
                ext_image_copy_capture_manager_v1::{
                    ExtImageCopyCaptureManagerV1, Options as CaptureOptions,
                },
                ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
            },
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shm::{raw::RawPool, Shm, ShmHandler},
};
use std::sync::Mutex;
use wayland_client::protocol::wl_output::WlOutput;

struct AppData {
    shm_state: Shm,
    registry_state: RegistryState,
    output_state: OutputState,
    image_copy_capture_state: ImageCopyCaptureState,
    outputs_done: u32,
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!();
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl ImageCopyCaptureHandler for AppData {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn init_done(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        session: &CaptureSession,
        formats: &Formats,
    ) {
        let (width, height) = formats.buffer_size;
        let format = [
            wl_shm::Format::Abgr8888,
            wl_shm::Format::Xbgr8888,
            wl_shm::Format::Xrgb8888,
            wl_shm::Format::Argb8888,
        ]
        .into_iter()
        .find(|format| formats.shm_formats.contains(format))
        .unwrap();
        let mut pool = RawPool::new(width as usize * height as usize * 4, &self.shm_state).unwrap();
        let wl_buffer =
            pool.create_buffer(0, width as i32, height as i32, width as i32 * 4, format, (), qh);
        session.capture(
            &wl_buffer,
            &[],
            qh,
            FrameData {
                frame_data: Default::default(),
                output_name: session.data::<SessionData>().unwrap().output_name.clone(),
                pool: Mutex::new(pool),
                size: formats.buffer_size,
                format,
            },
        );
    }

    fn stopped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &CaptureSession) {}

    fn ready(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        capture_frame: &CaptureFrame,
        _: Frame,
    ) {
        let data = capture_frame.data::<FrameData>().unwrap();
        let mut pool = data.pool.lock().unwrap();
        let bytes = pool.mmap();
        let mut rgba = Vec::with_capacity(bytes.len());
        for px in bytes.chunks_exact(4) {
            let (r, g, b, a) = match data.format {
                wl_shm::Format::Argb8888 => (px[2], px[1], px[0], px[3]),
                wl_shm::Format::Xrgb8888 => (px[2], px[1], px[0], u8::MAX),
                wl_shm::Format::Abgr8888 => (px[0], px[1], px[2], px[3]),
                wl_shm::Format::Xbgr8888 => (px[0], px[1], px[2], u8::MAX),
                _ => unreachable!("unsupported shm format {:?}", data.format),
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        let path = format!("{}.png", data.output_name);

        save_buffer(&path, &rgba, data.size.0, data.size.1, ColorType::Rgba8).unwrap();
        println!("Written image to '{}'.", path);

        self.outputs_done += 1;
    }

    fn failed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &CaptureFrame,
        reason: WEnum<FailureReason>,
    ) {
        println!("Failed to capture output: {:?}", reason);
        self.outputs_done += 1;
    }
}

struct SessionData {
    session_data: ImageCopyCaptureSessionData,
    output_name: String,
}

impl ImageCopyCaptureSessionDataExt for SessionData {
    fn image_copy_capture_session_data(&self) -> &ImageCopyCaptureSessionData {
        &self.session_data
    }
}

struct FrameData {
    frame_data: ImageCopyCaptureFrameData,
    output_name: String,
    pool: Mutex<RawPool>,
    size: (u32, u32),
    format: wl_shm::Format,
}

impl ImageCopyCaptureFrameDataExt for FrameData {
    fn image_copy_capture_frame_data(&self) -> &ImageCopyCaptureFrameData {
        &self.frame_data
    }
}

fn main() {
    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let shm_state = Shm::bind(&globals, &qh).unwrap();
    let image_copy_capture_state = ImageCopyCaptureState::new(&globals, &qh);
    let output_state = OutputState::new(&globals, &qh);

    let mut data: AppData = AppData {
        output_state,
        shm_state,
        registry_state,
        image_copy_capture_state,
        outputs_done: 0,
    };

    event_queue.roundtrip(&mut data).unwrap();

    let mut num_outputs = 0;
    let _sessions = data
        .output_state
        .outputs()
        .map(|output| {
            num_outputs += 1;
            let info = data.output_state.info(&output).unwrap();
            data.image_copy_capture_state
                .capturer()
                .create_session(
                    &CaptureSource::Output(output),
                    CaptureOptions::empty(),
                    &qh,
                    SessionData {
                        output_name: info.name.clone().unwrap(),
                        session_data: Default::default(),
                    },
                )
                .unwrap()
        })
        .collect::<Vec<_>>();

    while data.outputs_done < num_outputs {
        event_queue.blocking_dispatch(&mut data).unwrap();
    }
}

delegate_registry!(AppData);
delegate_dispatch!(AppData: [ExtOutputImageCaptureSourceManagerV1: GlobalData] => ImageCopyCaptureState);
delegate_dispatch!(AppData: [ExtForeignToplevelImageCaptureSourceManagerV1: GlobalData] => ImageCopyCaptureState);
delegate_dispatch!(AppData: [ExtImageCaptureSourceV1: GlobalData] => ImageCopyCaptureState);
delegate_dispatch!(AppData: [ExtImageCopyCaptureManagerV1: GlobalData] => ImageCopyCaptureState);
delegate_dispatch!(AppData: [ExtImageCopyCaptureSessionV1: SessionData] => ImageCopyCaptureState);
delegate_dispatch!(AppData: [ExtImageCopyCaptureFrameV1: FrameData] => ImageCopyCaptureState);
delegate_noop!(AppData: ignore wl_buffer::WlBuffer);
delegate_dispatch2!(AppData);
