/* SPDX-License-Identifier: Apache-2.0 OR MIT */

use crate::Error;
use image::codecs::bmp::BmpDecoder;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

const FILE_HEADER_SIZE: usize = 14;
const V5_HEADER_SIZE: usize = 124;

// 仅补文件头并借用 DIB；显式 bfOffBits 避免锁定解码器在 V5 bitfields 后多跳掩码字节。
pub(super) struct DibFile<'a> {
	header: [u8; FILE_HEADER_SIZE],
	data: &'a [u8],
	position: u64,
}

impl<'a> DibFile<'a> {
	fn new(data: &'a [u8]) -> Result<Self, Error> {
		if data.len() < V5_HEADER_SIZE || data[..4] != (V5_HEADER_SIZE as u32).to_le_bytes() {
			return Err(Error::ConversionFailure);
		}
		let bit_count = u16::from_le_bytes(data[14..16].try_into().unwrap());
		let colors_used = u32::from_le_bytes(data[32..36].try_into().unwrap());
		let colors = if colors_used != 0 {
			colors_used
		} else if (1..=8).contains(&bit_count) {
			1 << bit_count
		} else {
			0
		};
		let pixel_offset = u64::from(colors) * 4 + V5_HEADER_SIZE as u64;
		if pixel_offset > data.len() as u64 {
			return Err(Error::ConversionFailure);
		}
		let pixel_offset = u32::try_from(pixel_offset + FILE_HEADER_SIZE as u64)
			.map_err(|_| Error::ConversionFailure)?;
		let file_size = u32::try_from(data.len() as u64 + FILE_HEADER_SIZE as u64)
			.map_err(|_| Error::ConversionFailure)?;
		let mut header = [0u8; FILE_HEADER_SIZE];
		header[..2].copy_from_slice(b"BM");
		header[2..6].copy_from_slice(&file_size.to_le_bytes());
		header[10..14].copy_from_slice(&pixel_offset.to_le_bytes());
		Ok(Self { header, data, position: 0 })
	}
}

impl Read for DibFile<'_> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let source = self.fill_buf()?;
		let count = source.len().min(buf.len());
		buf[..count].copy_from_slice(&source[..count]);
		self.consume(count);
		Ok(count)
	}
}

impl BufRead for DibFile<'_> {
	fn fill_buf(&mut self) -> io::Result<&[u8]> {
		Ok(if self.position < FILE_HEADER_SIZE as u64 {
			&self.header[self.position as usize..]
		} else {
			let offset = match usize::try_from(self.position - FILE_HEADER_SIZE as u64) {
				Ok(offset) => offset,
				Err(_) => return Ok(&[]),
			};
			self.data.get(offset..).unwrap_or(&[])
		})
	}

	fn consume(&mut self, amount: usize) {
		debug_assert!(amount <= self.fill_buf().unwrap().len());
		self.position += amount as u64;
	}
}

impl Seek for DibFile<'_> {
	fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
		let position = match from {
			SeekFrom::Start(position) => Some(position),
			SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
			SeekFrom::End(offset) => {
				(FILE_HEADER_SIZE as u64 + self.data.len() as u64).checked_add_signed(offset)
			}
		}
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "DIB 文件视图寻址越界"))?;
		self.position = position;
		Ok(position)
	}
}

pub(super) fn decoder(data: &[u8]) -> Result<BmpDecoder<DibFile<'_>>, Error> {
	BmpDecoder::new(DibFile::new(data)?).map_err(|_| Error::ConversionFailure)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn dib(bit_count: u16) -> Vec<u8> {
		let mut data = vec![0u8; V5_HEADER_SIZE];
		data[..4].copy_from_slice(&124u32.to_le_bytes());
		data[14..16].copy_from_slice(&bit_count.to_le_bytes());
		data
	}

	#[test]
	fn file_view_reads_across_header_and_preserves_borrow() {
		let mut data = dib(32);
		data.extend_from_slice(&[11, 22, 33, 44]);
		let mut reader = DibFile::new(&data).unwrap();
		assert_eq!(reader.data.as_ptr(), data.as_ptr());
		assert_eq!(u32::from_le_bytes(reader.header[10..14].try_into().unwrap()), 138);
		let mut output = Vec::new();
		reader.read_to_end(&mut output).unwrap();
		assert_eq!(&output[..2], b"BM");
		assert_eq!(&output[FILE_HEADER_SIZE..], &data);
		reader.seek(SeekFrom::Start(12)).unwrap();
		let mut across = [0; 8];
		reader.read_exact(&mut across).unwrap();
		assert_eq!(&across[..2], &output[12..14]);
		assert_eq!(&across[2..], &data[..6]);
	}

	#[test]
	fn file_view_seek_handles_eof_and_invalid_negative_position() {
		let data = dib(24);
		let mut reader = DibFile::new(&data).unwrap();
		assert_eq!(reader.seek(SeekFrom::End(-1)).unwrap(), 137);
		assert_eq!(reader.seek(SeekFrom::Current(-13)).unwrap(), 124);
		assert_eq!(reader.seek(SeekFrom::Start(0)).unwrap(), 0);
		assert!(reader.seek(SeekFrom::Current(-1)).is_err());
		assert_eq!(reader.stream_position().unwrap(), 0);
		reader.seek(SeekFrom::Start(u64::MAX)).unwrap();
		assert_eq!(reader.read(&mut [0; 2]).unwrap(), 0);
		assert!(reader.seek(SeekFrom::Current(1)).is_err());
	}

	#[test]
	fn file_view_validates_v5_and_color_table_offsets() {
		assert!(DibFile::new(&[0; 123]).is_err());
		let mut data = dib(8);
		assert!(DibFile::new(&data).is_err());
		data[32..36].copy_from_slice(&2u32.to_le_bytes());
		data.extend_from_slice(&[0; 8]);
		let reader = DibFile::new(&data).unwrap();
		assert_eq!(u32::from_le_bytes(reader.header[10..14].try_into().unwrap()), 146);
		data[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
		assert!(DibFile::new(&data).is_err());
		data[..4].copy_from_slice(&40u32.to_le_bytes());
		assert!(DibFile::new(&data).is_err());
	}
}
