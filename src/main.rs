extern crate core;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::RwLock;

use anyhow::bail;
use clap::Parser;
use palette::{convert::FromColorUnclamped, FromColor, Mix, Shade};
use smithay_client_toolkit::{
    compositor::CompositorHandler,
    compositor::CompositorState,
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{calloop::EventLoop, calloop_wayland_source::WaylandSource},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    backend::ObjectId,
    globals::registry_queue_init,
    protocol::{
        wl_output::{Transform, WlOutput},
        wl_shm,
        wl_surface::WlSurface,
    },
    Connection, Proxy, QueueHandle,
};

pub mod upower;

#[derive(Copy, Clone, Debug)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Default for Side {
    fn default() -> Self {
        Self::Bottom
    }
}

impl Side {
    fn is_horizontal(self) -> bool {
        match self {
            Self::Top | Self::Bottom => true,
            Self::Left | Self::Right => false,
        }
    }

    fn compute_size(self, size: i32, (w, h): (i32, i32)) -> (i32, i32, i32, i32) {
        match self {
            Side::Top => (0, 0, w, size),
            Side::Bottom => (0, h - size, w, size),
            Side::Left => (0, 0, size, h),
            Side::Right => (w - size, 0, size, h),
        }
    }

    #[allow(unused)]
    fn ccw(self) -> Self {
        match self {
            Side::Top => Side::Left,
            Side::Left => Side::Bottom,
            Side::Bottom => Side::Right,
            Side::Right => Side::Top,
        }
    }

    #[allow(unused)]
    fn cw(self) -> Self {
        match self {
            Side::Top => Side::Right,
            Side::Right => Side::Bottom,
            Side::Bottom => Side::Left,
            Side::Left => Side::Top,
        }
    }
}

impl From<Side> for Anchor {
    fn from(value: Side) -> Self {
        match value {
            Side::Top => Anchor::TOP,
            Side::Bottom => Anchor::BOTTOM,
            Side::Left => Anchor::LEFT,
            Side::Right => Anchor::RIGHT,
        }
    }
}

impl FromStr for Side {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "b" | "bottom" => Ok(Self::Bottom),
            "t" | "top" => Ok(Self::Top),
            "l" | "left" => Ok(Self::Left),
            "r" | "right" => Ok(Self::Right),
            _ => bail!("Invalid side. Expected left, right, top, or bottom (or one of l,r,t,b)"),
        }
    }
}

#[derive(clap::Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct CliOptions {
    /// Which border to draw the bar on. One of left, right, top, or bottom (or l,r,t, or b)
    #[arg(short, long, default_value = "bottom")]
    border: Side,
    /// How many virtual pixels tall the bar should be
    #[arg(short, long, default_value_t = 3)]
    size: u32,
    /// Reverse the direction of the bar (i.e., right-to-left, or top-to-bottom)
    #[arg(short, long)]
    reverse: bool,

    /// Debugging aid to simply animate the bar.
    #[arg(long, hide = true)]
    mock_upower: bool,
}

#[derive(Copy, Clone, Debug)]
pub struct PowerState {
    /// Level, between 0 and 1
    level: f32,
    /// True if line power is available.
    charging: bool,
    /// Time to full charge/empty, in seconds
    #[allow(unused)] // TODO: actually use this to display the time remaining
    time_remaining: f32,
}

pub struct AppState {
    display_status: Arc<RwLock<Option<PowerState>>>,
    // Map from output ID to surface
    surfaces: HashMap<ObjectId, BarSurface>,
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    cli: CliOptions,
}

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum RenderEvent {
    Closed,
    Configure {
        size: Option<(u32, u32)>,
        scale: Option<i32>,
    },
    DataChanged,
}

pub struct BarSurface {
    layer_surface: LayerSurface,
    next_render_event: Option<RenderEvent>,
    side: Side,
    reverse: bool,
    scale: i32,
    dimensions: (u32, u32), // in raw pixels

    current_dimensions: (u32, u32),
    current_scale: i32,
    display_status: Arc<RwLock<Option<PowerState>>>,
    pub pool: SlotPool,
}

