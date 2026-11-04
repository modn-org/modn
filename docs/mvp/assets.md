# Read-only asset inspection

The core inspects DDS texture headers and a conservative REDengine RDAR version 12 structural subset.
It selects the format from bytes, not the filename.
It does not infer a game, compatibility, ownership, trust, or permission to install.

## Public API

```rust
use modn_core::archive::assets;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let report = assets::inspect(Path::new("texture.dds"))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
```

The exact entry point is `archive::assets::inspect(path: &Path) -> anyhow::Result<Inspection>`.
`Inspection` contains `bytes: u64` and `format: AssetFormat`.
The public types derive `Debug`, `Clone`, `PartialEq`, `Eq`, and `Serialize`.
Public enums and structs are non-exhaustive.

`AssetFormat` contains `Dds(DdsMetadata)` or `Redengine(RedengineMetadata)`.
Serialization flattens the format into `format` and `metadata` fields:

```json
{
  "bytes": 128,
  "format": "dds",
  "metadata": {
    "width": 64,
    "height": 32,
    "depth": 1,
    "mip_count": 7,
    "flags": 135175,
    "caps": 4096,
    "caps2": 0,
    "pixel_format": {
      "kind": "four_cc",
      "code": [68, 88, 84, 49],
      "compression": "bc1"
    }
  }
}
```

`bytes` reports the opened file's actual length.
Unknown signatures, unsupported REDengine versions, malformed structures, and I/O failures return errors.
The API opens the selected regular file for reading.
It does not enumerate directories, create manager state, execute tools, or load game modules.
It does not prevent concurrent file changes or create an integrity snapshot.
Filesystem access-time changes can follow the operating system's read policy.

## DDS output and limits

The parser reads the 4-byte `DDS ` signature and 124-byte `DDS_HEADER`.
It requires `dwSize == 124` and pixel-format size 32.
A FourCC of `DX10` adds the 20-byte `DDS_HEADER_DXT10`.
The maximum read is 148 bytes, independent of payload size.

`DdsMetadata` reports these fields:

| Field | Meaning |
| --- | --- |
| `width`, `height` | Declared pixel dimensions |
| `depth` | Declared volume depth, or one for other textures |
| `mip_count` | Declared levels, or one when the count is absent |
| `flags`, `caps`, `caps2` | Raw DDS flags and capability bits |
| `pixel_format` | A tagged `PixelFormat` value |

`PixelFormat::Legacy` serializes with `kind: "legacy"`.
It contains `flags`, `bits_per_pixel`, `red_mask`, `green_mask`, `blue_mask`, `alpha_mask`, and `compression`.
Recognized RGB, luminance, and alpha flags identify uncompressed data.
Other flags report unknown compression.
Supported legacy bit counts range from 1 through 32.
The parser reports masks without validating channel layout.

`PixelFormat::FourCc` serializes with `kind: "four_cc"`.
It contains the four raw `code` bytes and `compression`.
Raw bytes preserve numeric legacy identifiers and non-text codes.
Recognized identifiers are DXT1–DXT5, ATI1, ATI2, BC4U, BC4S, BC5U, and BC5S.
They identify BC1 through BC5 storage.
Premultiplied-alpha distinctions remain in the original FourCC bytes.
Unknown identifiers retain their bytes and report `compression: "unknown"`.

`PixelFormat::Dx10` serializes with `kind: "dx10"`.
It contains `header: Dx10Metadata` and `compression`.
The extension reports `dxgi_format`, `resource_dimension`, `misc_flag`, `array_size`, and `misc_flags2` as raw unsigned integers.
DXGI ranges 70–84 and 94–99 identify BC1 through BC7 storage.
Other DXGI values remain numeric and report unknown compression, including uncompressed DXGI formats in this MVP.

Dimensions must range from 1 through 131072.
This upper bound is an inspection policy, not a DDS specification limit or GPU compatibility claim.
Mip counts must fit the largest declared dimension.
A flagged mip count of zero fails.
The parser tolerates missing advisory DDS flags, as Microsoft recommends for readers.
DX10 dimensions must identify a 1D, 2D, or 3D texture.
Arrays must be nonempty; 3D textures require one array element and volume capability.
Cube textures require square 2D faces.

The parser does not read, size-check, decode, or decompress pixel payloads.
A complete header without pixel data can therefore produce metadata.
Header truncation fails; successful inspection does not prove a complete texture.
Pitch, channel masks, face counts, alpha modes, and payload integrity remain unverified.

