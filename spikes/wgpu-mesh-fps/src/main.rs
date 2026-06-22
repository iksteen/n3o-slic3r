//! Headless wgpu render-fps at several triangle counts — the decisive measure of
//! whether the GPU rasterizes a representative slicer scene fast enough that
//! wgpu+GtkGLArea would beat WebKitGTK's *software* WebGL on the Intel iGPU.
//!
//! Renders a lit, depth-tested UV sphere (a stand-in for a print model) to an
//! offscreen target, rotating the camera, and times GPU completion per frame
//! (submit + poll(Wait)). No window, no readback per frame — pure raster
//! throughput. The GtkGLArea present is a small additive cost on top (already
//! shown to coexist with the webview); raster throughput is the question.

use glam::{Mat4, Vec3};
use std::time::Instant;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

fn uv_sphere(target_tris: u32) -> (Vec<Vertex>, Vec<u32>) {
    // tris = stacks*slices*2 → pick stacks=slices=sqrt(target/2)
    let n = ((target_tris as f32 / 2.0).sqrt().round() as u32).max(2);
    let (stacks, slices) = (n, n);
    let mut verts = Vec::new();
    for i in 0..=stacks {
        let phi = std::f32::consts::PI * i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = 2.0 * std::f32::consts::PI * j as f32 / slices as f32;
            let (x, y, z) = (phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            verts.push(Vertex { pos: [x, y, z], nrm: [x, y, z] });
        }
    }
    let row = slices + 1;
    let mut idx = Vec::new();
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * row + j;
            let b = a + row;
            idx.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (verts, idx)
}

const SHADER: &str = r#"
struct U { vp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> VO {
  var o: VO; o.p = u.vp * vec4<f32>(pos, 1.0); o.n = nrm; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
  let l = normalize(vec3<f32>(0.5, 0.7, 1.0));
  let d = max(dot(normalize(i.n), l), 0.0) * 0.8 + 0.2;
  return vec4<f32>(vec3<f32>(0.7, 0.75, 0.85) * d, 1.0);
}
"#;

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
        .expect("no adapter");
    let info = adapter.get_info();
    println!("adapter: {} | {:?} | {:?}", info.name, info.backend, info.device_type);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .expect("device");

    let (w, h) = (1920u32, 1080u32);
    let color_fmt = wgpu::TextureFormat::Rgba8Unorm;
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: color_fmt,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&Default::default());

    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0, visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None, bind_group_layouts: &[&bgl], push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None, layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader, entry_point: "vs",
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader, entry_point: "fs",
            compilation_options: Default::default(),
            targets: &[Some(color_fmt.into())],
        }),
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(), bias: Default::default(),
        }),
        multisample: Default::default(), multiview: None,
    });

    let proj = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.1, 100.0);

    println!("{:<10} {:>10} {:>10} {:>9}", "tris", "verts", "ms/frame", "fps");
    for target in [100_000u32, 1_000_000, 4_000_000] {
        let (verts, idx) = uv_sphere(target);
        let real_tris = idx.len() / 3;
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX,
        });

        let warmup = 10u32;
        let frames = 120u32;
        let mut total = std::time::Duration::ZERO;
        for f in 0..(warmup + frames) {
            let ang = f as f32 * 0.03;
            let eye = Vec3::new(ang.cos() * 3.0, 1.2, ang.sin() * 3.0);
            let vp = proj * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
            queue.write_buffer(&ubuf, 0, bytemuck::cast_slice(&vp.to_cols_array()));

            let t0 = Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.12, a: 1.0 }), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None, occlusion_query_set: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &bg, &[]);
                rp.set_vertex_buffer(0, vb.slice(..));
                rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..idx.len() as u32, 0, 0..1);
            }
            queue.submit(Some(enc.finish()));
            device.poll(wgpu::Maintain::Wait); // block until the GPU finished this frame
            if f >= warmup {
                total += t0.elapsed();
            }
        }
        let ms = total.as_secs_f64() / frames as f64 * 1000.0;
        println!("{:<10} {:>10} {:>10.2} {:>9.0}", real_tris, verts.len(), ms, 1000.0 / ms);
    }
    println!("(1080p offscreen, lit + depth-tested, camera orbiting; GPU-completion timed per frame)");
}
