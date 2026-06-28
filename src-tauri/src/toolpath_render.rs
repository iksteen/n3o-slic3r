//! Strategy-A wgpu G-code toolpath renderer — the "spaghetti" preview.
//!
//! Sibling to [`crate::viewport_render`]: renders offscreen and reads
//! back tight RGBA8 for the frontend to blit into an opaque `<canvas>`.
//! Where the scene renderer draws meshes, this one draws every extrusion
//! as a solid width×height **tube**, one instance per segment, expanded
//! in the vertex shader from a shared cross-section template (no CPU tube
//! baking). It pulls the preview IR from the [`PreviewRegistry`] by handle
//! and caches the per-handle GPU buffers, mirroring how the scene renderer
//! caches meshes by `MeshId`.
//!
//! Controls carried over from the old layer-cake view map to GPU state:
//! color mode → the per-instance color buffer (rewritten on change, the
//! geometry buffer untouched); the layer-window slider → a shader uniform
//! cutoff (out-of-window instances collapse to a degenerate triangle);
//! travel / retraction visibility → whether their instanced draw runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::core::preview::ir::{PreviewGeometry, SegmentSet};
use crate::core::preview::{encode_colors, ColorMode, LoadedPreview, Palette, PreviewHandle, PreviewRegistry};
use crate::viewport_gpu::{cursor_ray, make_targets, ray_seg_dist, read_rgba, view_proj, COLOR_FMT, SAMPLES};

/// Travel moves render as thin tubes (mm); retractions as short vertical
/// red nubs. Both reuse the extrusion tube pipeline so there's one draw
/// path. ponytail: nubs over a dedicated point-sprite pipeline — visible
/// and free; revisit if the marker shape matters.
const TRAVEL_WIDTH_MM: f32 = 0.15;
const RETRACT_WIDTH_MM: f32 = 0.5;
const RETRACT_HEIGHT_MM: f32 = 0.6;
const TRAVEL_RGB: [f32; 3] = [0.45, 0.47, 0.52];
const RETRACT_RGB: [f32; 3] = [0.90, 0.13, 0.13];
/// Floor on tube cross-section so a zero-width segment still renders.
const MIN_DIM_MM: f32 = 0.05;
/// Cross-section ring resolution — 8 gives a visibly round tube cheaply.
const RING: usize = 8;
/// Extra world-space slop on the pick radius so thin lines stay grabbable.
const PICK_SLOP_MM: f32 = 0.35;

const SHADER: &str = r#"
struct U { vp: mat4x4<f32>, lmin: f32, lmax: f32, _p0: f32, _p1: f32 };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32>, @location(1) c: vec3<f32> };

@vertex fn vs(
  @location(0) tmpl: vec4<f32>,   // cross-section: x,y = ring dir; z = t (0|1); w = cap flag
  @location(1) start: vec3<f32>,
  @location(2) end: vec3<f32>,
  @location(3) dims: vec2<f32>,   // width, height (mm)
  @location(4) layer: f32,
  @location(5) col: vec3<f32>,
) -> VO {
  var o: VO;
  o.c = col;
  // Layer-window cutoff: collapse out-of-window instances to a zero-area
  // triangle (all verts at `start`) so they produce no fragments.
  if (layer < u.lmin - 0.5 || layer > u.lmax + 0.5) {
    o.p = u.vp * vec4<f32>(start, 1.0);
    o.n = vec3<f32>(0.0, 0.0, 1.0);
    return o;
  }
  let seg = end - start;
  let len = max(length(seg), 1e-6);
  let tangent = seg / len;
  // A stable frame: world-Z up unless the segment is near-vertical.
  var up_hint = vec3<f32>(0.0, 0.0, 1.0);
  if (abs(tangent.z) > 0.99) { up_hint = vec3<f32>(1.0, 0.0, 0.0); }
  let right = normalize(cross(tangent, up_hint));
  let up = cross(right, tangent);
  let center = mix(start, end, tmpl.z);
  let world = center + right * (tmpl.x * dims.x * 0.5) + up * (tmpl.y * dims.y * 0.5);
  o.p = u.vp * vec4<f32>(world, 1.0);
  if (tmpl.w > 0.5) {
    o.n = tangent * (tmpl.z * 2.0 - 1.0);   // end cap faces ±tangent
  } else {
    o.n = normalize(right * tmpl.x + up * tmpl.y);
  }
  return o;
}

