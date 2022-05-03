pub mod upower;

use std::cell::Cell;
use std::sync::RwLock;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use wayland_client::{
    protocol::{wl_output::WlOutput, wl_shm, wl_surface::WlSurface},
    Attached, Main,
};

use wayland_protocols::wlr::unstable::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use smithay_client_toolkit::{
    default_environment, environment::SimpleGlobal, new_default_environment,
    output::with_output_info, output::OutputInfo, shm::AutoMemPool, WaylandSource,
};

#[derive(Copy, Clone, Debug)]
pub struct PowerState {
    /// Level, between 0 and 1
    level: f32,
    /// True if line power is available.
    charging: bool,
    /// Time to full charge/empty, in seconds
    time_remaining: f32,
}

#[derive(Default, Clone)]
pub struct AppState {
    display_status: Arc<RwLock<Option<PowerState>>>,
}

default_environment! {
    MyEnv,
    fields = [
        layer_shell: SimpleGlobal<ZwlrLayerShellV1>,
    ],
    singles = [
        ZwlrLayerShellV1 => layer_shell,
    ],
}

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum RenderEvent {
    Closed,
    Configure { width: u32, height: u32 },
    DataChanged,
}

pub struct Surface {
    surface: WlSurface,
    layer_surface: Main<ZwlrLayerSurfaceV1>,
    next_render_event: Rc<Cell<Option<RenderEvent>>>,
    pool: AutoMemPool,
    dimensions: (u32, u32),
    display_status: Arc<RwLock<Option<PowerState>>>,
}

impl Surface {
    fn new(
        output: &WlOutput,
        surface: WlSurface,
        layer_shell: &Attached<ZwlrLayerShellV1>,
        pool: AutoMemPool,
	state: &AppState,
    ) -> Self {
        let layer_surface: Main<ZwlrLayerSurfaceV1> = layer_shell.get_layer_surface(
            &surface,
            Some(output),
            zwlr_layer_shell_v1::Layer::Bottom,
            "WattBar".to_owned(),
        );

        layer_surface.set_size(32, 32);
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::Bottom);
        layer_surface.set_exclusive_zone(3);
        let next_render_event = Rc::new(Cell::new(None));
        let nre_handle = Rc::clone(&next_render_event);

        layer_surface.quick_assign(move |layer_surface, event, _| {
            match (event, nre_handle.get()) {
                (zwlr_layer_surface_v1::Event::Closed, _) => {
                    nre_handle.set(Some(RenderEvent::Closed));
                }
                (
                    zwlr_layer_surface_v1::Event::Configure {
                        serial,
                        width,
                        height,
                    },
                    next,
                ) if next != Some(RenderEvent::Closed) => {
                    layer_surface.ack_configure(serial);
                    nre_handle.set(Some(RenderEvent::Configure { width, height }));
                }
                (_, _) => {}
            }
        });

        surface.commit();
        Surface {
            surface,
            layer_surface,
            next_render_event,
            pool,
            dimensions: (0, 0),
	    display_status: Arc::clone(&state.display_status),
        }
    }

    fn handle_events(&mut self) -> bool {
        match self.next_render_event.take() {
            Some(RenderEvent::Closed) => true,
            Some(RenderEvent::Configure { width, height }) => {
                if self.dimensions != (width, height) {
                    self.dimensions = (width, height);
                    self.draw();
                }
                false
            }
	    Some(RenderEvent::DataChanged) => {
		self.draw();
		false
	    }
            None => false,
        }
    }

    fn draw(&mut self) {
        let stride = 4 * self.dimensions.0 as i32;
        let width = self.dimensions.0 as i32;
        let height = self.dimensions.1 as i32;

        let (canvas, buffer) = self
            .pool
            .buffer(width, height, stride, wl_shm::Format::Argb8888)
            .unwrap();
        for dst_pixel in canvas.chunks_exact_mut(4) {
            let pixel = 0x01_00_8f_00u32.to_ne_bytes();
            dst_pixel[0] = pixel[0];
            dst_pixel[1] = pixel[1];
            dst_pixel[2] = pixel[2];
            dst_pixel[3] = pixel[3];
        }

        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.layer_surface.destroy();
        self.surface.destroy();
    }
}

fn main() -> anyhow::Result<()> {

    let mut app_state = AppState::default();

    // Spawn upower watcher
    let upower_channel = {
	let (sender, channel) = calloop::channel::channel();
	let reporter = upower::PowerReporter {
	    sender,
	    status: Arc::clone(&app_state.display_status),
	};

	upower::spawn_upower(reporter)?;
	channel
    };
    
    let (env, display, queue) =
        new_default_environment!(MyEnv, fields = [layer_shell: SimpleGlobal::new(),],)?;

    let env_handle = env.clone();

    let layer_shell = env.require_global::<ZwlrLayerShellV1>();

    // List surfaces
    let surfaces = Rc::new(RefCell::new(Vec::new()));

    let surfaces_handle = Rc::clone(&surfaces);
    let app_state_handle = app_state.clone();
    let output_handler = move |output: WlOutput, info: &OutputInfo| {
        if info.obsolete {
            surfaces_handle.borrow_mut().retain(|(i, _)| *i != info.id);
            output.release();
        } else {
            let surface = env_handle.create_surface().detach();
            let pool = env_handle
                .create_auto_pool()
                .expect("Failed to create a memeory pool!");
            surfaces_handle.borrow_mut().push((
                info.id,
                Surface::new(&output, surface, &layer_shell.clone(), pool, &app_state_handle),
            ));
        }
    };

    // Process currently existing outputs
    for output in env.get_all_outputs() {
        if let Some(info) = with_output_info(&output, Clone::clone) {
            output_handler(output, &info);
        }
    }

    let _listener_handle =
        env.listen_for_outputs(move |output, info, _| output_handler(output, info));
    let mut event_loop = calloop::EventLoop::<()>::try_new().expect("Failed to start event loop");

    let surfaces_handle = Rc::clone(&surfaces);
    event_loop.handle().insert_source(
	upower_channel,
	move |_, _, _| {
	    for (_, surface) in surfaces_handle.borrow_mut().iter() {
		if surface.next_render_event.get().is_none() {
		    surface.next_render_event.set(Some(RenderEvent::DataChanged));
		}
	    }
	}
    ).unwrap();

    WaylandSource::new(queue)
        .quick_insert(event_loop.handle())
        .unwrap();
    loop {
        {
            let mut surfaces = surfaces.borrow_mut();
            let mut i = 0;
            while i != surfaces.len() {
                if surfaces[i].1.handle_events() {
                    surfaces.remove(i);
                } else {
                    i += 1;
                }
            }
        }

        display.flush().unwrap();
        event_loop.dispatch(None, &mut ()).unwrap();
    }

    
    //println!("Registry: {:#?}", env);
}
