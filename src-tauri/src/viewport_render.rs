//! Strategy-A wgpu viewport: render the live scene to an offscreen texture and
//! read it back as RGBA8 so the frontend can blit it into an opaque `<canvas>`.
//!
//! WebKitGTK can't composite a transparent webview over native GPU content
//! (dynamic DOM smears — see docs/dev/wgpu-renderer.md), so the renderer lives in
//! Rust and hands finished frames to the webview. The GPU-resident scene mirror
//! is here too (per the decision doc): meshes are uploaded once, keyed by
//! `MeshId`, and drawn each frame with their object transforms. Camera stays
//! frontend-owned (passed in per frame).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::core::project::Project;
use crate::core::scene::state::MeshId;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

const SHADER: &str = r#"
struct U { mvp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = nrm; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  if (dot(i.n, i.n) < 0.01) { return vec4<f32>(0.34, 0.36, 0.40, 1.0); } // grid line
  let n = normalize(i.n);
  // Matte, two-sided lighting matching the Three.js viewport (ambient 0.55 +
  // key + fill). High ambient + abs() compresses contrast so the source model's
  // inconsistent/flipped normals don't carve stark facets (the "missing shapes"
  // artifact); a shinier, one-sided model exposed them.
  let key = normalize(vec3<f32>(0.5, -0.5, 0.85));
  let fill = normalize(vec3<f32>(-0.5, 0.5, 0.33));
  let d = 0.55 + abs(dot(n, key)) * 0.4 + abs(dot(n, fill)) * 0.1;
  return vec4<f32>(vec3<f32>(0.82, 0.72, 0.45) * min(d, 1.0), 1.0);
}
"#;

const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
// Bed extents are f64 (BoundingBox); converted to f32 for the GPU.
const DEFAULT_BED: ([f64; 3], [f64; 3]) = ([-110.0, -110.0, 0.0], [110.0, 110.0, 200.0]);

/// Camera + target the frontend passes per frame (camera is frontend-owned).
#[derive(serde::Deserialize)]
pub struct FrameRequest {
    pub width: u32,
    pub height: u32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
}

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    n_indices: u32,
}

/// Build our interleaved vertex buffer for a `Mesh` and upload it. Normals are
/// **recomputed** (area-weighted smooth vertex normals from the geometry), not
/// taken from `m.normals`: imported models often carry bad/flat per-vertex
/// normals that shade as faceted "missing shapes" even though the geometry is
/// fine (it slices smooth). The mesh is welded (shared vertices), so accumulating
/// face normals per vertex yields the smooth shading the slice shows.
fn upload_mesh(device: &wgpu::Device, m: &crate::core::scene::state::Mesh) -> GpuMesh {
    let vcount = m.vertices.len() / 3;
    let pos: Vec<Vec3> = (0..vcount)
        .map(|i| Vec3::new(m.vertices[3 * i], m.vertices[3 * i + 1], m.vertices[3 * i + 2]))
        .collect();
    let mut nrm = vec![Vec3::ZERO; vcount];
    for tri in m.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if a < vcount && b < vcount && c < vcount {
            let face = (pos[b] - pos[a]).cross(pos[c] - pos[a]); // area-weighted
            nrm[a] += face;
            nrm[b] += face;
            nrm[c] += face;
        }
    }
    let verts: Vec<Vertex> = (0..vcount)
        .map(|i| Vertex {
            pos: pos[i].to_array(),
            nrm: nrm[i].normalize_or_zero().to_array(),
        })
        .collect();
    GpuMesh {
        vb: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.mesh.vb"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        ib: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.mesh.ib"),
            contents: bytemuck::cast_slice(&m.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        n_indices: m.indices.len() as u32,
    }
}

