use super::DesktopRect;
use anyhow::{ensure, Context, Result};
use smithay_client_toolkit::{
    delegate_output, delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{
    globals::{registry_queue_init, GlobalList},
    protocol::wl_output::{Transform, WlOutput},
    Connection, EventQueue, QueueHandle,
};

pub(super) struct Registry {
    pub connection: Connection,
    globals: GlobalList,
    queue: EventQueue<Outputs>,
}

impl Registry {
    pub fn connect() -> Result<Self> {
        let connection = Connection::connect_to_env().context("Cannot open Wayland display")?;
        let (globals, queue) = registry_queue_init(&connection)?;
        Ok(Self {
            connection,
            globals,
            queue,
        })
    }

    pub fn has(&self, interface: &str, version: u32) -> bool {
        self.globals.contents().with_list(|globals| {
            globals
                .iter()
                .any(|global| global.interface == interface && global.version >= version)
        })
    }

    pub fn displays(mut self) -> Result<Vec<Display>> {
        let mut state = Outputs {
            registry: RegistryState::new(&self.globals),
            outputs: OutputState::new(&self.globals, &self.queue.handle()),
        };
        self.queue.roundtrip(&mut state)?;
        let outputs = state.outputs.outputs().collect::<Vec<_>>();
        let single_output = outputs.len() == 1;
        outputs
            .into_iter()
            .map(|output| {
                let info = state
                    .outputs
                    .info(&output)
                    .context("Output has no geometry")?;
                Display::new(info, single_output)
            })
            .collect()
    }
}

pub(super) struct Display {
    pub name: String,
    pub geometry: DesktopRect,
    pub size: (u32, u32),
    pub transform: Transform,
}

impl Display {
    fn new(info: OutputInfo, single_output: bool) -> Result<Self> {
        let mode = info
            .modes
            .iter()
            .find(|mode| mode.current)
            .context("Output has no active mode")?;
        let size = (
            u32::try_from(mode.dimensions.0)?,
            u32::try_from(mode.dimensions.1)?,
        );
        // 安全上下文可能隐藏 xdg-output。单屏允许用 wl_output 的比例，多屏需要明确的逻辑范围。
        ensure!(
            single_output || (info.logical_position.is_some() && info.logical_size.is_some()),
            "Cannot match the shared display without Wayland logical geometry"
        );
        let (x, y) = info.logical_position.unwrap_or(info.location);
        let (width, height) = match info.logical_size {
            Some(size) => size,
            None => {
                ensure!(info.scale_factor > 0, "Wayland output has an invalid scale");
                (
                    mode.dimensions.0 / info.scale_factor,
                    mode.dimensions.1 / info.scale_factor,
                )
            }
        };
        Ok(Self {
            name: info.name.context("Wayland output has no name")?,
            geometry: DesktopRect {
                x,
                y,
                width: u32::try_from(width)?,
                height: u32::try_from(height)?,
            },
            size,
            transform: info.transform,
        })
    }
}

struct Outputs {
    registry: RegistryState,
    outputs: OutputState,
}

impl OutputHandler for Outputs {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl ProvidesRegistryState for Outputs {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers!(OutputState);
}

delegate_registry!(Outputs);
delegate_output!(Outputs);
