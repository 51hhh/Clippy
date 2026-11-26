use crate::capture::Region;
use anyhow::Result;

#[cfg(target_os = "linux")]
mod x11;

#[derive(Clone, Copy)]
pub enum Direction {
    Down,
    Up,
    Left,
    Right,
}

pub trait Target {
    fn control_is_hidden(&self, window: u32) -> Result<bool>;
    fn position_pointer(&mut self) -> Result<()>;
    fn check(&self) -> Result<()>;
    fn step(&self, direction: Direction) -> Result<()>;
    fn finish(&mut self, restore_pointer: bool) -> Result<()>;
}

pub fn is_x11() -> bool {
    cfg!(target_os = "linux")
        && !crate::session::is_wayland()
        && std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
}

pub fn for_region(_region: &Region) -> Result<Option<Box<dyn Target>>> {
    #[cfg(target_os = "linux")]
    if is_x11() {
        let rect = crate::capture::x11_region_rect(_region.geometry, _region.bounds)?;
        return Ok(x11::Target::open(rect)?.map(|target| Box::new(target) as Box<dyn Target>));
    }
    Ok(None)
}
