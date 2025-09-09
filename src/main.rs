use std::fs::File;
use std::io::{Read, Seek};
use std::os::unix::io::AsFd;

use image::{ImageBuffer, Rgba};
use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection,
    Dispatch,
    QueueHandle,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{Event as FrameEvent, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};

struct AppState {
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    output: Option<wl_output::WlOutput>,
    shm: Option<wl_shm::WlShm>,
    buffer_done: bool,
    buffer_file: Option<File>,
    buffer_format: wl_shm::Format,
    buffer_width: u32,
    buffer_height: u32,
    buffer_stride: u32,
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for AppState {
    fn event(
        state: &mut AppState,
        frame: &ZwlrScreencopyFrameV1,
        event: FrameEvent,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        match event {
            FrameEvent::Buffer { format, width, height, stride } => {
                state.buffer_format = format.into_result().unwrap();
                state.buffer_width = width;
                state.buffer_height = height;
                state.buffer_stride = stride;

                let shm = state.shm.as_ref().unwrap();
                let file = tempfile::tempfile().unwrap();
                file.set_len((height * stride) as u64).unwrap();

                let pool = shm.create_pool(file.as_fd(), (height * stride) as i32, qh, ());
                let buffer = pool.create_buffer(
                    0,
                    width as i32,
                    height as i32,
                    stride as i32,
                    state.buffer_format,
                    qh,
                    (),
                );
                frame.copy(&buffer);
                state.buffer_file = Some(file);
            }
            FrameEvent::Ready { .. } => {
                state.buffer_done = true;
            }
            FrameEvent::Failed => {
                eprintln!("截图失败!");
                state.buffer_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut AppState,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match interface.as_str() {
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy_manager = Some(registry.bind(name, 3, qh, ()));
                }
                "wl_output" => {
                    if state.output.is_none() {
                        state.output = Some(registry.bind(name, 4, qh, ()));
                    }
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

fn main() {
    let conn = Connection::connect_to_env().unwrap();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());

    let mut app_state = AppState {
        screencopy_manager: None,
        output: None,
        shm: None,
        buffer_done: false,
        buffer_file: None,
        buffer_format: wl_shm::Format::Xrgb8888,
        buffer_width: 0,
        buffer_height: 0,
        buffer_stride: 0,
    };

    event_queue.roundtrip(&mut app_state).unwrap();

    let manager = app_state
        .screencopy_manager
        .as_ref()
        .expect("Compositor does not support zwlr_screencopy_manager_v1");
    let output = app_state.output.as_ref().expect("No wl_output found");
    let _shm = app_state.shm.as_ref().expect("No wl_shm found");

    println!("发起截图...");
    manager.capture_output(0, output, &qh, ());

    while !app_state.buffer_done {
        event_queue.blocking_dispatch(&mut app_state).unwrap();
    }

    println!("截图流程结束。");
    if let Some(mut file) = app_state.buffer_file {
        file.rewind().unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();

        let mut tight_buf =
            Vec::with_capacity((app_state.buffer_width * app_state.buffer_height * 4) as usize);
        let bytes_per_pixel = 4;
        for y in 0..app_state.buffer_height {
            let row_start = (y * app_state.buffer_stride) as usize;
            for x in 0..app_state.buffer_width {
                let pixel_start = row_start + (x * bytes_per_pixel) as usize;
                let b = buf[pixel_start];
                let g = buf[pixel_start + 1];
                let r = buf[pixel_start + 2];
                tight_buf.push(r);
                tight_buf.push(g);
                tight_buf.push(b);
                tight_buf.push(255); // Alpha
            }
        }

        let image_buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(
            app_state.buffer_width,
            app_state.buffer_height,
            tight_buf,
        )
        .unwrap();

        image_buffer.save("screenshot.png").unwrap();
        println!("截图已保存到 screenshot.png");
    }
}