/// Build-plate grid lines on the bed floor (`z = min.z`), ~10mm spacing.
fn grid_verts(min: [f32; 3], max: [f32; 3]) -> Vec<Vertex> {
    let z = min[2];
    let step = 10.0_f32;
    let mut v = Vec::new();
    let nx = ((max[0] - min[0]) / step).ceil() as i32;
    let ny = ((max[1] - min[1]) / step).ceil() as i32;
    for i in 0..=nx {
        let x = min[0] + i as f32 * step;
        v.push(Vertex { pos: [x, min[1], z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [x, max[1], z], nrm: [0.0; 3] });
    }
    for j in 0..=ny {
        let y = min[1] + j as f32 * step;
        v.push(Vertex { pos: [min[0], y, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [max[0], y, z], nrm: [0.0; 3] });
    }
    v
}

pub struct ViewportRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bgl: wgpu::BindGroupLayout,
    mesh_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
    // per-object MVP, one 256-aligned slot each
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    slot: u32,
    slots_cap: u32,
    // caches
    meshes: HashMap<MeshId, GpuMesh>,
    grid_key: Option<[f32; 6]>,
    vb_grid: wgpu::Buffer,
    n_grid: u32,
    // size-dependent targets
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

        let slot = device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(64);
        let slots_cap = 64u32;
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport.mvp"),
            size: (slot * slots_cap) as u64,
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
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(64),
                },
                count: None,
            }],
        });
        let bind = make_bind(&device, &bgl, &ubuf);

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
        let make_pipe = |topology, cull| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
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
                    topology,
                    cull_mode: cull,
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
            })
        };
        // No back-face culling: imported meshes (STL etc.) have no winding
        // guarantee, and mixed winding would drop valid front faces (holes). The
        // depth test still resolves the nearest face correctly.
        let mesh_pipe = make_pipe(wgpu::PrimitiveTopology::TriangleList, None);
        let line_pipe = make_pipe(wgpu::PrimitiveTopology::LineList, None);

        let gmin = DEFAULT_BED.0.map(|v| v as f32);
        let gmax = DEFAULT_BED.1.map(|v| v as f32);
        let gverts = grid_verts(gmin, gmax);
        let vb_grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.grid"),
            contents: bytemuck::cast_slice(&gverts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (color, depth_view, readback, padded_bpr) = make_targets(&device, 8, 8);
        ViewportRenderer {
            n_grid: gverts.len() as u32,
            grid_key: Some([gmin[0], gmin[1], gmin[2], gmax[0], gmax[1], gmax[2]]),
            device,
            queue,
            bgl,
            mesh_pipe,
            line_pipe,
            ubuf,
            bind,
            slot,
            slots_cap,
            meshes: HashMap::new(),
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

    fn ensure_grid(&mut self, min: [f32; 3], max: [f32; 3]) {
        let key = [min[0], min[1], min[2], max[0], max[1], max[2]];
        if self.grid_key == Some(key) {
            return;
        }
        let gverts = grid_verts(min, max);
        self.vb_grid = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport.grid"),
            contents: bytemuck::cast_slice(&gverts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.n_grid = gverts.len() as u32;
        self.grid_key = Some(key);
    }

    fn ensure_mvp_capacity(&mut self, slots: u32) {
        if slots <= self.slots_cap {
            return;
        }
        self.slots_cap = slots.next_power_of_two();
        self.ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport.mvp"),
            size: (self.slot * self.slots_cap) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bind = make_bind(&self.device, &self.bgl, &self.ubuf);
    }

    /// Render the live scene and read it back as tight RGBA8, top row first.
    pub fn frame(&mut self, req: &FrameRequest, project: &Arc<Mutex<Project>>) -> Vec<u8> {
        let (w, h) = (req.width.max(1), req.height.max(1));
        self.resize(w, h);

        // --- gather under the project lock (cheap): bed + per-object models ---
        let (bmin, bmax, draws) = {
            let p = project.lock().unwrap();
            let plate = p.active_plate();
            let (bmin, bmax) = plate
                .scene
                .bed
                .as_ref()
                .map(|b| (b.extents.min, b.extents.max))
                .unwrap_or(DEFAULT_BED);
            let mut draws: Vec<(MeshId, Mat4)> = Vec::new();
            for (_, obj) in plate.scene.objects.iter() {
                if !obj.visible {
                    continue;
                }
                if !self.meshes.contains_key(&obj.mesh) {
                    match p.meshes.get(&obj.mesh) {
                        Some(m) => {
                            self.meshes.insert(obj.mesh, upload_mesh(&self.device, m));
                        }
                        None => tracing::warn!(
                            "viewport: mesh {:?} for object {:?} NOT in registry",
                            obj.mesh,
                            obj.id
                        ),
                    }
                }
                if self.meshes.contains_key(&obj.mesh) {
                    draws.push((obj.mesh, obj.transform.to_mat4()));
                }
            }
            (bmin, bmax, draws)
        };
        let bmin = bmin.map(|v| v as f32);
        let bmax = bmax.map(|v| v as f32);

        self.ensure_grid(bmin, bmax);

        // Camera (frontend-owned), z up.
        let center = Vec3::from(req.center);
        let (ce, se) = (req.el.cos(), req.el.sin());
        let (ca, sa) = (req.az.cos(), req.az.sin());
        let eye = center + req.dist * Vec3::new(ce * ca, ce * sa, se);
        let far = (req.dist * 10.0).max(1000.0);
        let proj = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.1, far);
        let vp = proj * Mat4::look_at_rh(eye, center, Vec3::Z);

        // Pack MVPs: slot 0 = grid (model identity), slots 1.. = vp * model.
        self.ensure_mvp_capacity(1 + draws.len() as u32);
        let mut bytes = vec![0u8; self.slot as usize * (1 + draws.len())];
        bytes[0..64].copy_from_slice(bytemuck::cast_slice(&vp.to_cols_array()));
        for (i, (_, model)) in draws.iter().enumerate() {
            let mvp = vp * *model;
            let off = (i + 1) * self.slot as usize;
            bytes[off..off + 64].copy_from_slice(bytemuck::cast_slice(&mvp.to_cols_array()));
        }
        self.queue.write_buffer(&self.ubuf, 0, &bytes);

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
            // grid
            rp.set_pipeline(&self.line_pipe);
            rp.set_bind_group(0, &self.bind, &[0]);
            rp.set_vertex_buffer(0, self.vb_grid.slice(..));
            rp.draw(0..self.n_grid, 0..1);
            // meshes
            rp.set_pipeline(&self.mesh_pipe);
            for (i, (mesh_id, _)) in draws.iter().enumerate() {
                let off = ((i + 1) as u32) * self.slot;
                let gm = &self.meshes[mesh_id];
                rp.set_bind_group(0, &self.bind, &[off]);
                rp.set_vertex_buffer(0, gm.vb.slice(..));
                rp.set_index_buffer(gm.ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..gm.n_indices, 0, 0..1);
            }
        }
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

fn make_bind(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    ubuf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: ubuf,
                offset: 0,
                size: std::num::NonZeroU64::new(64),
            }),
        }],
    })
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
    let padded_bpr = (w * 4).div_ceil(256) * 256;
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