impl BarSurface {
    fn new(
        _output: &WlOutput,
        layer_surface: LayerSurface,
        pool: SlotPool,
        state: &AppState,
    ) -> Self {
        let side = state.cli.border;
        // layer_surface.set_anchor(Anchor::from(side) | Anchor::from(side.ccw()) | side.cw().into());
        layer_surface.set_anchor(Anchor::from(side));
        let next_render_event = None;

        let result = BarSurface {
            layer_surface,
            next_render_event,
            scale: 1,
            pool,
            side,
            reverse: state.cli.reverse,
            dimensions: (0, 0),
            current_dimensions: (0, 0),
            current_scale: 1,
            display_status: Arc::clone(&state.display_status),
        };

        result
    }

    fn handle_events(&mut self) -> bool {
        match self.next_render_event.take() {
            Some(RenderEvent::Closed) => true,
            Some(RenderEvent::Configure { size, scale }) => {
                self.scale = scale.unwrap_or(self.scale);
                self.dimensions = size.unwrap_or(self.dimensions);
                self.resize();
                self.draw();
                false
            }
            Some(RenderEvent::DataChanged) => {
                self.draw();
                false
            }
            None => false,
        }
    }

    /// Adjust the size of the surface. Returns true if a draw should be performed.
    fn resize(&mut self) -> bool {
        if self.dimensions.0 == 0 || self.dimensions.1 == 0 {
            return false;
        }

        let ret = self.current_dimensions != self.dimensions || self.current_scale != self.scale;
        if !ret {
            return false; // nothing to change
        }
        // eprintln!("Setting size to {:?}", self.dimensions);
        if self
            .layer_surface
            .set_buffer_scale(self.scale as u32)
            .is_err()
        {
            self.scale = self.current_scale;
        }
        if self.dimensions != self.current_dimensions {
            self.layer_surface
                .set_size(self.dimensions.0, self.dimensions.1);
        }
        // eprintln!("Committing layer surface {}", self.layer_surface.wl_surface().id());
        self.layer_surface.commit();
        let ret = self.current_dimensions != self.dimensions || self.current_scale != self.scale;

        self.current_scale = self.scale;
        self.current_dimensions = self.dimensions;
        return ret;
    }

    fn schedule_event(&mut self, event: RenderEvent) {
        match (self.next_render_event, event) {
            (_, RenderEvent::Closed) => self.next_render_event = Some(RenderEvent::Closed),
            (Some(RenderEvent::Closed), _) => {}
            (
                Some(RenderEvent::Configure {
                    size: osize,
                    scale: oscale,
                }),
                RenderEvent::Configure { size, scale },
            ) => {
                self.next_render_event = Some(RenderEvent::Configure {
                    size: size.or(osize),
                    scale: scale.or(oscale),
                });
            }
            (_, RenderEvent::Configure { .. }) => self.next_render_event = Some(event),
            (Some(RenderEvent::Configure { .. }), _) => {}
            (_, RenderEvent::DataChanged) => {
                self.next_render_event = Some(RenderEvent::DataChanged)
            }
        }
    }

    // Returns fg, bg
    fn compute_color(&self, charging: bool, level: f32) -> ([u8; 4], [u8; 4]) {
        let fg_color = if !charging {
            let min_color = palette::Oklab::from_color_unclamped(palette::LinSrgb::new(1., 0., 0.));
            let max_color = palette::Oklab::from_color_unclamped(palette::LinSrgb::new(0., 1., 0.));
            min_color.mix(&max_color, level)
        } else {
            palette::Oklab::from_color_unclamped(palette::Srgb::new(0., 0.5, 1.))
        };

        let bg_color = fg_color.darken(0.5);

        let to_u32 = |color| {
            palette::LinSrgba::from_color(color)
                .into_encoding::<palette::encoding::Srgb>()
                .into_format::<u8, u8>()
                .into_u32::<palette::rgb::channels::Argb>()
                .to_le_bytes()
        };

        let fg_color = to_u32(fg_color);
        let bg_color = to_u32(bg_color);
        (fg_color, bg_color)
    }