@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  let n = normalize(i.n);
  // Matte two-sided shade, same lighting family as the scene viewport.
  let key = normalize(vec3<f32>(0.5, -0.5, 0.85));
  let fill = normalize(vec3<f32>(-0.5, 0.5, 0.33));
  let d = 0.55 + abs(dot(n, key)) * 0.4 + abs(dot(n, fill)) * 0.1;
  return vec4<f32>(i.c * min(d, 1.0), 1.0);
}
"#;

/// Unlit bed-grid lines — shares the uniform's `vp`, flat grid color.
const GRID_SHADER: &str = r#"
struct U { vp: mat4x4<f32>, lmin: f32, lmax: f32, _p0: f32, _p1: f32 };
@group(0) @binding(0) var<uniform> u: U;
@vertex fn vs(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
  return u.vp * vec4<f32>(pos, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.34, 0.36, 0.40, 1.0); }
"#;

/// Build-plate grid lines on the bed floor (`z = min.z`), ~10mm spacing.
/// Positions only (the grid shader is unlit). Mirrors the scene viewport.
fn grid_verts(min: [f32; 3], max: [f32; 3]) -> Vec<[f32; 3]> {
    let z = min[2];
    let step = 10.0_f32;
    let mut v = Vec::new();
    let nx = ((max[0] - min[0]) / step).ceil() as i32;
    let ny = ((max[1] - min[1]) / step).ceil() as i32;
    for i in 0..=nx {
        let x = min[0] + i as f32 * step;
        v.push([x, min[1], z]);
        v.push([x, max[1], z]);
    }
    for j in 0..=ny {
        let y = min[1] + j as f32 * step;
        v.push([min[0], y, z]);
        v.push([max[0], y, z]);
    }
    v
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Tmpl {
    /// `[ring.x, ring.y, t, cap]` — see the shader.
    local: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Geo {
    start: [f32; 3],
    end: [f32; 3],
    dims: [f32; 2],
    layer: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    vp: [f32; 16],
    lmin: f32,
    lmax: f32,
    _pad: [f32; 2],
}

/// The shared tube cross-section: a `RING`-gon swept t=0→1 with end caps.
/// Identical for every instance; the shader places it per segment.
fn tube_template() -> (Vec<Tmpl>, Vec<u16>) {
    let ring: Vec<(f32, f32)> = (0..RING)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / RING as f32;
            (a.cos(), a.sin())
        })
        .collect();
    let mut v = Vec::new();
    // Side ring verts: [0..RING) at t=0, [RING..2RING) at t=1.
    for &t in &[0.0f32, 1.0] {
        for &(x, y) in &ring {
            v.push(Tmpl { local: [x, y, t, 0.0] });
        }
    }
    // Cap ring verts (own normals): [2RING..3RING) t=0, [3RING..4RING) t=1.
    for &t in &[0.0f32, 1.0] {
        for &(x, y) in &ring {
            v.push(Tmpl { local: [x, y, t, 1.0] });
        }
    }
    let c0 = v.len() as u16;
    v.push(Tmpl { local: [0.0, 0.0, 0.0, 1.0] });
    let c1 = v.len() as u16;
    v.push(Tmpl { local: [0.0, 0.0, 1.0, 1.0] });

    let mut idx = Vec::new();
    let r = RING as u16;
    for k in 0..r {
        let (a, b) = (k, (k + 1) % r);
        // side quad (a, b) at t=0 → (a, b) at t=1
        idx.extend_from_slice(&[a, b, r + b, a, r + b, r + a]);
    }
    for k in 0..r {
        let b = (k + 1) % r;
        // cap fans (cap ring base 2r at t=0, 3r at t=1)
        idx.extend_from_slice(&[c0, 2 * r + k, 2 * r + b]);
        idx.extend_from_slice(&[c1, 3 * r + b, 3 * r + k]);
    }
    (v, idx)
}

/// One instanced draw's GPU buffers: geometry (static) + color (rewritten
/// on color-mode change) + the instance count.
struct InstanceSet {
    geo: wgpu::Buffer,
    col: wgpu::Buffer,
    n: u32,
}