/// Render one frame and return tight RGBA8 bytes.
#[tauri::command]
pub fn viewport_frame(
    state: tauri::State<'_, ViewportState>,
    project: tauri::State<'_, Arc<Mutex<Project>>>,
    req: FrameRequest,
) -> tauri::ipc::Response {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ViewportRenderer::new);
    tauri::ipc::Response::new(r.frame(&req, project.inner()))
}

/// Initial camera framing for the active plate's bed — the frontend sets the
/// orbit center + distance from this on load / bed change.
#[derive(serde::Serialize)]
pub struct SceneInfo {
    pub center: [f32; 3],
    pub distance: f32,
}

#[tauri::command]
pub fn viewport_scene_info(project: tauri::State<'_, Arc<Mutex<Project>>>) -> SceneInfo {
    let p = project.lock().unwrap();
    let (min, max) = p
        .active_plate()
        .scene
        .bed
        .as_ref()
        .map(|b| (b.extents.min, b.extents.max))
        .unwrap_or(DEFAULT_BED);
    let span = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
    SceneInfo {
        center: [
            ((min[0] + max[0]) / 2.0) as f32,
            ((min[1] + max[1]) / 2.0) as f32,
            0.0,
        ],
        distance: (span * 1.7) as f32,
    }
}

/// Whether the Strategy-A wgpu viewport is enabled (`N3O_WGPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("N3O_WGPU").is_some()
}
