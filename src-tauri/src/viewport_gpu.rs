//! Shared wgpu plumbing for the offscreen renderers.
//!
//! Both the prepare-tab scene renderer (`viewport_render`) and the
//! G-code toolpath renderer (`toolpath_render`) render offscreen and
//! blit the readback into a webview canvas (Strategy A). The camera
//! math, MSAA target allocation, and tight-RGBA readback are identical
//! between them, so they live here once.

use std::sync::{Arc, OnceLock};

use glam::camera::rh::{proj::directx::perspective as perspective_rh, view::look_at_mat4 as look_at_rh};
use glam::{Mat4, Vec3};

/// Read-back color format. Plain RGBA8 so `putImageData` can consume it
/// directly on the frontend.
pub const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// 4x MSAA — cheap edge anti-aliasing; the multisampled target resolves
/// into the single-sample `color` that's read back.
pub const SAMPLES: u32 = 4;

/// Process-wide offscreen render device, created once on first use.
static SHARED: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();

/// Lazily create (once) and return the process-wide offscreen render device.
/// Both offscreen renderers share it — they use identical `DeviceDescriptor`s,
/// and one device avoids allocating two GPU contexts.
pub fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    SHARED
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .expect("wgpu: no adapter");
            let info = adapter.get_info();
            tracing::info!("offscreen wgpu adapter: {} | {:?}", info.name, info.backend);
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .expect("wgpu: request_device");
            (Arc::new(device), Arc::new(queue))
        })
        .clone()
}

/// View-projection for the orbit camera (z up). Shared by render and
/// pick so the click ray matches exactly what's drawn.
pub fn view_proj(w: f32, h: f32, az: f32, el: f32, dist: f32, center: Vec3) -> Mat4 {
    let eye = cam_eye(az, el, dist, center);
    let far = (dist * 10.0).max(1000.0);
    let proj = perspective_rh(45f32.to_radians(), w / h, 0.1, far);
    // The frontend clamps `el` just shy of ±90° (EL_LIMIT), so the eye
    // never sits exactly over the center and world-Z up stays defined.
    proj * look_at_rh(eye, center, Vec3::Z)
}

/// Eye position for the orbit camera (matches `view_proj`).
pub fn cam_eye(az: f32, el: f32, dist: f32, center: Vec3) -> Vec3 {
    let (ce, se) = (el.cos(), el.sin());
    let (ca, sa) = (az.cos(), az.sin());
    center + dist * Vec3::new(ce * ca, ce * sa, se)
}

/// Cursor world ray (origin, unit dir) for the orbit camera — the same
/// unprojection the pick uses, so hit-tests match what's drawn.
pub fn cursor_ray(w: f32, h: f32, x: f32, y: f32, az: f32, el: f32, dist: f32, center: Vec3) -> (Vec3, Vec3) {
    let inv = view_proj(w, h, az, el, dist, center).inverse();
    let ndc = Vec3::new(2.0 * x / w - 1.0, 1.0 - 2.0 * y / h, 0.0);
    let ro = inv.project_point3(ndc);
    let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
    (ro, (far - ro).normalize())
}

/// Closest distance between the ray and segment [a,b], with the ray
/// parameter at the closest point (picks the nearest hit to the camera).
pub fn ray_seg_dist(ro: Vec3, rd: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let ab = b - a;
    let r = ro - a;
    let (aa, bb, d, e) = (ab.dot(ab), ab.dot(rd), ab.dot(r), rd.dot(r));
    let denom = aa - bb * bb;
    let s = if denom > 1e-9 { ((d - bb * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let t = (bb * s - e).max(0.0);
    ((a + ab * s - (ro + rd * t)).length(), t)
}

/// Map the offscreen readback buffer into tight RGBA8, top row first
/// (strips the 256-byte row padding `make_targets` allocated).
pub fn read_rgba(device: &wgpu::Device, readback: &wgpu::Buffer, padded_bpr: u32, w: u32, h: u32) -> Vec<u8> {
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let mapped = slice.get_mapped_range();
    let row = (w * 4) as usize;
    let mut out = vec![0u8; row * h as usize];
    for y in 0..h as usize {
        let src = y * padded_bpr as usize;
        out[y * row..(y + 1) * row].copy_from_slice(&mapped[src..src + row]);
    }
    drop(mapped);
    readback.unmap();
    out
}

/// Allocate the MSAA color/depth targets, the single-sample resolve
/// target (the read-back source), and the padded readback buffer.
/// Returns `(resolve_color, msaa_view, depth_view, readback, padded_bpr)`.
pub fn make_targets(
    device: &wgpu::Device,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView, wgpu::Buffer, u32) {
    let ext = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    // Single-sample resolve target — what gets read back.
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu.color"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // Multisampled color the scene actually renders into.
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu.msaa"),
        size: ext,
        mip_level_count: 1,
        sample_count: SAMPLES,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu.depth"),
        size: ext,
        mip_level_count: 1,
        sample_count: SAMPLES,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let padded_bpr = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu.readback"),
        size: (padded_bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (
        color,
        msaa.create_view(&Default::default()),
        depth.create_view(&Default::default()),
        readback,
        padded_bpr,
    )
}

