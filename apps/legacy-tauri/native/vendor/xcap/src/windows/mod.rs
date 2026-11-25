mod capture;
#[cfg(not(feature = "wgc"))]
mod dxgi_video_recorder;
#[cfg(not(feature = "wgc"))]
mod gdi;
mod registry_build;
mod utils;
#[cfg(feature = "wgc")]
mod wgc;
#[cfg(feature = "wgc")]
mod wgc_frame_slot;
#[cfg(feature = "wgc")]
mod wgc_init;
#[cfg(feature = "wgc")]
pub(crate) mod wgc_region_reader;
#[cfg(feature = "wgc")]
mod wgc_runtime;
#[cfg(feature = "wgc")]
mod wgc_video_recorder;

pub mod impl_monitor;
pub mod impl_video_recorder;
pub mod impl_window;
