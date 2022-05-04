extern crate core;

pub mod upower;

use std::cell::Cell;
use std::sync::RwLock;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use std::cmp::min;
use std::os::linux::raw::stat;
use palette::convert::FromColorUnclamped;
use palette::{Blend, FromColor, IntoColor, LinSrgba, Mix, Oklab, Oklaba, Packed, Pixel, Shade, Srgb, Srgba};
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
use smithay_client_toolkit::output::Mode;

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
    output: WlOutput,
    layer_surface: Main<ZwlrLayerSurfaceV1>,
    next_render_event: Rc<Cell<Option<RenderEvent>>>,
    pool: AutoMemPool,
    mode: Option<Mode>,
    scale: i32,
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


        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::Bottom);
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

        let mut result = Surface {
            surface,
            output: output.clone(),
            layer_surface,
            next_render_event,
            mode: None,
            scale: 1,
            pool,
            dimensions: (0, 0),
            display_status: Arc::clone(&state.display_status),
        };
        result.resize();
        result.surface.commit();

        result
    }

    fn resize(&mut self) {
        with_output_info(&self.output, |info| {
            let mode = info.modes.iter().find(|mode| (*mode).is_current).cloned();
            if self.mode.map(|mode| mode.dimensions) == mode.map(|mode| mode.dimensions) && self.scale == info.scale_factor {
                return;
            }
            // eprintln!("Output {} mode: {:?}, scale: {}", info.name, mode, info.scale_factor);
            if let Some(mode) = mode {
                self.layer_surface.set_size((mode.dimensions.0 / info.scale_factor) as u32, 3);
                self.layer_surface.set_exclusive_zone(3);
                self.scale = info.scale_factor;
            }
        });

    }

    fn handle_events(&mut self) -> bool {
        self.resize(); // There's probably a better way of doing this, but this isn't going to cost too much
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
        if self.dimensions.0 == 0 || self.dimensions.1 == 0 {
            return;
        }
        let stride = 4 * self.dimensions.0 as i32;
        let width = self.dimensions.0 as i32;
        let height = self.dimensions.1 as i32;

        let (canvas, buffer) = self
            .pool
            .buffer(width, height, stride, wl_shm::Format::Argb8888)
            .unwrap();

        let state = self.display_status.read().map_or(None, |lock| lock.clone());

        let (base_color, pct) = if let Some(state) = state {
            let mix_color = if !state.charging {
                let min_color = Oklaba::from_color_unclamped(palette::LinSrgba::new(1., 0., 0., 1.));
                let max_color = Oklaba::from_color_unclamped(palette::LinSrgba::new(0., 1., 0., 1.));
                min_color.mix(&max_color, state.level)
            } else {
                Oklaba::from_color_unclamped(Srgba::new(0., 0.5, 1., 1.0f32))
            };

            (mix_color, state.level)
        } else {
            let color = Oklaba::from_color_unclamped(Srgba::new(0., 0.5, 1., 1.0f32));
            let pct = 0.5;
            (color, pct)
        };

        let bg_color = base_color.darken(0.5);

        let to_u32 = |color| {
            LinSrgba::from_color(color).into_encoding::<palette::encoding::Srgb>().into_format::<u8,u8>().into_u32::<palette::rgb::channels::Argb>().to_le_bytes()
        } ;


        let fg_color = to_u32(base_color);
        let bg_color = to_u32(bg_color);
        // eprintln!("Colors: {:?}/{:?}", fg_color, bg_color);

        // let pct = pct * 0.75 + 0.125;
        // blit the buffer
        let fill_width = (width as f32 * pct) as usize * 4;
        for row in canvas.chunks_exact_mut(stride as usize) {
            // println!("Filling ..{}", fill_width);
            row[..fill_width].chunks_exact_mut(4).for_each(|chunk| chunk.copy_from_slice(fg_color.as_slice()));
            row[fill_width..].chunks_exact_mut(4).for_each(|chunk| chunk.copy_from_slice(bg_color.as_slice()));
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
        // upower::spawn_mock(reporter)?;
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

            // output.
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
    let power_state_handle = Arc::clone(&app_state.display_status);
    event_loop.handle().insert_source(
        upower_channel,
        move |_, _, _| {
            // eprintln!("Power state: {:?}", &*power_state_handle.read().unwrap());
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
