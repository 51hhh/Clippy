/* SPDX-License-Identifier: Apache-2.0 OR MIT */

use crate::{Error, ImageData};
use image::{DynamicImage, ImageDecoder};

// 与 Clippy watcher 的后置布局校验一致；此处先保护整图像素分配。
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 40_000_000;

fn check_dimensions(width: u32, height: u32) -> Result<(), Error> {
	if width == 0
		|| height == 0
		|| width > MAX_DIMENSION
		|| height > MAX_DIMENSION
		|| u64::from(width) * u64::from(height) > MAX_PIXELS
	{
		return Err(Error::ConversionFailure);
	}
	Ok(())
}

pub(super) fn png_decoder(
	data: &[u8],
) -> image::ImageResult<image::codecs::png::PngDecoder<std::io::Cursor<&[u8]>>> {
	// PNG 在 read_info 前先检查单边尺寸；总像素在下面的共用解码入口检查。
	// 不把 RGBA8 的字节数当作 16-bit 解码/元数据的总内存预算。
	let mut limits = image::Limits::no_limits();
	limits.max_image_width = Some(MAX_DIMENSION);
	limits.max_image_height = Some(MAX_DIMENSION);
	image::codecs::png::PngDecoder::with_limits(std::io::Cursor::new(data), limits)
}

pub(super) fn decode(decoder: impl ImageDecoder) -> Result<ImageData<'static>, Error> {
	let (width, height) = decoder.dimensions();
	check_dimensions(width, height)?;
	let bytes = DynamicImage::from_decoder(decoder)
		.map_err(|_| Error::ConversionFailure)?
		.into_rgba8()
		.into_raw();
	Ok(ImageData { width: width as usize, height: height as usize, bytes: bytes.into() })
}

#[cfg(test)]
mod tests {
	use super::*;
	use image::{codecs::png::PngEncoder, ColorType, ExtendedColorType, ImageEncoder, ImageResult};
	use std::cell::Cell;

	struct ObservedDecoder<'a> {
		dimensions: (u32, u32),
		reads: &'a Cell<usize>,
	}

	impl ImageDecoder for ObservedDecoder<'_> {
		fn dimensions(&self) -> (u32, u32) {
			self.dimensions
		}
		fn color_type(&self) -> ColorType {
			ColorType::Rgba8
		}
		// 故障注入始终只分配四字节，旧路径失败也不会尝试超大分配。
		fn total_bytes(&self) -> u64 {
			4
		}
		fn read_image(self, bytes: &mut [u8]) -> ImageResult<()> {
			self.reads.set(self.reads.get() + 1);
			bytes.copy_from_slice(&[11, 22, 33, 128]);
			Ok(())
		}
		fn read_image_boxed(self: Box<Self>, bytes: &mut [u8]) -> ImageResult<()> {
			(*self).read_image(bytes)
		}
	}

	#[test]
	fn invalid_dimensions_never_decode_pixels() {
		for dimensions in [(0, 1), (1, 0), (16_385, 1), (1, 16_385), (u32::MAX, u32::MAX)] {
			let reads = Cell::new(0);
			assert!(decode(ObservedDecoder { dimensions, reads: &reads }).is_err());
			assert_eq!(reads.get(), 0, "异常尺寸不得进入像素解码: {dimensions:?}");
		}
	}

	#[test]
	fn excess_pixels_never_decode_pixels() {
		for dimensions in [(8_000, 5_001), (10_000, 5_000), (16_384, 16_384)] {
			let reads = Cell::new(0);
			assert!(decode(ObservedDecoder { dimensions, reads: &reads }).is_err());
			assert_eq!(reads.get(), 0, "超限像素不得进入解码: {dimensions:?}");
		}
	}

	#[test]
	fn metadata_accepts_4k_8k_and_exact_boundaries() {
		for (width, height) in [(3840, 2160), (7680, 4320), (8000, 5000), (16_384, 1), (1, 16_384)]
		{
			assert!(check_dimensions(width, height).is_ok());
		}
	}

	#[test]
	fn png_constructor_checks_dimensions_before_read_info() {
		let mut png = Vec::new();
		PngEncoder::new(&mut png).write_image(&[0; 4], 1, 1, ExtendedColorType::Rgba8).unwrap();
		// 保留有效 IHDR（含 CRC），删除后续块，确保尺寸错误先于缺失 IDAT。
		png.truncate(33);
		png[16..20].copy_from_slice(&16_385u32.to_be_bytes());
		let mut crc = u32::MAX;
		for byte in &png[12..29] {
			crc ^= u32::from(*byte);
			for _ in 0..8 {
				crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
			}
		}
		png[29..33].copy_from_slice(&(!crc).to_be_bytes());
		assert!(matches!(png_decoder(&png), Err(image::ImageError::Limits(_))));
		assert!(super::super::image_data::read_png(&png).is_err());
	}

	#[test]
	fn valid_decoder_preserves_dimensions_and_rgba() {
		let reads = Cell::new(0);
		let image = decode(ObservedDecoder { dimensions: (1, 1), reads: &reads }).unwrap();
		assert_eq!((image.width, image.height), (1, 1));
		assert_eq!(&*image.bytes, &[11, 22, 33, 128]);
		assert_eq!(reads.get(), 1);
	}

	#[test]
	fn png_rgba8_keeps_all_alpha_values() {
		let pixels = [11, 22, 33, 0, 44, 55, 66, 128, 77, 88, 99, 255];
		let mut png = Vec::new();
		PngEncoder::new(&mut png).write_image(&pixels, 3, 1, ExtendedColorType::Rgba8).unwrap();
		let image = super::super::image_data::read_png(&png).unwrap();
		assert_eq!((image.width, image.height), (3, 1));
		assert_eq!(&*image.bytes, &pixels);
	}

	#[test]
	fn png_rgba16_keeps_existing_conversion() {
		let pixels: Vec<u8> = [
			0x1111u16, 0x2222, 0x3333, 0, 0xaaaa, 0xbbbb, 0xcccc, 0x8080, 0xdddd, 0xeeee, 0xffff,
			0xffff,
		]
		.into_iter()
		.flat_map(u16::to_ne_bytes)
		.collect();
		let mut png = Vec::new();
		PngEncoder::new(&mut png).write_image(&pixels, 3, 1, ExtendedColorType::Rgba16).unwrap();
		let expected = image::load_from_memory(&png).unwrap().into_rgba8().into_raw();
		let image = super::super::image_data::read_png(&png).unwrap();
		assert_eq!((image.width, image.height), (3, 1));
		assert_eq!(&*image.bytes, &expected);
		assert_eq!([image.bytes[3], image.bytes[7], image.bytes[11]], [0, 128, 255]);
	}
}
