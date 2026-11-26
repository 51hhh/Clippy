use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use image::RgbaImage;
use scopeguard::{ScopeGuard, guard};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem},
        DirectX::Direct3D11::IDirect3DDevice,
        SizeInt32,
    },
    Win32::{
        Graphics::{
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::IDXGIDevice,
            Gdi::HMONITOR,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
    },
    core::{IInspectable, Interface, factory},
};

use crate::{XCapError, XCapResult};

use super::{
    utils::{create_d3d_device, texture_to_frame},
    wgc_frame_slot::FrameSlot,
    wgc_video_recorder::WgcRuntime,
};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_UPDATE_WAIT: Duration = Duration::from_millis(16);
type NativeFrame = ScopeGuard<Direct3D11CaptureFrame, fn(Direct3D11CaptureFrame)>;

struct ReceivedFrame {
    frame: NativeFrame,
    observed_at: Instant,
}

pub struct WgcRegionFrame {
    pub pixels: RgbaImage,
    pub system_time_100ns: Option<i64>,
    pub observed_at: Instant,
}

/// A monitor-local region read from one Windows Graphics Capture session.
pub struct WgcRegionReader {
    _monitor_closed: MonitorClosed,
    runtime: WgcRuntime,
    pending: Arc<FrameSlot<ReceivedFrame>>,
    last: Option<WgcRegionFrame>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    pixels: [u32; 4],
}

impl WgcRegionReader {
    pub(crate) fn open(
        monitor: HMONITOR,
        source_size: (u32, u32),
        pixels: [u32; 4],
    ) -> XCapResult<Self> {
        check_region(source_size, pixels)?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item = unsafe { interop.CreateForMonitor::<GraphicsCaptureItem>(monitor)? };
        let size = item.Size()?;
        check_size(size, source_size)?;

        // 为持续流保留独立 immediate context，避免与同时触发的普通截图跨线程共享。
        let device = create_d3d_device(D3D11_CREATE_DEVICE_BGRA_SUPPORT)?;
        let context = unsafe { device.GetImmediateContext()? };
        let dxgi = device.cast::<IDXGIDevice>()?;
        let winrt_device: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;

        let pending = Arc::new(FrameSlot::default());
        let closed = MonitorClosed::register(&item, &pending)?;
        let on_frame = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new({
            let pending = pending.clone();
            move |pool, _| {
                if pending.check_open().is_ok() {
                    let result = pool
                        .as_ref()
                        .ok_or_else(|| XCapError::new("WGC frame pool is unavailable"))
                        .and_then(|pool| next_frame(pool, source_size));
                    match result {
                        Ok(Some(frame)) => pending.publish(ReceivedFrame {
                            frame,
                            observed_at: Instant::now(),
                        }),
                        Ok(None) => {}
                        Err(error) => pending.close(error.to_string()),
                    }
                }
                Ok(())
            }
        });
        let runtime = WgcRuntime::new(&item, size, &winrt_device, &on_frame)?;
        if let Err(error) = runtime.session.SetIsCursorCaptureEnabled(false) {
            log::debug!("WGC could not disable the cursor: {error}");
        }
        let reader = Self {
            _monitor_closed: closed,
            runtime,
            pending,
            last: None,
            device,
            context,
            pixels,
        };
        reader.runtime.session.StartCapture()?;
        Ok(reader)
    }

    pub fn read(&mut self) -> XCapResult<RgbaImage> {
        self.read_sample(false)
            .map(|sample| sample.expect("WGC 最新帧读取保留静止画面").pixels.clone())
    }

    pub fn read_sample(&mut self, new_only: bool) -> XCapResult<Option<&WgcRegionFrame>> {
        let timeout = if self.last.is_some() {
            FRAME_UPDATE_WAIT
        } else {
            FIRST_FRAME_TIMEOUT
        };
        let update = self.pending.take(timeout).map_err(XCapError::new)?;
        let fresh = update.is_some();
        if let Some(ReceivedFrame { frame, observed_at }) = update {
            let system_time_100ns = frame.SystemRelativeTime().ok().map(|span| span.Duration);
            match self.read_pixels(&frame) {
                Ok(pixels) => {
                    self.last = Some(WgcRegionFrame {
                        pixels,
                        system_time_100ns,
                        observed_at,
                    });
                }
                Err(error) => {
                    self.last = None;
                    self.pending.close(error.to_string());
                    return Err(error);
                }
            }
        }
        self.pending.check_open().map_err(XCapError::new)?;
        let last = self
            .last
            .as_ref()
            .ok_or_else(|| XCapError::new("Timed out waiting for the first WGC frame"))?;
        Ok((fresh || !new_only).then_some(last))
    }

