use super::{super::thumbnail::MAX_DECODE_PIXELS, PlaybackUpdate};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSink;
use gstreamer_video as video;
use image::RgbaImage;
use std::path::Path;
use video::prelude::*;

pub struct RecordingPlayer {
    pipeline: gst::Element,
    sink: AppSink,
    bus: gst::Bus,
    paused: bool,
    pending_seek: Option<gst::ClockTime>,
}

impl RecordingPlayer {
    pub fn open(path: &Path) -> Result<Self, String> {
        gst::init().map_err(|error| error.to_string())?;
        let uri = gst::glib::filename_to_uri(path, None).map_err(|error| error.to_string())?;
        let webm = path.extension().is_some_and(|ext| ext == "webm");
        let format = if webm { "I420" } else { "RGBA" };
        let sink = AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", format)
                    .build(),
            )
            .max_buffers(1)
            .drop(true)
            .enable_last_sample(false)
            .wait_on_eos(false)
            .build();
        let flags = if webm {
            "video+audio+native-video"
        } else {
            "video+audio"
        };
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri)
            .property("video-sink", &sink)
            .property_from_str("flags", flags)
            .build()
            .map_err(|error| error.to_string())?;
        let bus = pipeline.bus().expect("playbin 持有一条消息总线");
        let player = Self {
            pipeline,
            sink,
            bus,
            paused: false,
            pending_seek: None,
        };
        player.set_state(gst::State::Playing)?;
        Ok(player)
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), String> {
        if self.paused != paused {
            self.set_state(if paused {
                gst::State::Paused
            } else {
                gst::State::Playing
            })?;
            self.paused = paused;
        }
        Ok(())
    }

    pub fn seek(&mut self, millis: u64) -> Result<(), String> {
        let target = gst::ClockTime::from_mseconds(millis);
        self.pipeline
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, target)
            .map_err(|error| error.to_string())?;
        self.pending_seek = Some(target);
        Ok(())
    }

    pub fn poll(&mut self) -> Result<PlaybackUpdate, String> {
        let timeout_ms = if self.paused && self.pending_seek.is_none() {
            40
        } else {
            12
        };
        let message = self.bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(timeout_ms),
            &[gst::MessageType::Error, gst::MessageType::Eos],
        );
        let ended = match message.as_ref().map(|message| message.view()) {
            Some(gst::MessageView::Error(error)) => return Err(error.error().to_string()),
            Some(gst::MessageView::Eos(_)) => true,
            _ => false,
        };
        let sample = if self.paused && self.pending_seek.is_some() {
            self.sink.try_pull_preroll(gst::ClockTime::ZERO)
        } else if !self.paused {
            self.sink.try_pull_sample(gst::ClockTime::ZERO)
        } else {
            None
        };
        let frame = if let Some(sample) = sample.filter(|sample| {
            self.pending_seek.is_none_or(|target| {
                sample
                    .buffer()
                    .and_then(|buffer| buffer.pts())
                    .is_some_and(|pts| pts >= target)
            })
        }) {
            self.pending_seek = None;
            Some(frame_rgba(&sample)?)
        } else {
            None
        };
        let position_ms = self
            .pending_seek
            .or_else(|| self.pipeline.query_position::<gst::ClockTime>())
            .map_or(0, |position| position.mseconds());
        let duration_ms = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0, |duration| duration.mseconds());
        Ok(PlaybackUpdate {
            duration_ms,
            position_ms,
            frame,
            ended,
        })
    }

    fn set_state(&self, state: gst::State) -> Result<(), String> {
        self.pipeline
            .set_state(state)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

impl Drop for RecordingPlayer {
    fn drop(&mut self) {
        if let Err(error) = self.set_state(gst::State::Null) {
            log::warn!("关闭录屏播放管线失败: {error}");
        }
    }
}

fn frame_rgba(sample: &gst::Sample) -> Result<RgbaImage, String> {
    let caps = sample.caps().ok_or("Playback frame has no video format")?;
    let buffer = sample.buffer().ok_or("Playback frame has no pixels")?;
    let info = video::VideoInfo::from_caps(caps).map_err(|error| error.to_string())?;
    let (width, height) = (info.width(), info.height());
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_DECODE_PIXELS {
        return Err("Playback frame exceeds the pixel budget".into());
    }
    let frame = video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|error| error.to_string())?;
    let plane = |index| frame.plane_data(index).map_err(|error| error.to_string());
    let stride = |index: usize| {
        u32::try_from(frame.plane_stride()[index]).map_err(|error| error.to_string())
    };
    let rgba = match frame.format() {
        video::VideoFormat::I420 => {
            let src = yuv::YuvPlanarImage {
                y_plane: plane(0)?,
                y_stride: stride(0)?,
                u_plane: plane(1)?,
                u_stride: stride(1)?,
                v_plane: plane(2)?,
                v_stride: stride(2)?,
                width,
                height,
            };
            let mut rgba = vec![0; pixels as usize * 4];
            // 已有 Clippy WebM 按 BT.709 限幅编码，容器未声明色彩矩阵。
            yuv::yuv420_to_rgba(
                &src,
                &mut rgba,
                width * 4,
                yuv::YuvRange::Limited,
                yuv::YuvStandardMatrix::Bt709,
            )
            .map_err(|error| error.to_string())?;
            rgba
        }
        video::VideoFormat::Rgba => {
            let row_bytes = width as usize * 4;
            let stride = stride(0)? as usize;
            if stride < row_bytes {
                return Err("Invalid playback pixel stride".into());
            }
            let mut rgba = Vec::with_capacity(pixels as usize * 4);
            for row in plane(0)?.chunks(stride).take(height as usize) {
                rgba.extend_from_slice(row.get(..row_bytes).ok_or("Truncated playback pixel row")?);
            }
            rgba
        }
        _ => return Err("Unsupported playback pixel format".into()),
    };
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "Invalid playback pixels".into())
}
