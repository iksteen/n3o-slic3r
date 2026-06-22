//! Spike 0 — the Strategy A (offscreen wgpu -> blit to webview canvas) economic
//! make-or-break: how many ms does it cost to render an offscreen frame on the
//! real GPU AND read it back to CPU, per frame, at interactive resolutions?
//!
//! This measures the *Rust half* of Strategy A's per-frame cost: GPU render +
//! texture->buffer copy + GPU->CPU readback + buffer map. The *other half* (IPC
//! to the webview + canvas `putImageData` + WebKit composite) needs the Tauri
//! app and is the follow-up sub-spike. If readback alone blows the frame budget
//! here, Strategy A is dead and there's no point measuring the transfer.
//!
//! Budget: ~16.7 ms = one 60fps frame. Strategy A only needs to be cheaper than
//! the thing it replaces — WebKit's *software* triangle rasterization on Linux —
//! but 60fps is the honest interactive bar.

use std::time::{Duration, Instant};

fn main() {
    pollster::block_on(run());
}

async fn run() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .expect("no wgpu adapter (no Vulkan/GL?)");
    let info = adapter.get_info();
    println!(
        "adapter: {} | backend {:?} | type {:?} | driver {}",
        info.name, info.backend, info.device_type, info.driver
    );

    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        )
        .await
        .expect("no wgpu device");

    println!(
        "{:<7} {:>11}  {:>9}  {:>16}  {:>14}  {}",
        "res", "MB/frame", "encode ms", "render+readback ms", "readback MB/s", "60fps?"
    );
    for (w, h, label) in [(1920u32, 1080u32, "1080p"), (3840, 2160, "4K")] {
        bench(&device, &queue, w, h, label);
    }
}

fn bench(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, label: &str) {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = width * 4;
    // These widths are 256-aligned (1920*4=7680, 3840*4=15360), so no row
    // padding is needed for this spike.
    assert_eq!(bytes_per_row % 256, 0, "row not 256-aligned");
    let buffer_size = (bytes_per_row * height) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let warmup = 15;
    let frames = 120u32;
    let mut full = Duration::ZERO;
    let mut encode = Duration::ZERO;

    for i in 0..(warmup + frames) {
        let t0 = Instant::now();
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let encode_dt = t0.elapsed();
        queue.submit(Some(encoder.finish()));

        // Map + block until the GPU finishes and the readback is visible to CPU.
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().expect("map failed");
        {
            // Touch the bytes so the readback can't be optimized away.
            let data = slice.get_mapped_range();
            std::hint::black_box(data[0] ^ data[data.len() - 1]);
        }
        staging.unmap();

        if i >= warmup {
            full += t0.elapsed();
            encode += encode_dt;
        }
    }

    let full_ms = full.as_secs_f64() / frames as f64 * 1000.0;
    let encode_ms = encode.as_secs_f64() / frames as f64 * 1000.0;
    let mb = buffer_size as f64 / (1024.0 * 1024.0);
    let mb_s = mb / (full_ms / 1000.0);
    println!(
        "{:<7} {:>8.1} MB  {:>7.2}  {:>16.2}  {:>12.0}  {}",
        label,
        mb,
        encode_ms,
        full_ms,
        mb_s,
        if full_ms < 16.7 { "PASS" } else { "OVER" }
    );
}