/// Per-handle cached GPU state. Built on first frame for a handle, reused
/// across camera moves / slider scrubs; freed by [`ToolpathRenderer::drop_handle`].
struct GpuToolpath {
    extrusions: Option<InstanceSet>,
    travels: Option<InstanceSet>,
    retractions: Option<InstanceSet>,
    color_key: (ColorMode, Palette),
}

pub struct ToolpathRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipe: wgpu::RenderPipeline,
    grid_pipe: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    template_vb: wgpu::Buffer,
    template_ib: wgpu::Buffer,
    template_n: u32,
    // Bed grid, rebuilt when the extents change.
    grid_vb: Option<wgpu::Buffer>,
    grid_n: u32,
    grid_key: Option<[f32; 6]>,
    cache: HashMap<PreviewHandle, GpuToolpath>,
    // size-dependent targets
    size: (u32, u32),
    color: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bpr: u32,
}

impl ToolpathRenderer {
    pub fn new() -> Self {
        let (device, queue) = crate::viewport_gpu::shared_device();

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("toolpath.uniform"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("toolpath.bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Uniform>() as u64),
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toolpath.bind"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("toolpath.shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("toolpath.pipe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    // 0: template (per-vertex)
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Tmpl>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4],
                    },
                    // 1: geometry (per-instance)
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Geo>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![1 => Float32x3, 2 => Float32x3, 3 => Float32x2, 4 => Float32],
                    },
                    // 2: color (per-instance)
                    wgpu::VertexBufferLayout {
                        array_stride: (3 * std::mem::size_of::<f32>()) as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![5 => Float32x3],
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FMT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // No culling — the tube is closed and depth resolves overlap.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("toolpath.grid.shader"),
            source: wgpu::ShaderSource::Wgsl(GRID_SHADER.into()),
        });
        let grid_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("toolpath.grid.pipe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (3 * std::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FMT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        let (tverts, tidx) = tube_template();
        let template_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("toolpath.template.vb"),
            contents: bytemuck::cast_slice(&tverts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let template_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("toolpath.template.ib"),
            contents: bytemuck::cast_slice(&tidx),
            usage: wgpu::BufferUsages::INDEX,
        });

        let (color, msaa_view, depth_view, readback, padded_bpr) = make_targets(&device, 8, 8);
        ToolpathRenderer {
            device,
            queue,
            pipe,
            grid_pipe,
            ubuf,
            bind,
            template_vb,
            template_ib,
            template_n: tidx.len() as u32,
            grid_vb: None,
            grid_n: 0,
            grid_key: None,
            cache: HashMap::new(),
            size: (0, 0),
            color,
            msaa_view,
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
        let (color, msaa_view, depth_view, readback, padded_bpr) = make_targets(&self.device, w, h);
        self.color = color;
        self.msaa_view = msaa_view;
        self.depth_view = depth_view;
        self.readback = readback;
        self.padded_bpr = padded_bpr;
    }

