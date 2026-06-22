//! wgpu → GtkGLArea, zero-copy, interactive. Renders an STL sitting on a build
//! plate; left-drag orbits, scroll zooms. The point is to FEEL whether
//! wgpu-on-the-iGPU presented through a GtkGLArea is smooth on Intel — the
//! production Linux present path, minus the webview (coexistence is proven in
//! the gtk-glarea spike).
//!
//! Bridge: wgpu takes over the GLArea's *own* GL context (gles backend,
//! `Adapter::new_external`), renders the scene into an offscreen wgpu texture,
//! then a raw `glBlitFramebuffer` copies that texture into the framebuffer GTK
//! bound for the GLArea. No CPU readback — the texture never leaves the GPU.

use std::cell::RefCell;
use std::rc::Rc;

use glam::{Mat4, Vec3};
use gtk::gdk;
use gtk::prelude::*;
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
  return vec4<f32>(vec3<f32>(0.82, 0.72, 0.45) * d, 1.0); // warm model
}
"#;

/// Flat-shaded triangle soup from a binary/ascii STL, plus its XY footprint and
/// the min-Z (so we can drop it onto z=0) and a center to orbit around.
struct Model {
    verts: Vec<Vertex>,
    center: Vec3,
    footprint: f32, // max of x/y extent
}

fn load_stl(path: &str) -> Model {
    let mut f = std::fs::File::open(path).expect("open stl");
    let mesh = stl_io::read_stl(&mut f).expect("parse stl");
    let mut verts = Vec::with_capacity(mesh.faces.len() * 3);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for face in &mesh.faces {
        let n = face.normal;
        let nrm = [n[0], n[1], n[2]];
        for &vi in &face.vertices {
            let v = mesh.vertices[vi];
            let p = Vec3::new(v[0], v[1], v[2]);
            lo = lo.min(p);
            hi = hi.max(p);
            verts.push(Vertex { pos: [v[0], v[1], v[2]], nrm });
        }
    }
    // Drop the model onto the plate (min-Z → 0) and center it in XY.
    let shift = Vec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, lo.z);
    for v in &mut verts {
        v.pos[0] -= shift.x;
        v.pos[1] -= shift.y;
        v.pos[2] -= shift.z;
    }
    let size = hi - lo;
    Model {
        verts,
        center: Vec3::new(0.0, 0.0, size.z * 0.5),
        footprint: size.x.max(size.y),
    }
}

/// Build-plate grid lines on z=0, sized to ~1.5× the model footprint so the
/// model and the plate frame together regardless of model scale. Unlit (normal
/// = 0 → flagged in the shader).
fn grid(footprint: f32) -> Vec<Vertex> {
    let half = (footprint * 0.75).max(20.0);
    let step = (half * 2.0 / 12.0).max(1.0);
    let z = 0.0;
    let mut v = Vec::new();
    let mut x = -half;
    while x <= half + 0.001 {
        v.push(Vertex { pos: [x, -half, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [x, half, z], nrm: [0.0; 3] });
        x += step;
    }
    let mut y = -half;
    while y <= half + 0.001 {
        v.push(Vertex { pos: [-half, y, z], nrm: [0.0; 3] });
        v.push(Vertex { pos: [half, y, z], nrm: [0.0; 3] });
        y += step;
    }
    v
}

struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    mesh_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
    vb_mesh: wgpu::Buffer,
    n_mesh: u32,
    vb_grid: wgpu::Buffer,
    n_grid: u32,
    // offscreen targets, recreated on resize
    size: (u32, u32),
    color: wgpu::Texture,
    color_gl: u32,
    depth_view: wgpu::TextureView,
    blit_fbo: u32,
    dumped: bool,
}

