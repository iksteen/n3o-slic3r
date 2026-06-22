//! Strategy B PoC — render wgpu to a NATIVE Tauri window surface and put a
//! webview child in the same window (vs Strategy A's offscreen+copy).
//!
//! Layout under test: a bare `Window` (no default webview). wgpu clears its
//! surface MAGENTA. A child webview (green panel) is placed over the RIGHT
//! half. If we see magenta-left + green-right, stably, native GPU content and
//! the webview coexist in one window. The literature predicts this FAILS on
//! Linux/WebKitGTK/Wayland (GTK and wgpu are different rendering systems);
//! this PoC records what actually happens on this machine.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl};

struct GpuState {
    // keep the window alive for the 'static surface
    _window: tauri::Window,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

type SharedGpu = std::sync::Mutex<Option<GpuState>>;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = tauri::window::WindowBuilder::new(app, "main")
                .title("native wgpu PoC")
                .inner_size(1100.0, 700.0)
                .build()?;

            // What kind of native handle does Tauri give us on this platform?
            match win.window_handle() {
                Ok(h) => eprintln!("[poc] window handle: {:?}", h.as_raw()),
                Err(e) => eprintln!("[poc] window_handle ERR: {e}"),
            }
            match win.display_handle() {
                Ok(h) => eprintln!("[poc] display handle: {:?}", h.as_raw()),
                Err(e) => eprintln!("[poc] display_handle ERR: {e}"),
            }

            let size = win.inner_size()?;
            let (w, h) = (size.width.max(1), size.height.max(1));

            match pollster::block_on(init_wgpu(win.clone(), w, h)) {
                Ok(state) => {
                    render_clear(&state);
                    app.manage::<SharedGpu>(std::sync::Mutex::new(Some(state)));
                    eprintln!("[poc] wgpu init + first present OK");
                }
                Err(e) => eprintln!("[poc] wgpu init/render FAILED: {e}"),
            }

            // Child webview over the right half of the same window.
            let wv = tauri::webview::WebviewBuilder::new(
                "panel",
                WebviewUrl::App("index.html".into()),
            );
            let (wf, hf) = (w as f64, h as f64);
            match win.add_child(
                wv,
                LogicalPosition::new(wf * 0.5, 0.0),
                LogicalSize::new(wf * 0.5, hf),
            ) {
                Ok(_) => eprintln!("[poc] add_child webview (right half) OK"),
                Err(e) => eprintln!("[poc] add_child FAILED: {e}"),
            }

            // Re-render the magenta region whenever the window resizes.
            let win2 = win.clone();
            win.on_window_event(move |ev| {
                if let tauri::WindowEvent::Resized(_) = ev {
                    if let Some(state) = win2.app_handle().try_state::<SharedGpu>() {
                        if let Ok(mut g) = state.lock() {
                            if let Some(s) = g.as_mut() {
                                if let Ok(sz) = win2.inner_size() {
                                    s.config.width = sz.width.max(1);
                                    s.config.height = sz.height.max(1);
                                    s.surface.configure(&s.device, &s.config);
                                    render_clear(s);
                                }
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}

async fn init_wgpu(
    window: tauri::Window,
    width: u32,
    height: u32,
) -> Result<GpuState, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| format!("create_surface: {e}"))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .ok_or("no adapter compatible with the Tauri window surface")?;
    eprintln!("[poc] adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .map_err(|e| format!("request_device: {e}"))?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: caps.present_modes[0],
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);
    Ok(GpuState { _window: window, surface, device, queue, config })
}

fn render_clear(s: &GpuState) {
    let frame = match s.surface.get_current_texture() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[poc] get_current_texture: {e:?}");
            return;
        }
    };
    let view = frame.texture.create_view(&Default::default());
    let mut enc = s.device.create_command_encoder(&Default::default());
    drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.0, b: 1.0, a: 1.0 }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    }));
    s.queue.submit(Some(enc.finish()));
    frame.present();
    eprintln!("[poc] presented magenta frame");
}