    fn draw(&mut self) {
        self.resize();
        let surface = self.layer_surface.wl_surface();
        if self.dimensions.0 == 0 || self.dimensions.1 == 0 {
            return;
        }

        let state = self.display_status.read().map_or(None, |lock| lock.clone());

        let (charging, pct) = if let Some(state) = state {
            (state.charging, state.level)
        } else {
            (true, 0.5)
        };

        let (fg_color, bg_color) = self.compute_color(charging, pct);

        let (pct, fg_color, bg_color) = if self.reverse {
            (1. - pct, bg_color, fg_color)
        } else {
            (pct, fg_color, bg_color)
        };

        let width = self.current_dimensions.0 as i32 * self.scale;
        let height = self.current_dimensions.1 as i32 * self.scale;
        let stride = 4 * width;
        let (buffer, canvas) = self
            .pool
            .create_buffer(width, height, stride, wl_shm::Format::Argb8888)
            .unwrap();

        if self.side.is_horizontal() {
            let fill_width = (width as f32 * pct) as usize * 4;
            for row in canvas.chunks_exact_mut(stride as usize) {
                // println!("Filling ..{}", fill_width);
                row[..fill_width]
                    .chunks_exact_mut(4)
                    .for_each(|chunk| chunk.copy_from_slice(fg_color.as_slice()));
                row[fill_width..]
                    .chunks_exact_mut(4)
                    .for_each(|chunk| chunk.copy_from_slice(bg_color.as_slice()));
            }
        } else {
            let fill_height = ((height as f32 * (1. - pct)) as usize).clamp(0, height as usize - 1);
            let (bg_part, fg_part) = canvas.split_at_mut(stride as usize * fill_height);
            debug_assert!(
                bg_part.len() % stride as usize == 0,
                "vertical split was not an integer number of rows"
            );
            bg_part
                .chunks_exact_mut(4)
                .for_each(|chunk| chunk.copy_from_slice(bg_color.as_slice()));
            fg_part
                .chunks_exact_mut(4)
                .for_each(|chunk| chunk.copy_from_slice(fg_color.as_slice()));
        }

        surface.attach(Some(buffer.wl_buffer()), 0, 0);
        surface.damage_buffer(0, 0, width, height);
        // eprintln!("Committing WL surface");
        surface.commit();
    }
}

impl Drop for BarSurface {
    fn drop(&mut self) {
        // self.layer_surface.destroy();
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        let surface = self.compositor.create_surface(qh);
        let info = self
            .output_state
            .info(&output)
            .expect("No info for new output");
        let mode = info
            .modes
            .iter()
            .find(|mode| mode.current)
            .expect("Output should have a mode");
        let (_x, _y, w, h) = self
            .cli
            .border
            .compute_size(self.cli.size as i32 * info.scale_factor, mode.dimensions);
        let buf_sz = (w * h * 4) as usize;
        let pool = SlotPool::new(buf_sz, &self.shm).expect("Failed to create a backing store");
        let layer_surface = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Bottom,
            Some("WattBar"),
            Some(&output),
        );

        // eprintln!("Allocated surface {} on {}", layer_surface.wl_surface().id(), output.id());

        let mut surface = BarSurface::new(&output, layer_surface, pool, &self);
        surface
            .layer_surface
            .set_buffer_scale(info.scale_factor as u32)
            .unwrap();
        surface.scale = info.scale_factor;
        surface.current_scale = surface.scale;
        surface.layer_surface.set_size(
            w as u32 / info.scale_factor as u32,
            h as u32 / info.scale_factor as u32,
        );
        surface
            .layer_surface
            .set_exclusive_zone(self.cli.size as i32);
        surface.layer_surface.commit();
        _conn.flush().unwrap();
        self.surfaces.insert(output.id(), surface);
    }

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        if let Some(surface) = self.surfaces.get_mut(&output.id()) {
            let info = self
                .output_state
                .info(&output)
                .expect("No info for new output");
            let mode = info
                .modes
                .iter()
                .find(|mode| mode.current)
                .expect("Output should have a mode");
            let (_x, _y, w, h) = self
                .cli
                .border
                .compute_size(self.cli.size as i32 * info.scale_factor, mode.dimensions);

            if surface
                .layer_surface
                .set_buffer_scale(info.scale_factor as u32)
                .is_ok()
            {
                surface.scale = info.scale_factor;
            }
            surface.current_scale = surface.scale;
            surface.layer_surface.set_size(
                w as u32 / surface.scale as u32,
                h as u32 / surface.scale as u32,
            );
        }
    }

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: WlOutput) {
        if let Some(surface) = self.surfaces.get_mut(&output.id()) {
            surface.schedule_event(RenderEvent::Closed);
        }
    }
}

