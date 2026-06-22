//! Strategy-A wgpu viewport: render the 3D scene to an offscreen texture and
//! read it back as RGBA8 so the frontend can blit it into an opaque `<canvas>`.
//!
//! WebKitGTK can't composite a transparent webview over native GPU content
//! (dynamic DOM smears — see docs/dev/wgpu-renderer.md), so instead of presenting
//! a GL surface *behind* the page, the renderer lives in Rust and hands finished
//! frames to the webview. The cost is a per-frame GPU→CPU readback + transport;
//! at edit-viewport sizes (render-at-viewport-size, on-demand) that's fine.
//!
//! Foundation: an orbitable build-plate grid. Real meshes (from
//! `scene_mesh_buffers`) land next; the device/pipeline/readback path is the
//! reusable part.

use std::sync::Mutex;

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

const SHADER: &str = r#"
struct U { vp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.vp * vec4<f32>(pos, 1.0); o.n = nrm; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  if (dot(i.n, i.n) < 0.01) { return vec4<f32>(0.34, 0.36, 0.40, 1.0); } // grid line
  let l = normalize(vec3<f32>(0.4, 0.5, 0.9));
  let d = max(dot(normalize(i.n), l), 0.0) * 0.75 + 0.25;
  return vec4<f32>(vec3<f32>(0.82, 0.72, 0.45) * d, 1.0); // lit mesh
}
"#;

const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Camera + target the frontend passes per frame.
#[derive(serde::Deserialize)]
pub struct FrameRequest {
    pub width: u32,
    pub height: u32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    /// Orbit/look-at center (bed center, z up).
    pub center: [f32; 3],
}

/// Build-plate grid lines on z=0, `half` mm each side, `step` mm spacing. Unlit
/// (normal = 0 → flagged in the shader).
fn grid(half: f32, step: f32) -> Vec<Vertex> {
    let z = 0.0;
    let mut v = Vec::new();
    let n = (half / step).floor() as i32;
    for i in -n..=n {
        let x = i as f32 * step;
        v.push(Vertex { pos: [x, -half, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [x, half, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [-half, x, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [half, x, z], nrm: [0.0; 3] });
    }
    v
}

pub struct ViewportRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    line_pipe: wgpu::RenderPipeline,
    vb_grid: wgpu::Buffer,
    n_grid: u32,
    // size-dependent targets, recreated on resize
    size: (u32, u32),
    color: wgpu::Texture,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bpr: u32,
}

impl ViewportRenderer {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("wgpu: no adapter");
        let info = adapter.get_info();
        tracing::info!("viewport wgpu adapter: {} | {:?}", info.name, info.backend);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("wgpu: request_device");

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport.ubuf"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let vbl = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
        };
        let line_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport.lines"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs",
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(&vbl),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs",
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FMT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
        });

        let grid_verts = grid(110.0, 10.0); // placeholder bed; real extents next
        let vb_grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.grid"),
            contents: bytemuck::cast_slice(&grid_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (color, depth_view, readback, padded_bpr) = make_targets(&device, 8, 8);
        ViewportRenderer {
            n_grid: grid_verts.len() as u32,
            device,
            queue,
            ubuf,
            bind,
            line_pipe,
            vb_grid,
            size: (0, 0),
            color,
            depth_view,
            readback,
            padded_bpr,
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if self.size == (w, h) {
            return;
        }
        self.size = (w, h);
        let (color, depth_view, readback, padded_bpr) = make_targets(&self.device, w, h);
        self.color = color;
        self.depth_view = depth_view;
        self.readback = readback;
        self.padded_bpr = padded_bpr;
    }

    /// Render one frame and read it back as tight (unpadded) RGBA8, top-row first
    /// — ready for `putImageData`.
    pub fn frame(&mut self, req: &FrameRequest) -> Vec<u8> {
        let (w, h) = (req.width.max(1), req.height.max(1));
        self.resize(w, h);

        let center = Vec3::from(req.center);
        let (ce, se) = (req.el.cos(), req.el.sin());
        let (ca, sa) = (req.az.cos(), req.az.sin());
        let eye = center + req.dist * Vec3::new(ce * ca, ce * sa, se);
        let proj = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.1, req.dist * 10.0);
        let vp = proj * Mat4::look_at_rh(eye, center, Vec3::Z);
        self.queue
            .write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&vp.to_cols_array()));

        let color_view = self.color.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.10,
                            g: 0.10,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_bind_group(0, &self.bind, &[]);
            rp.set_pipeline(&self.line_pipe);
            rp.set_vertex_buffer(0, self.vb_grid.slice(..));
            rp.draw(0..self.n_grid, 0..1);
        }
        // Copy the rendered texture into the readback buffer (256-byte aligned rows).
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(enc.finish()));

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let mapped = slice.get_mapped_range();

        // Unpad rows: padded_bpr → w*4. RGBA8 maps straight to ImageData.
        let row = (w * 4) as usize;
        let mut out = vec![0u8; row * h as usize];
        for y in 0..h as usize {
            let src = y * self.padded_bpr as usize;
            out[y * row..(y + 1) * row].copy_from_slice(&mapped[src..src + row]);
        }
        drop(mapped);
        self.readback.unmap();
        out
    }
}

fn make_targets(
    device: &wgpu::Device,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let ext = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport.color"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport.depth"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let padded_bpr = (w * 4).div_ceil(256) * 256; // 256-byte row alignment for copy
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport.readback"),
        size: (padded_bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (color, depth.create_view(&Default::default()), readback, padded_bpr)
}

/// Tauri-managed renderer (lazily created on first frame; wgpu init is ~100ms).
#[derive(Default)]
pub struct ViewportState(pub Mutex<Option<ViewportRenderer>>);

/// Render one frame at the requested size/camera and return tight RGBA8 bytes.
#[tauri::command]
pub fn viewport_frame(
    state: tauri::State<'_, ViewportState>,
    req: FrameRequest,
) -> tauri::ipc::Response {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ViewportRenderer::new);
    tauri::ipc::Response::new(r.frame(&req))
}

/// Whether the Strategy-A wgpu viewport is enabled (`N3O_WGPU=1`). The frontend
/// reads this to mount the wgpu canvas instead of the Three.js viewport.
pub fn enabled() -> bool {
    std::env::var_os("N3O_WGPU").is_some()
}