const COLOR_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Renderer {
    fn new(loader: &dyn Fn(&str) -> *const std::ffi::c_void, model: &Model) -> Renderer {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..Default::default()
        });
        // Adopt the GLArea's *current* GL context as our adapter — no surface,
        // no second context, no readback.
        let exposed = unsafe { wgpu_hal::gles::Adapter::new_external(loader) }
            .expect("wgpu could not adopt the GtkGLArea GL context");
        let adapter = unsafe { instance.create_adapter_from_hal(exposed) };
        let info = adapter.get_info();
        eprintln!("[stl] wgpu adapter: {} | {:?}", info.name, info.backend);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("request_device");

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
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
        let make_pipe = |topology: wgpu::PrimitiveTopology, cull: Option<wgpu::Face>| {
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
        let mesh_pipe = make_pipe(wgpu::PrimitiveTopology::TriangleList, Some(wgpu::Face::Back));
        let line_pipe = make_pipe(wgpu::PrimitiveTopology::LineList, None);

        let vb_mesh = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&model.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let grid_verts = grid(model.footprint);
        let vb_grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&grid_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Real 1×1-ish initial targets so the struct is valid; the first
        // connect_render calls resize() with the actual widget size.
        let (color, color_gl, depth_view) = make_targets(&device, 8, 8);
        Renderer {
            n_mesh: model.verts.len() as u32,
            n_grid: grid_verts.len() as u32,
            device,
            queue,
            ubuf,
            bind,
            mesh_pipe,
            line_pipe,
            vb_mesh,
            vb_grid,
            size: (0, 0), // (0,0) ≠ (8,8) → resize() rebuilds + wires the FBO
            color,
            color_gl,
            depth_view,
            blit_fbo: 0,
            dumped: false,
        }
    }

    /// Copy the offscreen color texture back to the CPU and write a PPM — purely
    /// a debug check that the wgpu render itself produced pixels.
    fn dump_ppm(&self) {
        let (w, h) = self.size;
        let bpr = (w * 4).div_ceil(256) * 256; // 256-byte row alignment
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
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
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            let row = &data[(y * bpr) as usize..];
            for x in 0..w {
                let p = &row[(x * 4) as usize..];
                ppm.extend_from_slice(&[p[0], p[1], p[2]]);
            }
        }
        std::fs::write("/tmp/wgpu-dump.ppm", ppm).unwrap();
        eprintln!("[stl] dumped /tmp/wgpu-dump.ppm ({w}x{h})");
    }

    fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        if self.size == (w, h) {
            return;
        }
        self.size = (w, h);
        let (color, color_gl, depth_view) = make_targets(&self.device, w, h);
        self.color = color;
        self.color_gl = color_gl;
        self.depth_view = depth_view;

        // A GL read-FBO with our wgpu color texture attached, for the blit.
        unsafe {
            if self.blit_fbo == 0 {
                let mut fbo = 0u32;
                epoxy::GenFramebuffers(1, &mut fbo);
                self.blit_fbo = fbo;
            }
            epoxy::BindFramebuffer(epoxy::READ_FRAMEBUFFER, self.blit_fbo);
            epoxy::FramebufferTexture2D(
                epoxy::READ_FRAMEBUFFER,
                epoxy::COLOR_ATTACHMENT0,
                epoxy::TEXTURE_2D,
                self.color_gl,
                0,
            );
            epoxy::BindFramebuffer(epoxy::READ_FRAMEBUFFER, 0);
        }
    }

    /// Render the scene with the given view-projection, then blit into whatever
    /// framebuffer GTK currently has bound (its GLArea target).
    fn frame(&mut self, vp: Mat4) {
        // Capture GTK's GLArea framebuffer NOW — wgpu's submit below rebinds its
        // own FBO on this shared context and won't restore GTK's.
        let mut gtk_fbo = 0i32;
        unsafe { epoxy::GetIntegerv(epoxy::FRAMEBUFFER_BINDING, &mut gtk_fbo) };

        self.queue
            .write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&vp.to_cols_array()));
        let color_view = self.color.create_view(&Default::default());
        let mut enc = self
            .device
            .create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.11,
                            g: 0.12,
                            b: 0.14,
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
            rp.set_pipeline(&self.mesh_pipe);
            rp.set_vertex_buffer(0, self.vb_mesh.slice(..));
            rp.draw(0..self.n_mesh, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        self.device.poll(wgpu::Maintain::Wait);

        // Debug: dump what wgpu actually rendered (separates "did wgpu draw" from
        // "did the GL blit/composite work"). N3O_DUMP=1 → /tmp/wgpu-dump.ppm once.
        if std::env::var("N3O_DUMP").is_ok() && !self.dumped {
            self.dumped = true;
            self.dump_ppm();
        }

        // Blit our texture into GTK's framebuffer, flipping Y (wgpu origin
        // top-left → GL bottom-left). Reset state wgpu may have left that would
        // suppress the blit (scissor on, color mask off).
        let (w, h) = self.size;
        unsafe {
            epoxy::Disable(epoxy::SCISSOR_TEST);
            epoxy::ColorMask(1, 1, 1, 1);
            epoxy::BindFramebuffer(epoxy::READ_FRAMEBUFFER, self.blit_fbo);
            epoxy::BindFramebuffer(epoxy::DRAW_FRAMEBUFFER, gtk_fbo as u32);
            epoxy::BlitFramebuffer(
                0, 0, w as i32, h as i32, // src
                0, h as i32, w as i32, 0, // dst (y-flipped)
                epoxy::COLOR_BUFFER_BIT,
                epoxy::NEAREST,
            );
            // Leave GTK's fbo bound so GTK composites the result.
            epoxy::BindFramebuffer(epoxy::READ_FRAMEBUFFER, gtk_fbo as u32);
            epoxy::BindFramebuffer(epoxy::DRAW_FRAMEBUFFER, gtk_fbo as u32);
        }
    }
}

