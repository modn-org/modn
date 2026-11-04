use super::{inspect_reader, AssetFormat};
use std::io::{self, Read, Seek, SeekFrom};

struct MetadataOnlyReader {
    header: Vec<u8>,
    index: Vec<u8>,
    index_offset: u64,
    position: u64,
    reads: usize,
}

impl Read for MetadataOnlyReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let source = if self.position < self.header.len() as u64 {
            &self.header[self.position as usize..]
        } else if self.position >= self.index_offset
            && self.position - self.index_offset < self.index.len() as u64
        {
            &self.index[(self.position - self.index_offset) as usize..]
        } else {
            return Err(io::Error::other("parser attempted a payload read"));
        };
        let count = output.len().min(source.len());
        output[..count].copy_from_slice(&source[..count]);
        self.position += count as u64;
        self.reads += count;
        Ok(count)
    }
}

impl Seek for MetadataOnlyReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let SeekFrom::Start(offset) = target else {
            return Err(io::Error::other("unexpected relative seek"));
        };
        self.position = offset;
        Ok(offset)
    }
}

#[test]
fn terabyte_assets_read_only_fixed_metadata() {
    let bytes = 1_u64 << 40;
    let index_offset = bytes - 28;
    let mut header = Vec::new();
    header.extend_from_slice(b"RDAR");
    header.extend_from_slice(&12_u32.to_le_bytes());
    header.extend_from_slice(&index_offset.to_le_bytes());
    header.extend_from_slice(&28_u32.to_le_bytes());
    header.extend_from_slice(&0_u64.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&bytes.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    let mut index = vec![0; 28];
    index[0..4].copy_from_slice(&8_u32.to_le_bytes());
    index[4..8].copy_from_slice(&20_u32.to_le_bytes());
    let mut reader = MetadataOnlyReader {
        header,
        index,
        index_offset,
        position: 0,
        reads: 0,
    };
    let report = inspect_reader(&mut reader, bytes).unwrap();
    assert_eq!(report.bytes, bytes);
    assert!(matches!(report.format, AssetFormat::Redengine(metadata) if metadata.file_count == 0));
    assert_eq!(reader.reads, 72);

    let mut header = vec![0; 148];
    header[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (12, 1),
        (16, 1),
        (76, 32),
        (80, 4),
        (128, 98),
        (132, 3),
        (140, 1),
    ] {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    header[84..88].copy_from_slice(b"DX10");
    let mut reader = MetadataOnlyReader {
        header,
        index: Vec::new(),
        index_offset: bytes,
        position: 0,
        reads: 0,
    };
    inspect_reader(&mut reader, bytes).unwrap();
    assert_eq!(reader.reads, 148);
    println!("bounded-read assets checked: 2; metadata bytes: 72 and 148; virtual size: {bytes}");
}

#[test]
fn oversized_index_fails_before_any_index_read() {
    let bytes = 1_u64 << 40;
    let mut header = vec![0; 44];
    header[..4].copy_from_slice(b"RDAR");
    header[4..8].copy_from_slice(&12_u32.to_le_bytes());
    header[8..16].copy_from_slice(&172_u64.to_le_bytes());
    header[16..20].copy_from_slice(&(16_u32 * 1024 * 1024 + 1).to_le_bytes());
    header[32..40].copy_from_slice(&bytes.to_le_bytes());
    let mut reader = MetadataOnlyReader {
        header,
        index: Vec::new(),
        index_offset: bytes,
        position: 0,
        reads: 0,
    };
    let error = inspect_reader(&mut reader, bytes).unwrap_err();
    assert!(error.to_string().contains("index size or offset"));
    assert_eq!(reader.reads, 44);
    println!("oversized index cases checked: 1; bytes read before rejection: 44");
}