    fn vbuf(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// (Re)build the bed grid buffer when the extents change.
    fn ensure_grid(&mut self, min: [f32; 3], max: [f32; 3]) {
        let key = [min[0], min[1], min[2], max[0], max[1], max[2]];
        if self.grid_key == Some(key) {
            return;
        }
        let verts = grid_verts(min, max);
        self.grid_n = verts.len() as u32;
        self.grid_vb = Some(self.vbuf("toolpath.grid", bytemuck::cast_slice(&verts)));
        self.grid_key = Some(key);
    }

    /// Per-extrusion colors (one RGB per instance) from the existing encoder,
    /// which yields the same color for both segment vertices — take the first.
    fn ext_colors(&self, geom: &PreviewGeometry, layer_times: &[f32], key: (ColorMode, Palette)) -> Vec<u8> {
        let per_vertex = encode_colors(&geom.extrusions, key.0, key.1, Some(layer_times));
        let mut out = Vec::with_capacity(geom.extrusions.len() * 12);
        for i in 0..geom.extrusions.len() {
            for &c in &per_vertex[i * 6..i * 6 + 3] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }

    fn ext_set(&self, geom: &PreviewGeometry, layer_times: &[f32], key: (ColorMode, Palette)) -> Option<InstanceSet> {
        let s = &geom.extrusions;
        let n = s.len();
        if n == 0 {
            return None;
        }
        let geo: Vec<Geo> = (0..n)
            .map(|i| Geo {
                start: [s.positions[6 * i], s.positions[6 * i + 1], s.positions[6 * i + 2]],
                end: [s.positions[6 * i + 3], s.positions[6 * i + 4], s.positions[6 * i + 5]],
                dims: [s.width[i].max(MIN_DIM_MM), s.height[i].max(MIN_DIM_MM)],
                layer: s.layer_index[2 * i],
            })
            .collect();
        Some(InstanceSet {
            geo: self.vbuf("toolpath.ext.geo", bytemuck::cast_slice(&geo)),
            col: self.vbuf("toolpath.ext.col", &self.ext_colors(geom, layer_times, key)),
            n: n as u32,
        })
    }

    /// Travels: thin tubes at a single flat color. `dims` overrides the IR's
    /// zero so they render as hairlines.
    fn travel_set(&self, s: &SegmentSet) -> Option<InstanceSet> {
        let n = s.len();
        if n == 0 {
            return None;
        }
        let geo: Vec<Geo> = (0..n)
            .map(|i| Geo {
                start: [s.positions[6 * i], s.positions[6 * i + 1], s.positions[6 * i + 2]],
                end: [s.positions[6 * i + 3], s.positions[6 * i + 4], s.positions[6 * i + 5]],
                dims: [TRAVEL_WIDTH_MM, TRAVEL_WIDTH_MM],
                layer: s.layer_index[2 * i],
            })
            .collect();
        let col: Vec<f32> = TRAVEL_RGB.iter().cloned().cycle().take(n * 3).collect();
        Some(InstanceSet {
            geo: self.vbuf("toolpath.travel.geo", bytemuck::cast_slice(&geo)),
            col: self.vbuf("toolpath.travel.col", bytemuck::cast_slice(&col)),
            n: n as u32,
        })
    }

    /// Retractions: short vertical red nubs at the retract point.
    fn retract_set(&self, geom: &PreviewGeometry) -> Option<InstanceSet> {
        let r = &geom.retractions;
        if r.is_empty() {
            return None;
        }
        let geo: Vec<Geo> = r
            .iter()
            .map(|m| Geo {
                start: m.position,
                end: [m.position[0], m.position[1], m.position[2] + RETRACT_HEIGHT_MM],
                dims: [RETRACT_WIDTH_MM, RETRACT_WIDTH_MM],
                layer: m.layer_index as f32,
            })
            .collect();
        let col: Vec<f32> = RETRACT_RGB.iter().cloned().cycle().take(r.len() * 3).collect();
        Some(InstanceSet {
            geo: self.vbuf("toolpath.retr.geo", bytemuck::cast_slice(&geo)),
            col: self.vbuf("toolpath.retr.col", bytemuck::cast_slice(&col)),
            n: r.len() as u32,
        })
    }

    /// Upload (or refresh) the cached GPU buffers for `handle`. Geometry is
    /// built once; a color-mode change rewrites only the extrusion color
    /// buffer, leaving the (large) geometry buffer in place.
    pub fn ensure(&mut self, handle: PreviewHandle, preview: &LoadedPreview, mode: ColorMode, palette: Palette) {
        let key = (mode, palette);
        let layer_times: Vec<f32> = preview.layer_stats.iter().map(|s| s.duration_seconds).collect();
        let geom = &preview.geometry;
        match self.cache.get(&handle) {
            Some(c) if c.color_key == key => {}
            Some(_) => {
                let col = self.ext_colors(geom, &layer_times, key);
                let buf = self.vbuf("toolpath.ext.col", &col);
                let c = self.cache.get_mut(&handle).expect("present");
                if let Some(set) = c.ext_mut() {
                    set.col = buf;
                }
                c.color_key = key;
            }
            None => {
                let gp = GpuToolpath {
                    extrusions: self.ext_set(geom, &layer_times, key),
                    travels: self.travel_set(&geom.travels),
                    retractions: self.retract_set(geom),
                    color_key: key,
                };
                self.cache.insert(handle, gp);
            }
        }
    }

    pub fn drop_handle(&mut self, handle: PreviewHandle) {
        self.cache.remove(&handle);
    }

    /// Render one frame for `handle` and read it back as tight RGBA8.
    pub fn frame(&mut self, handle: PreviewHandle, req: &ToolpathFrameRequest) -> Vec<u8> {
        let (w, h) = (req.width.max(1), req.height.max(1));
        self.resize(w, h);

        let draw_grid = match (req.bed_min, req.bed_max) {
            (Some(min), Some(max)) => {
                self.ensure_grid(min, max);
                self.grid_vb.is_some()
            }
            _ => false,
        };

        let vp = view_proj(w as f32, h as f32, req.az, req.el, req.dist, Vec3::from(req.center));
        let uni = Uniform {
            vp: vp.to_cols_array(),
            lmin: req.layer_min,
            lmax: req.layer_max,
            _pad: [0.0; 2],
        };
        self.queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&uni));

        let color_view = self.color.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toolpath.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&color_view),
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
                multiview_mask: None,
            });
            // Bed grid first (floor), so the toolpath draws over it.
            if draw_grid {
                if let Some(vb) = &self.grid_vb {
                    rp.set_pipeline(&self.grid_pipe);
                    rp.set_bind_group(0, &self.bind, &[]);
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.draw(0..self.grid_n, 0..1);
                }
            }
            if let Some(gp) = self.cache.get(&handle) {
                // Extrusions always; travels / retractions per visibility toggle.
                let mut sets: Vec<&InstanceSet> = Vec::new();
                if let Some(s) = &gp.extrusions {
                    sets.push(s);
                }
                if req.show_travels {
                    if let Some(s) = &gp.travels {
                        sets.push(s);
                    }
                }
                if req.show_retractions {
                    if let Some(s) = &gp.retractions {
                        sets.push(s);
                    }
                }
                if !sets.is_empty() {
                    rp.set_pipeline(&self.pipe);
                    rp.set_bind_group(0, &self.bind, &[]);
                    rp.set_index_buffer(self.template_ib.slice(..), wgpu::IndexFormat::Uint16);
                    rp.set_vertex_buffer(0, self.template_vb.slice(..));
                    for s in sets {
                        rp.set_vertex_buffer(1, s.geo.slice(..));
                        rp.set_vertex_buffer(2, s.col.slice(..));
                        rp.draw_indexed(0..self.template_n, 0, 0..s.n);
                    }
                }
            }
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
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
        read_rgba(&self.device, &self.readback, self.padded_bpr, w, h)
    }
}