/// Pull the raw GL texture name out of a wgpu texture (gles backend).
fn gl_texture_id(tex: &wgpu::Texture) -> u32 {
    unsafe {
        tex.as_hal::<wgpu::hal::api::Gles, _, _>(|t| {
            let t = t.expect("gles texture");
            match t.inner {
                wgpu_hal::gles::TextureInner::Texture { raw, .. } => raw.0.get(),
                _ => panic!("expected a GL texture, not a renderbuffer"),
            }
        })
    }
}

/// Create the offscreen color (+ its GL name) and depth view for a given size.
fn make_targets(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, u32, wgpu::TextureView) {
    let ext = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("color"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_gl = gl_texture_id(&color);
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (color, color_gl, depth.create_view(&Default::default()))
}

/// Orbit-camera state owned by the UI (renderer-local, like the real app).
struct Camera {
    az: f32,
    el: f32,
    dist: f32,
    center: Vec3,
}

impl Camera {
    fn vp(&self, w: f32, h: f32) -> Mat4 {
        let (ce, se) = (self.el.cos(), self.el.sin());
        let (ca, sa) = (self.az.cos(), self.az.sin());
        let eye = self.center + self.dist * Vec3::new(ce * ca, ce * sa, se);
        let proj = Mat4::perspective_rh(45f32.to_radians(), w / h, 0.1, self.dist * 10.0);
        proj * Mat4::look_at_rh(eye, self.center, Vec3::Z)
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../../../ocon/m5sticks3_click_case_no_logo.stl".to_string());
    let model = Rc::new(load_stl(&path));
    eprintln!(
        "[stl] {} tris, footprint {:.1}mm",
        model.verts.len() / 3,
        model.footprint
    );

    gtk::init().expect("gtk init");
    load_gl();

    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_title("wgpu → GtkGLArea — orbit the STL");
    win.set_default_size(1000, 800);

    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_has_depth_buffer(false); // wgpu owns its own depth
    area.set_has_stencil_buffer(false);
    area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK,
    );
    win.add(&area);

    let renderer: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
    // initial distance frames the bed (footprint) comfortably
    let cam = Rc::new(RefCell::new(Camera {
        az: 0.9,
        el: 0.6,
        dist: model.footprint.max(20.0) * 2.2,
        center: model.center,
    }));
    let drag: Rc<RefCell<Option<(f64, f64)>>> = Rc::new(RefCell::new(None));

    {
        let renderer = renderer.clone();
        let cam = cam.clone();
        let model = model.clone();
        area.connect_render(move |area, _ctx| {
            let mut slot = renderer.borrow_mut();
            let r = slot.get_or_insert_with(|| Renderer::new(&gl_loader, &model));
            let scale = area.scale_factor();
            let w = (area.allocated_width() * scale).max(1) as u32;
            let h = (area.allocated_height() * scale).max(1) as u32;
            r.resize(w, h);
            r.frame(cam.borrow().vp(w as f32, h as f32));
            gtk::glib::Propagation::Stop
        });
    }

    // left-drag orbit
    {
        let drag = drag.clone();
        area.connect_button_press_event(move |_a, e| {
            if e.button() == 1 {
                *drag.borrow_mut() = Some(e.position());
            }
            gtk::glib::Propagation::Stop
        });
    }
    {
        let drag = drag.clone();
        area.connect_button_release_event(move |_a, _e| {
            *drag.borrow_mut() = None;
            gtk::glib::Propagation::Stop
        });
    }
    {
        let drag = drag.clone();
        let cam = cam.clone();
        area.connect_motion_notify_event(move |a, e| {
            let prev = *drag.borrow(); // copy out, then drop the borrow
            if let Some((px, py)) = prev {
                let (x, y) = e.position();
                let mut c = cam.borrow_mut();
                c.az -= (x - px) as f32 * 0.01;
                c.el = (c.el + (y - py) as f32 * 0.01).clamp(-1.45, 1.45);
                *drag.borrow_mut() = Some((x, y));
                a.queue_render();
            }
            gtk::glib::Propagation::Stop
        });
    }
    // scroll zoom
    {
        let cam = cam.clone();
        area.connect_scroll_event(move |a, e| {
            let mut c = cam.borrow_mut();
            let f = match e.direction() {
                gdk::ScrollDirection::Up => 0.9,
                gdk::ScrollDirection::Down => 1.0 / 0.9,
                _ => {
                    let (_dx, dy) = e.delta();
                    1.0 + dy as f32 * 0.1
                }
            };
            c.dist = (c.dist * f).clamp(model.footprint * 0.2 + 1.0, model.footprint * 20.0 + 10.0);
            a.queue_render();
            gtk::glib::Propagation::Stop
        });
    }

    win.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });
    win.show_all();
    gtk::main();
}

