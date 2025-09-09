use std::fs::File;
use std::io::{Read, Seek};
use std::os::unix::io::AsFd;


use clap::Parser;
use image::{GenericImage, ImageBuffer, Rgba};
use png;
use rayon::prelude::*;
use fast_image_resize::{images::Image, Resizer, ResizeOptions, ResizeAlg, PixelType};
use wayland_client::{
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{Event as FrameEvent, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    geometry: Option<String>,
}

fn parse_geometry(geometry: &str) -> Option<(i32, i32, u32, u32)> {
    let parts: Vec<&str> = geometry.split(' ').collect();
    if parts.len() != 2 {
        return None;
    }

    let coords: Vec<&str> = parts[0].split(',').collect();
    if coords.len() != 2 {
        return None;
    }

    let dims: Vec<&str> = parts[1].split('x').collect();
    if dims.len() != 2 {
        return None;
    }

    let x = coords[0].parse::<i32>().ok()?;
    let y = coords[1].parse::<i32>().ok()?;
    let width = dims[0].parse::<u32>().ok()?;
    let height = dims[1].parse::<u32>().ok()?;

    Some((x, y, width, height))
}

#[derive(Clone, Debug)]
struct OutputInfo {
    output: wl_output::WlOutput,
    xdg_output: Option<zxdg_output_v1::ZxdgOutputV1>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: i32,
}

struct AppState {
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: Vec<OutputInfo>,
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
        _frame: &ZwlrScreencopyFrameV1,
        event: FrameEvent,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        match event {
            FrameEvent::Buffer {
                format,
                width,
                height,
                stride,
            } => {
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
                _frame.copy(&buffer);
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
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some(info) = state.outputs.iter_mut().find(|info| info.output == *output) {
            match event {
                wl_output::Event::Mode { flags, width, height, .. } => {
                    if let Ok(flags) = flags.into_result() {
                        if flags.contains(wl_output::Mode::Current) {
                            info.width = width;
                            info.height = height;
                        }
                    }
                }
                wl_output::Event::Scale { factor } => {
                    info.scale = factor;
                }
                _ => {}
            }
        }
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

impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for AppState {
    fn event(
        _: &mut Self,
        _: &zxdg_output_manager_v1::ZxdgOutputManagerV1,
        _: zxdg_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        xdg_output: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some(info) = state
            .outputs
            .iter_mut()
            .find(|info| info.xdg_output.as_ref().map_or(false, |o| o == xdg_output))
        {
            if let zxdg_output_v1::Event::LogicalPosition { x, y } = event {
                info.x = x;
                info.y = y;
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy_manager = Some(registry.bind(name, 3, qh, ()));
                }
                "zxdg_output_manager_v1" => {
                    state.xdg_output_manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                "wl_output" => {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                    state.outputs.push(OutputInfo {
                        output,
                        xdg_output: None,
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                        scale: 1,
                    });
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

fn save_as_png_fast(
    image: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    path: &str,
) -> Result<(), png::EncodingError> {
    let file = std::fs::File::create(path)?;
    let w = &mut std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(image.as_raw())?;
    Ok(())
}

fn main() {
    let args = Args::parse();

    let conn = Connection::connect_to_env().unwrap();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());

    let mut app_state = AppState {
        screencopy_manager: None,
        xdg_output_manager: None,
        outputs: Vec::new(),
        shm: None,
        buffer_done: false,
        buffer_file: None,
        buffer_format: wl_shm::Format::Xrgb8888,
        buffer_width: 0,
        buffer_height: 0,
        buffer_stride: 0,
    };

    event_queue.roundtrip(&mut app_state).unwrap();

    if let Some(manager) = &app_state.xdg_output_manager {
        for info in &mut app_state.outputs {
            if info.xdg_output.is_none() {
                let xdg_output = manager.get_xdg_output(&info.output, &qh, ());
                info.xdg_output = Some(xdg_output);
            }
        }
    }

    event_queue.roundtrip(&mut app_state).unwrap();
    event_queue.roundtrip(&mut app_state).unwrap();

    // Geometry is handled in logical pixels until compositing.

    let manager = app_state
        .screencopy_manager
        .as_ref()
        .expect("Compositor does not support zwlr_screencopy_manager_v1")
        .clone();
    let _shm = app_state.shm.as_ref().expect("No wl_shm found");

    let mut crop_details: Option<(i32, i32, u32, u32)> = None;
    let mut target_outputs: Vec<OutputInfo> = Vec::new();

    if let Some(geometry_str) = &args.geometry {
        if let Some((gx, gy, gwidth, gheight)) = parse_geometry(geometry_str) {
            println!("[DEBUG] Slurp geometry: x={}, y={}, width={}, height={}", gx, gy, gwidth, gheight);
            println!("[DEBUG] Detected outputs (logical pixels):");
            for info in &app_state.outputs {
                println!("[DEBUG]   Output: x={}, y={}, width={}, height={}, scale={}", info.x, info.y, info.width, info.height, info.scale);
            }
            
            target_outputs = app_state
                .outputs
                .iter()
                .filter(|info| {
                    let ox = info.x;
                    let oy = info.y;
                    let ow = info.width;
                    let oh = info.height;
                    let sx = gx;
                    let sy = gy;
                    let sw = gwidth as i32;
                    let sh = gheight as i32;

                    sx < ox + ow && sx + sw > ox && sy < oy + oh && sy + sh > oy
                })
                .cloned()
                .collect();
            
            println!("[DEBUG] Filtered target outputs for geometry:");
            for info in &target_outputs {
                println!("[DEBUG]   -> Output: x={}, y={}, width={}, height={}", info.x, info.y, info.width, info.height);
            }
            
            crop_details = Some((gx, gy, gwidth, gheight));
        }
    }

    if target_outputs.is_empty() {
        if let Some(output) = app_state.outputs.first() {
            target_outputs.push(output.clone());
        }
    }

    if !target_outputs.is_empty() {
        let mut captured_data: Vec<(OutputInfo, ImageBuffer<Rgba<u8>, Vec<u8>>)> = Vec::new();

        for output_info in &target_outputs {
            app_state.buffer_done = false;
            app_state.buffer_file = None;

            println!("发起截图...");
            manager.capture_output(0, &output_info.output, &qh, ());

            while !app_state.buffer_done {
                event_queue.blocking_dispatch(&mut app_state).unwrap();
            }
            println!("截图流程结束。");

            if let Some(mut file) = app_state.buffer_file.take() {
                println!("[DEBUG] Processing buffer for output: x={}, y={}, width={}, height={}", output_info.x, output_info.y, output_info.width, output_info.height);
                println!("[DEBUG] Buffer details from event: width={}, height={}, stride={}", app_state.buffer_width, app_state.buffer_height, app_state.buffer_stride);

                file.rewind().unwrap();
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).unwrap();
                println!("[DEBUG] Read {} bytes from temp file.", buf.len());

                let tight_buf: Vec<u8> = (0..app_state.buffer_height)
                    .into_par_iter()
                    .flat_map(|y| {
                        let row_start = (y * app_state.buffer_stride) as usize;
                        let bytes_per_pixel = 4;
                        let mut row_data =
                            Vec::with_capacity((app_state.buffer_width * bytes_per_pixel) as usize);
                        for x in 0..app_state.buffer_width {
                            let pixel_start = row_start + (x * bytes_per_pixel) as usize;
                            let b = buf[pixel_start];
                            let g = buf[pixel_start + 1];
                            let r = buf[pixel_start + 2];
                            row_data.extend_from_slice(&[r, g, b, 255]);
                        }
                        row_data
                    })
                    .collect();

                println!("[DEBUG] Converted buffer size: {} bytes.", tight_buf.len());
                let image_buffer = ImageBuffer::from_raw(
                    app_state.buffer_width,
                    app_state.buffer_height,
                    tight_buf,
                )
                .unwrap();
                captured_data.push((output_info.clone(), image_buffer));
            }
        }

        if !captured_data.is_empty() {
            // Create a new Vec of outputs with dimensions corrected from the buffer
            let mut corrected_outputs = Vec::new();
            for (output_info, image_buffer) in &captured_data {
                let mut info = output_info.clone();
                info.width = image_buffer.width() as i32;
                info.height = image_buffer.height() as i32;
                corrected_outputs.push(info);
            }

            if captured_data.len() == 1 && crop_details.is_none() {
                // Only one screen, no crop, just save it
                save_as_png_fast(&captured_data[0].1, "screenshot.png").unwrap();
            } else {
                // Composite multiple images or crop a single one
                println!("[DEBUG] Starting image compositing/cropping...");

                // Bounds are calculated in logical pixels from the original target_outputs
                let min_x = target_outputs.iter().map(|o| o.x).min().unwrap_or(0);
                let min_y = target_outputs.iter().map(|o| o.y).min().unwrap_or(0);
                let max_x = target_outputs.iter().map(|o| o.x + o.width).max().unwrap_or(0);
                let max_y = target_outputs.iter().map(|o| o.y + o.height).max().unwrap_or(0);

                // The scale of the final composite image. Use the max scale from all targeted outputs.
                let composite_scale = target_outputs.iter().map(|o| o.scale).max().unwrap_or(1);
                
                let composite_width = ((max_x - min_x) * composite_scale) as u32;
                let composite_height = ((max_y - min_y) * composite_scale) as u32;

                println!("[DEBUG] Logical canvas bounds: min_x={}, min_y={}, max_x={}, max_y={}", min_x, min_y, max_x, max_y);
                println!("[DEBUG] Composite scale: {}", composite_scale);
                println!("[DEBUG] Physical composite canvas dimensions: width={}, height={}", composite_width, composite_height);

                let mut composite_image = ImageBuffer::new(composite_width, composite_height);

                println!("[DEBUG] Overlaying images...");
                for (output_info, image_buffer) in &captured_data {
                    let dest_x = (output_info.x - min_x) * composite_scale;
                    let dest_y = (output_info.y - min_y) * composite_scale;

                    let scaled_buffer = if output_info.scale != composite_scale {
                        println!("[DEBUG]   -> Scaling buffer for output at logical ({}, {}) from scale {} to {}", output_info.x, output_info.y, output_info.scale, composite_scale);
                        let new_width = (image_buffer.width() as f64 * composite_scale as f64 / output_info.scale as f64).round() as u32;
                        let new_height = (image_buffer.height() as f64 * composite_scale as f64 / output_info.scale as f64).round() as u32;
                        
                        // Use fast_image_resize for performance
                        // This involves a copy to create an owned Image, but the resize performance gain is worth it.
                        let src_image = Image::from_vec_u8(
                            image_buffer.width(),
                            image_buffer.height(),
                            image_buffer.to_vec(),
                            PixelType::U8x4,
                        ).unwrap();

                        // Create the destination image
                        let mut dst_image = Image::new(
                            new_width,
                            new_height,
                            PixelType::U8x4,
                        );

                        // Generic resizer
                        let mut resizer = Resizer::new();
                        // Set options with algorithm
                        let options = ResizeOptions::new().resize_alg(ResizeAlg::Nearest);
                        // Resize into the destination image's view
                        resizer.resize(&src_image, &mut dst_image, &options).unwrap();

                        ImageBuffer::from_raw(new_width, new_height, dst_image.into_vec()).unwrap()
                    } else {
                        image_buffer.clone()
                    };

                    println!("[DEBUG]   -> Overlaying image from output at logical ({}, {}) onto canvas at physical ({}, {})", output_info.x, output_info.y, dest_x, dest_y);
                    image::imageops::overlay(&mut composite_image, &scaled_buffer, dest_x as i64, dest_y as i64);
                }

                if let Some((gx, gy, gwidth, gheight)) = crop_details {
                    println!("[DEBUG] Cropping final image...");
                    
                    let crop_x = ((gx - min_x) * composite_scale) as u32;
                    let crop_y = ((gy - min_y) * composite_scale) as u32;
                    let final_gwidth = gwidth * composite_scale as u32;
                    let final_gheight = gheight * composite_scale as u32;

                    let final_gwidth = final_gwidth.min(composite_width.saturating_sub(crop_x));
                    let final_gheight = final_gheight.min(composite_height.saturating_sub(crop_y));

                    println!("[DEBUG] Crop details (physical): canvas_crop_x={}, canvas_crop_y={}, width={}, height={}", crop_x, crop_y, final_gwidth, final_gheight);

                    let cropped_image = composite_image.sub_image(crop_x, crop_y, final_gwidth, final_gheight);
                    save_as_png_fast(&cropped_image.to_image(), "screenshot.png").unwrap();
                } else {
                    println!("[DEBUG] No crop details, saving composite image directly.");
                    save_as_png_fast(&composite_image, "screenshot.png").unwrap();
                }
            }
            println!("截图已保存到 screenshot.png");
        }
    } else {
        eprintln!("No output found.");
    }
}