impl Default for ToolpathRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuToolpath {
    fn ext_mut(&mut self) -> Option<&mut InstanceSet> {
        self.extrusions.as_mut()
    }
}

/// The extrusion segment under the cursor, honoring the layer window and
/// occlusion. Among the segments whose tube the ray actually pierces (within
/// the tube tolerance), returns the one *nearest along the ray* — the front
/// -most surface, matching what's drawn — not merely the centerline closest
/// to the ray line (which would happily pick a support strand behind the
/// outer wall). Pure CPU over the IR — no GPU. ponytail: O(n) scan; a
/// per-layer spatial index if a 10M-segment print ever lags the hover.
pub fn pick_segment(geom: &PreviewGeometry, ro: Vec3, rd: Vec3, lmin: f32, lmax: f32) -> Option<u32> {
    let s = &geom.extrusions;
    let mut best: Option<(f32, u32)> = None; // (ray param t, index)
    for i in 0..s.len() {
        let layer = s.layer_index[2 * i];
        if layer < lmin - 0.5 || layer > lmax + 0.5 {
            continue;
        }
        let a = Vec3::new(s.positions[6 * i], s.positions[6 * i + 1], s.positions[6 * i + 2]);
        let b = Vec3::new(s.positions[6 * i + 3], s.positions[6 * i + 4], s.positions[6 * i + 5]);
        let (dist, t) = ray_seg_dist(ro, rd, a, b);
        let tol = s.width[i].max(MIN_DIM_MM) * 0.5 + PICK_SLOP_MM;
        if dist <= tol && best.map_or(true, |(bt, _)| t < bt) {
            best = Some((t, i as u32));
        }
    }
    best.map(|(_, i)| i)
}

// ---- Tauri command surface ----------------------------------------------

/// Tauri-managed renderer (lazily created on first frame; wgpu init is ~100ms).
#[derive(Default)]
pub struct ToolpathState(pub Mutex<Option<ToolpathRenderer>>);

