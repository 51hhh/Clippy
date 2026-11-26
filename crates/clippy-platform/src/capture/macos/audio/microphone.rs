use super::super::os_at_least;
use crate::capture::RegionCancellation;
use anyhow::{ensure, Context, Result};
use block2::RcBlock;
use objc2::{rc::autoreleasepool, rc::Retained, runtime::Bool};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureDevice, AVCaptureDeviceDiscoverySession,
    AVCaptureDevicePosition, AVMediaType, AVMediaTypeAudio,
};
use objc2_foundation::{NSArray, NSBundle, NSString};
use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

pub fn microphone_available() -> bool {
    os_at_least(15, 0)
}

pub struct MicrophoneDevice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

pub fn enumerate_microphones() -> Result<Vec<MicrophoneDevice>> {
    if !microphone_available() {
        return Ok(Vec::new());
    }
    autoreleasepool(|_| {
        let media_type = audio_media_type()?;
        let default_id = unsafe { AVCaptureDevice::defaultDeviceWithMediaType(media_type) }
            .map(|device| unsafe { device.uniqueID() });
        Ok(devices(media_type)?
            .iter()
            .take(64)
            .map(|device| {
                let id = unsafe { device.uniqueID() };
                MicrophoneDevice {
                    is_default: default_id.as_deref() == Some(&*id),
                    id: id.to_string(),
                    label: unsafe { device.localizedName() }.to_string(),
                }
            })
            .collect())
    })
}

pub fn request_microphone_permission(cancellation: &RegionCancellation) -> Result<()> {
    ensure!(
        microphone_available(),
        "Microphone capture needs macOS 15 or later"
    );
    cancellation.check_recording_start()?;
    let media_type = audio_media_type()?;
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    if status == AVAuthorizationStatus::Authorized {
        return Ok(());
    }
    ensure!(
        status == AVAuthorizationStatus::NotDetermined,
        "Allow microphone access for Clippy in System Settings"
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    autoreleasepool(|_| {
        let description = NSBundle::mainBundle()
            .objectForInfoDictionaryKey(&NSString::from_str("NSMicrophoneUsageDescription"))
            .and_then(|value| value.downcast::<NSString>().ok());
        ensure!(
            description.is_some_and(|value| value.length() > 0),
            "Open Clippy from its macOS app bundle to allow microphone access"
        );
        let reply = RcBlock::new(move |allowed: Bool| {
            let _ = sender.try_send(allowed.as_bool());
        });
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &reply) };
        Ok(())
    })?;
    wait_for_permission(&receiver, cancellation)
}

fn wait_for_permission(receiver: &Receiver<bool>, cancellation: &RegionCancellation) -> Result<()> {
    loop {
        cancellation.check_recording_start()?;
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(allowed) => {
                cancellation.check_recording_start()?;
                ensure!(
                    allowed,
                    "Allow microphone access for Clippy in System Settings"
                );
                return Ok(());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Microphone permission request closed")
            }
        }
    }
}

pub(super) fn resolve(id: Option<&str>) -> Result<Retained<AVCaptureDevice>> {
    ensure!(
        microphone_available(),
        "Microphone capture needs macOS 15 or later"
    );
    let media_type = audio_media_type()?;
    let device = match id {
        Some(id) => {
            let id = NSString::from_str(id);
            devices(media_type)?
                .to_vec()
                .into_iter()
                .find(|device| *unsafe { device.uniqueID() } == *id)
                .context("The selected microphone is unavailable")?
        }
        None => unsafe { AVCaptureDevice::defaultDeviceWithMediaType(media_type) }
            .context("The default microphone is unavailable")?,
    };
    Ok(device)
}

pub(super) fn check_permission() -> Result<()> {
    ensure!(
        unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio_media_type()?) }
            == AVAuthorizationStatus::Authorized,
        "Allow microphone access for Clippy in System Settings"
    );
    Ok(())
}

fn audio_media_type() -> Result<&'static AVMediaType> {
    unsafe { AVMediaTypeAudio }.context("AVFoundation audio media type unavailable")
}

fn devices(media_type: &AVMediaType) -> Result<Retained<NSArray<AVCaptureDevice>>> {
    // 该常量从 macOS 14 起提供。运行时查询避免在 macOS 11–13 启动时加载不存在的符号。
    let process = libloading::os::unix::Library::this();
    let symbol =
        unsafe { process.get::<*const *const NSString>(b"AVCaptureDeviceTypeMicrophone\0") }
            .context("Finding AVFoundation microphone device type")?;
    let microphone = unsafe { (**symbol).as_ref() }
        .context("AVFoundation microphone device type unavailable")?;
    let types = NSArray::from_slice(&[microphone]);
    Ok(unsafe {
        AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
            &types,
            Some(media_type),
            AVCaptureDevicePosition::Unspecified,
        )
        .devices()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_wait_respects_denial_and_cancelled_approval() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancellation = RegionCancellation::default();
        sender.send(false).unwrap();
        assert!(wait_for_permission(&receiver, &cancellation).is_err());
        sender.send(true).unwrap();
        wait_for_permission(&receiver, &cancellation).unwrap();
        cancellation.clone().cancel();
        sender.send(true).unwrap();
        assert!(wait_for_permission(&receiver, &cancellation).is_err());
    }

    #[test]
    fn lists_devices_on_supported_macos_without_prompting() {
        let devices = enumerate_microphones().unwrap();
        if microphone_available() {
            assert!(devices.iter().all(|device| !device.id.is_empty()));
        } else {
            assert!(devices.is_empty());
        }
    }
}