impl ShmHandler for AppState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        // eprintln!("Window closed");
        _conn.flush().unwrap();
        let bar = self
            .surfaces
            .values_mut()
            .find_map(|surface| (&surface.layer_surface == layer).then_some(surface));

        if let Some(surface) = bar {
            surface.schedule_event(RenderEvent::Closed)
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // eprintln!("Received configure event for {}: {:?}", layer.wl_surface().id(), configure);
        _conn.flush().unwrap();
        let bar = self
            .surfaces
            .values_mut()
            .find_map(|surface| (&surface.layer_surface == layer).then_some(surface));

        if let Some(surface) = bar {
            surface.schedule_event(RenderEvent::Configure {
                size: Some((configure.new_size.0, configure.new_size.1)),
                scale: None,
            });
            // surface.handle_events();
            _conn.flush().unwrap()
        }
    }
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        let bar = self
            .surfaces
            .values_mut()
            .find_map(|bar| (bar.layer_surface.wl_surface() == surface).then_some(bar));
        if let Some(bar) = bar {
            bar.schedule_event(RenderEvent::Configure {
                size: None,
                scale: Some(new_factor),
            })
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: Transform,
    ) {
        // We do nothing with this
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _time: u32,
    ) {
        let bar = self
            .surfaces
            .values_mut()
            .find_map(|bar| (bar.layer_surface.wl_surface() == surface).then_some(bar));
        if let Some(bar) = bar {
            bar.draw()
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli: CliOptions = CliOptions::parse();
    let display_status = Arc::new(Default::default());

    // Spawn upower watcher
    let upower_channel = {
        let (sender, channel) = calloop::channel::channel();
        let reporter = upower::PowerReporter {
            sender,
            status: Arc::clone(&display_status),
        };

        if cli.mock_upower {
            upower::spawn_mock(reporter)?;
        } else {
            upower::spawn_upower(reporter)?;
        }
        channel
    };

    // connect to wayland
    let conn = Connection::connect_to_env()?;
    // enumerate the list of globals
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let registry_state = RegistryState::new(&globals);

    let qh = event_queue.handle();
    let mut event_loop: EventLoop<AppState> =
        EventLoop::try_new().expect("Failed to initialize the event loop");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .unwrap();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1 not available");
    let shm = Shm::bind(&globals, &qh).expect("wl shm not available");
    let output_state = OutputState::new(&globals, &qh);
    // TODO: add code to spawn windows per output

    // List surfaces
    let mut app_state = AppState {
        display_status,
        surfaces: Default::default(),
        registry_state,
        output_state,
        compositor,
        layer_shell,
        shm,
        cli,
    };

    event_loop
        .handle()
        .insert_source(upower_channel, move |_evt, _evt_md, app_state| {
            // eprintln!("Power state: {:?}", &*power_state_handle.read().unwrap());
            for (_, surface) in app_state.surfaces.iter_mut() {
                surface.schedule_event(RenderEvent::DataChanged);
                surface.handle_events();
            }
        })
        .unwrap();

    loop {
        event_loop.dispatch(None, &mut app_state).unwrap();
        // eprintln!("Finished event loop");
        {
            let surfaces = &mut app_state.surfaces;
            let to_remove = surfaces
                .iter_mut()
                .filter_map(|(oid, bar)| bar.handle_events().then_some(oid.clone()))
                .collect::<Vec<_>>();
            to_remove.into_iter().for_each(|oid| {
                surfaces.remove(&oid);
            });
        }
    }

    //println!("Registry: {:#?}", env);
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_shm!(AppState);
delegate_layer!(AppState);
delegate_registry!(AppState);

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState,];
}
