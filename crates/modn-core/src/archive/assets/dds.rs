use super::{read_array, u32_at, Compression};
use anyhow::{bail, Result};
use serde::Serialize;
use std::io::Read;

/// DDS header values. Pixel payload length, masks and decoded pixels are not verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct DdsMetadata {
    /// Width in pixels; this MVP supports dimensions up to 131072.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Depth for a volume texture, otherwise one.
    pub depth: u32,
    /// Effective mip levels; an absent count means one level.
    pub mip_count: u32,
    /// Raw DDS header flags, retained because writers can omit advisory flags.
    pub flags: u32,
    /// Raw DDS capability bits.
    pub caps: u32,
    /// Raw extended DDS capability bits, including cube and volume flags.
    pub caps2: u32,
    /// Declared pixel encoding, without assuming codec support.
    pub pixel_format: PixelFormat,
}

/// Pixel format identifiers and legacy masks are reported without decoding pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
#[expect(
    missing_docs,
    reason = "Fields preserve standard DDS pixel format names"
)]
pub enum PixelFormat {
    Legacy {
        flags: u32,
        bits_per_pixel: u32,
        red_mask: u32,
        green_mask: u32,
        blue_mask: u32,
        alpha_mask: u32,
        compression: Compression,
    },
    FourCc {
        /// Raw four bytes, including non-text legacy numeric identifiers.
        code: [u8; 4],
        compression: Compression,
    },
    Dx10 {
        header: Dx10Metadata,
        compression: Compression,
    },
}

/// DDS DX10 extension. Unknown DXGI identifiers remain numeric with unknown compression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
#[expect(
    missing_docs,
    reason = "Fields preserve standard DDS DX10 header names"
)]
pub struct Dx10Metadata {
    pub dxgi_format: u32,
    pub resource_dimension: u32,
    pub misc_flag: u32,
    pub array_size: u32,
    pub misc_flags2: u32,
}

pub(super) fn inspect(reader: &mut impl Read) -> Result<DdsMetadata> {
    let header = read_array::<124>(reader)?;
    if u32_at(&header, 0) != 124 || u32_at(&header, 72) != 32 {
        bail!("invalid DDS header or pixel format size");
    }
    let width = u32_at(&header, 12);
    let height = u32_at(&header, 8);
    let flags = u32_at(&header, 4);
    let caps2 = u32_at(&header, 108);
    let volume = caps2 & 0x20_0000 != 0;
    let depth = if volume { u32_at(&header, 20) } else { 1 };
    if [width, height, depth]
        .iter()
        .any(|&n| n == 0 || n > 131_072)
    {
        bail!("invalid or unsupported DDS dimensions (expected 1..=131072)");
    }
    let declared_mips = u32_at(&header, 24);
    let mip_count = declared_mips.max(1);
    let max_mips = u32::BITS - width.max(height).max(depth).leading_zeros();
    if mip_count > max_mips || (flags & 0x2_0000 != 0 && declared_mips == 0) {
        bail!("invalid DDS mip count");
    }
    let pixel_flags = u32_at(&header, 76);
    let pixel_format = if pixel_flags & 4 != 0 {
        let code: [u8; 4] = header[80..84].try_into().expect("four-byte DDS identifier");
        if code == *b"DX10" {
            let extension = read_array::<20>(reader)?;
            let header = Dx10Metadata {
                dxgi_format: u32_at(&extension, 0),
                resource_dimension: u32_at(&extension, 4),
                misc_flag: u32_at(&extension, 8),
                array_size: u32_at(&extension, 12),
                misc_flags2: u32_at(&extension, 16),
            };
            if header.array_size == 0
                || !matches!(header.resource_dimension, 2..=4)
                || (header.resource_dimension == 2 && height != 1)
                || (header.resource_dimension == 4 && (!volume || header.array_size != 1))
                || (header.resource_dimension != 4 && volume)
                || (header.misc_flag & 4 != 0
                    && (header.resource_dimension != 3 || width != height))
            {
                bail!("invalid DDS DX10 dimensions or array size");
            }
            let compression = match header.dxgi_format {
                70..=72 => Compression::Bc1,
                73..=75 => Compression::Bc2,
                76..=78 => Compression::Bc3,
                79..=81 => Compression::Bc4,
                82..=84 => Compression::Bc5,
                94..=96 => Compression::Bc6h,
                97..=99 => Compression::Bc7,
                _ => Compression::Unknown,
            };
            PixelFormat::Dx10 {
                header,
                compression,
            }
        } else {
            let compression = match &code {
                b"DXT1" => Compression::Bc1,
                b"DXT2" | b"DXT3" => Compression::Bc2,
                b"DXT4" | b"DXT5" => Compression::Bc3,
                b"ATI1" | b"BC4U" | b"BC4S" => Compression::Bc4,
                b"ATI2" | b"BC5U" | b"BC5S" => Compression::Bc5,
                _ => Compression::Unknown,
            };
            PixelFormat::FourCc { code, compression }
        }
    } else {
        let bits_per_pixel = u32_at(&header, 84);
        let known_uncompressed = pixel_flags & (0x40 | 0x2_0000 | 2) != 0;
        if known_uncompressed && (bits_per_pixel == 0 || bits_per_pixel > 32) {
            bail!("invalid or unsupported legacy DDS pixel bit count");
        }
        PixelFormat::Legacy {
            flags: pixel_flags,
            bits_per_pixel,
            red_mask: u32_at(&header, 88),
            green_mask: u32_at(&header, 92),
            blue_mask: u32_at(&header, 96),
            alpha_mask: u32_at(&header, 100),
            compression: if known_uncompressed {
                Compression::Uncompressed
            } else {
                Compression::Unknown
            },
        }
    };
    Ok(DdsMetadata {
        width,
        height,
        depth,
        mip_count,
        flags,
        caps: u32_at(&header, 104),
        caps2,
        pixel_format,
    })
}