    fn read_pixels(&self, frame: &Direct3D11CaptureFrame) -> XCapResult<RgbaImage> {
        let access = frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?;
        let texture = unsafe { access.GetInterface::<ID3D11Texture2D>()? };
        let [left, top, right, bottom] = self.pixels;
        let frame = texture_to_frame(
            &self.device,
            &self.context,
            &texture,
            left,
            top,
            right - left,
            bottom - top,
        )?;
        Ok(RgbaImage::from_raw(frame.width, frame.height, frame.raw)
            .expect("WGC readback returns tightly packed RGBA pixels"))
    }
}

impl Drop for WgcRegionReader {
    fn drop(&mut self) {
        self.pending.close("WGC region capture stopped".into());
    }
}

struct MonitorClosed {
    item: GraphicsCaptureItem,
    token: i64,
}

impl MonitorClosed {
    fn register(
        item: &GraphicsCaptureItem,
        pending: &Arc<FrameSlot<ReceivedFrame>>,
    ) -> XCapResult<Self> {
        let pending = pending.clone();
        let token = item.Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                pending.close("WGC display source closed".into());
                Ok(())
            }),
        )?;
        Ok(Self {
            item: item.clone(),
            token,
        })
    }
}

impl Drop for MonitorClosed {
    fn drop(&mut self) {
        if let Err(error) = self.item.RemoveClosed(self.token) {
            log::debug!("WGC could not remove display close handler: {error}");
        }
    }
}

fn next_frame(
    pool: &Direct3D11CaptureFramePool,
    source_size: (u32, u32),
) -> XCapResult<Option<NativeFrame>> {
    let frame = match pool.TryGetNextFrame() {
        Ok(frame) => frame,
        // windows-rs 把成功返回的空 WinRT 接口包装为状态码 S_OK 的空错误。
        Err(error) if error.code().is_ok() => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let frame: NativeFrame = guard(frame, |frame| {
        if let Err(error) = frame.Close() {
            log::debug!("WGC frame close failed: {error}");
        }
    });
    check_size(frame.ContentSize()?, source_size)?;
    Ok(Some(frame))
}

fn check_size(size: SizeInt32, (width, height): (u32, u32)) -> XCapResult<()> {
    if (
        u32::try_from(size.Width).ok(),
        u32::try_from(size.Height).ok(),
    ) != (Some(width), Some(height))
    {
        return Err(XCapError::new(format!(
            "WGC display size changed: expected {width}x{height}, received {}x{}",
            size.Width, size.Height
        )));
    }
    Ok(())
}

fn check_region((width, height): (u32, u32), pixels: [u32; 4]) -> XCapResult<()> {
    let [left, top, right, bottom] = pixels;
    if left >= right || top >= bottom || right > width || bottom > height {
        return Err(XCapError::InvalidCaptureRegion(format!(
            "{pixels:?} is outside {width}x{height}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_smaller_or_rotated_source_even_when_the_region_still_fits() {
        let expected = (3840, 2160);
        assert!(check_region(expected, [300, 400, 540, 670]).is_ok());
        assert!(
            check_size(
                SizeInt32 {
                    Width: 3840,
                    Height: 2160
                },
                expected
            )
            .is_ok()
        );
        assert!(
            check_size(
                SizeInt32 {
                    Width: 1920,
                    Height: 1080
                },
                expected
            )
            .is_err()
        );
        assert!(
            check_size(
                SizeInt32 {
                    Width: 2160,
                    Height: 3840
                },
                expected
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_an_empty_or_outside_region() {
        let expected = (3840, 2160);
        assert!(check_region(expected, [300, 400, 300, 670]).is_err());
        assert!(check_region(expected, [300, 400, u32::MAX, 670]).is_err());
        assert!(check_region(expected, [0, 0, 3840, 2160]).is_ok());
    }
}