## REDengine output and limits

The parser supports the little-endian `RDAR` version 12 layout used by the cited WolvenKit reader and writer.
Other versions return an explicit unsupported-version error.
This support does not identify a game or approve a game-specific package layout.

`RedengineMetadata` reports these fields:

| Field | Meaning |
| --- | --- |
| `version` | Supported header version, currently 12 |
| `declared_bytes` | Declared file length, required to equal actual length |
| `index_offset`, `index_bytes` | Index section location and length |
| `debug_offset`, `debug_bytes` | Optional debug section location and length |
| `custom_data_bytes` | Custom region length after the 172-byte extended header |
| `file_table_offset`, `file_table_bytes` | Raw index table declarations |
| `index_crc64` | Declared CRC64, without verification |
| `file_count`, `segment_count`, `dependency_count` | Validated index counts |
| `encoded_segment_count` | Segments with different stored and expanded lengths |
| `compression` | Always `"unknown"`; index lengths do not identify a codec |

The parser reads the 40-byte core header and the following 4-byte custom-data length.
It reserves the complete 172-byte extended header and validates the following custom region's extent.
The index must start after that region.
The index must range from 28 bytes through 16 MiB.
Its fixed prefix contains two 32-bit table fields, a 64-bit CRC, and three 32-bit counts.
Supported table declarations use offset 8 and table size `20 + records_bytes`.
The complete index size must equal `28 + records_bytes`.
`records_bytes` equals `56 * file_count + 16 * segment_count + 8 * dependency_count`.
Checked arithmetic and the index limit bound all loops.
The implementation retains no vector of index records.

Each 56-byte file record contains a hash, timestamp, inline-segment count, segment range, dependency range, and SHA1 bytes.
The parser validates nonempty segment ranges and bounded dependency ranges.
It requires the inline-segment count to fit the file's segment range.
Each 16-byte segment record contains an offset, stored length, and expanded length.
Stored sections must fit the file and avoid the extended header, custom region, index, and nonempty debug section.
The debug section must also avoid the extended header, custom region, and index.
Eight-byte dependency identifiers need no content validation and are not read.
Their declared extent must fit the index exactly.

Total reads cannot exceed 44 header bytes plus 16 MiB of index metadata.
The parser does not read debug contents, reserved header padding, custom contents, or game payloads.
It does not verify CRC64, SHA1, timestamp meaning, duplicate hashes, shared segments, or resource dependency identities.
It does not resolve resource names or validate decompressed lengths.
It does not claim support for all RDAR variants or other REDengine asset formats.

## CLI inspection

Run `modn asset inspect <path> [--json]` for read-only structural metadata.
The handler calls `assets::inspect` directly, without selecting a game or opening manager state.
It does not load modules, fetch network content, or write the input.
JSON output serializes `Inspection` with the fields shown above.
Human output shows the format, file length, dimensions or index counts, and structural-only limitation.
Errors exit unsuccessfully and retain the parser's reason.
Asset conversion, payload decoding, MCP, and desktop UI integration remain pending.

## Format references

Microsoft publishes the authoritative DDS layout:

- [DDS file layout](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide)
- [DDS_HEADER](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-header)
- [DDS_PIXELFORMAT](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-pixelformat)
- [DDS_HEADER_DXT10](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-header-dxt10)
- [DXGI_FORMAT identifiers](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format)

The REDengine subset follows WolvenKit's maintained format implementation at commit `6ab5da88c009b99e653ce87be5251a9c5bd15330`.
These implementation references are not an official CD Projekt format specification:

- [RDAR header and version](https://github.com/WolvenKit/WolvenKit/blob/6ab5da88c009b99e653ce87be5251a9c5bd15330/WolvenKit.RED4/Archive/Base/Header.cs)
- [Index record types](https://github.com/WolvenKit/WolvenKit/blob/6ab5da88c009b99e653ce87be5251a9c5bd15330/WolvenKit.RED4/Archive/Base/Index.cs)
- [Binary reader layouts](https://github.com/WolvenKit/WolvenKit/blob/6ab5da88c009b99e653ce87be5251a9c5bd15330/WolvenKit.RED4/Archive/IO/ArchiveReader.cs)
- [Binary writer table offsets and lengths](https://github.com/WolvenKit/WolvenKit/blob/6ab5da88c009b99e653ce87be5251a9c5bd15330/WolvenKit.RED4/Archive/IO/ArchiveWriter.cs)