#[derive(serde::Deserialize)]
pub struct ToolpathFrameRequest {
    pub handle: PreviewHandle,
    pub width: u32,
    pub height: u32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    /// Inclusive layer-index window (the slider). Instances outside collapse.
    pub layer_min: f32,
    pub layer_max: f32,
    pub color_mode: ColorMode,
    pub palette: Palette,
    pub show_travels: bool,
    pub show_retractions: bool,
    /// Bed extents for the floor grid (mm). `None` skips the grid.
    #[serde(default)]
    pub bed_min: Option<[f32; 3]>,
    #[serde(default)]
    pub bed_max: Option<[f32; 3]>,
}

/// Render the toolpath for `handle` and return tight RGBA8 bytes.
#[tauri::command]
pub fn toolpath_frame(
    state: tauri::State<'_, ToolpathState>,
    registry: tauri::State<'_, Arc<PreviewRegistry>>,
    req: ToolpathFrameRequest,
) -> tauri::ipc::Response {
    let mut guard = state.0.lock().unwrap();
    let r = guard.get_or_insert_with(ToolpathRenderer::new);
    registry.with(req.handle, |p| r.ensure(req.handle, p, req.color_mode, req.palette));
    tauri::ipc::Response::new(r.frame(req.handle, &req))
}

#[derive(serde::Deserialize)]
pub struct ToolpathPickRequest {
    pub handle: PreviewHandle,
    pub width: u32,
    pub height: u32,
    pub x: f32,
    pub y: f32,
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    pub center: [f32; 3],
    pub layer_min: f32,
    pub layer_max: f32,
}

/// Cursor → nearest visible extrusion segment index, or `None`.
#[tauri::command]
pub fn toolpath_pick(
    registry: tauri::State<'_, Arc<PreviewRegistry>>,
    req: ToolpathPickRequest,
) -> Option<u32> {
    let (w, h) = (req.width.max(1) as f32, req.height.max(1) as f32);
    let (ro, rd) = cursor_ray(w, h, req.x, req.y, req.az, req.el, req.dist, Vec3::from(req.center));
    registry
        .with(req.handle, |p| pick_segment(&p.geometry, ro, rd, req.layer_min, req.layer_max))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_indices_are_in_range() {
        let (v, idx) = tube_template();
        assert!(idx.iter().all(|&i| (i as usize) < v.len()));
        // sides (2 tris/ring edge) + 2 caps (1 tri/ring edge) = 4 tris/edge.
        assert_eq!(idx.len(), RING * 4 * 3);
    }

    #[test]
    fn pick_hits_nearest_segment_within_window() {
        // Two stacked horizontal segments; a ray straight down the -Z axis
        // through (5,0) should hit the one in the layer window.
        let mut geom = PreviewGeometry::default();
        for (z, layer) in [(0.2f32, 0u32), (0.4, 1)] {
            geom.extrusions.push(crate::core::preview::ir::Segment {
                start: [0.0, 0.0, z],
                end: [10.0, 0.0, z],
                layer,
                feature: crate::core::gcode::FeatureType::Perimeter,
                speed: 50.0,
                flow: 5.0,
                extrusion_mm: 0.0,
                tool: 0,
                source_line: 0,
                width: 0.45,
                height: 0.2,
            });
        }
        let ro = Vec3::new(5.0, 0.0, 100.0);
        let rd = Vec3::new(0.0, 0.0, -1.0);
        // Window = layer 0 only → must pick segment 0, not the higher one.
        assert_eq!(pick_segment(&geom, ro, rd, 0.0, 0.0), Some(0));
        // Window = layer 1 only → segment 1.
        assert_eq!(pick_segment(&geom, ro, rd, 1.0, 1.0), Some(1));
        // Both layers visible: the ray pierces both, so occlusion decides —
        // the camera at z=100 sees the higher segment (z=0.4, layer 1) first.
        assert_eq!(pick_segment(&geom, ro, rd, 0.0, 1.0), Some(1));
        // A ray that misses both (off to the side) → None.
        let miss = Vec3::new(50.0, 50.0, 100.0);
        assert_eq!(pick_segment(&geom, miss, rd, 0.0, 1.0), None);
    }
}