// --- GL loader (libepoxy), shared by epoxy's dispatch and wgpu's new_external ---

thread_local! {
    static EPOXY_LIB: RefCell<Option<libloading::Library>> = RefCell::new(None);
}

fn load_gl() {
    let lib = unsafe { libloading::Library::new("libepoxy.so.0") }.expect("libepoxy.so.0");
    epoxy::load_with(|name| gl_proc(&lib, name));
    // keep the handle alive for the whole process
    EPOXY_LIB.with(|c| *c.borrow_mut() = Some(lib));
}

fn gl_proc(lib: &libloading::Library, name: &str) -> *const std::ffi::c_void {
    unsafe {
        lib.get::<unsafe extern "C" fn()>(name.as_bytes())
            .map(|s| *s as *const std::ffi::c_void)
            .unwrap_or(std::ptr::null())
    }
}

/// Loader handed to wgpu's gles backend. libepoxy exports its GL entry points
/// as `epoxy_*` dispatch pointers, not plain `glFoo` symbols, so a raw dlsym
/// finds nothing — `epoxy::get_proc_addr` returns the (already-loaded) dispatch
/// wrapper, which is what glow needs.
fn gl_loader(name: &str) -> *const std::ffi::c_void {
    epoxy::get_proc_addr(name)
}
