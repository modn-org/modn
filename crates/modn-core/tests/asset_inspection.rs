use modn_core::archive::assets;
use modn_core::archive::assets::{AssetFormat, Compression, PixelFormat};
use std::fs;
use std::path::Path;

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn dds(code: &[u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0; if code == b"DX10" { 148 } else { 128 }];
    bytes[..4].copy_from_slice(b"DDS ");
    put32(&mut bytes, 4, 124);
    put32(&mut bytes, 8, 0x2_1007);
    put32(&mut bytes, 12, 32);
    put32(&mut bytes, 16, 64);
    put32(&mut bytes, 28, 7);
    put32(&mut bytes, 76, 32);
    put32(&mut bytes, 80, 4);
    bytes[84..88].copy_from_slice(code);
    put32(&mut bytes, 108, 0x1000);
    if code == b"DX10" {
        put32(&mut bytes, 128, 98);
        put32(&mut bytes, 132, 3);
        put32(&mut bytes, 140, 2);
    }
    bytes
}

fn rdar() -> Vec<u8> {
    let mut bytes = vec![0; 284];
    bytes[..4].copy_from_slice(b"RDAR");
    put32(&mut bytes, 4, 12);
    put64(&mut bytes, 8, 176);
    put32(&mut bytes, 16, 108);
    put64(&mut bytes, 32, 284);
    bytes[172..176].copy_from_slice(b"data");
    put32(&mut bytes, 176, 8);
    put32(&mut bytes, 180, 100);
    put64(&mut bytes, 184, 0x1234_5678_9abc_def0);
    put32(&mut bytes, 192, 1);
    put32(&mut bytes, 196, 1);
    put32(&mut bytes, 200, 1);
    put64(&mut bytes, 204, 0xabcd);
    put32(&mut bytes, 228, 1);
    put32(&mut bytes, 236, 1);
    put64(&mut bytes, 260, 172);
    put32(&mut bytes, 268, 4);
    put32(&mut bytes, 272, 16);
    put64(&mut bytes, 276, 0x9876);
    bytes
}

fn inspect_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<assets::Inspection> {
    fs::write(path, bytes).expect("write disposable asset fixture");
    assets::inspect(path)
}

#[test]
fn dds_fourcc_and_dx10_report_independent_header_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unrelated.extension");
    let report = inspect_bytes(&path, &dds(b"DXT1")).unwrap();
    assert_eq!(report.bytes, 128);
    let AssetFormat::Dds(metadata) = report.format else {
        panic!("DDS expected")
    };
    assert_eq!(
        (
            metadata.width,
            metadata.height,
            metadata.depth,
            metadata.mip_count
        ),
        (64, 32, 1, 7)
    );
    assert_eq!(metadata.caps, 0x1000);
    assert!(
        matches!(metadata.pixel_format, PixelFormat::FourCc { code, compression: Compression::Bc1 } if code == *b"DXT1")
    );
    let report = inspect_bytes(&path, &dds(b"DX10")).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["format"], "dds");
    assert_eq!(json["metadata"]["pixel_format"]["kind"], "dx10");
    assert_eq!(
        json["metadata"]["pixel_format"]["header"]["dxgi_format"],
        98
    );
    let AssetFormat::Dds(metadata) = report.format else {
        panic!("DDS expected")
    };
    let PixelFormat::Dx10 {
        header,
        compression,
    } = metadata.pixel_format
    else {
        panic!("DX10 expected")
    };
    assert_eq!((header.array_size, header.resource_dimension), (2, 3));
    assert_eq!(compression, Compression::Bc7);
}

#[test]
fn dds_legacy_masks_and_unknown_formats_remain_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mut bytes = dds(b"NONE");
    put32(&mut bytes, 80, 0x41);
    put32(&mut bytes, 88, 32);
    put32(&mut bytes, 92, 0xff);
    put32(&mut bytes, 96, 0xff00);
    put32(&mut bytes, 100, 0xff0000);
    put32(&mut bytes, 104, 0xff000000);
    put32(&mut bytes, 8, 0x1007);
    put32(&mut bytes, 28, 0);
    let report = inspect_bytes(&path, &bytes).unwrap();
    let AssetFormat::Dds(metadata) = report.format else {
        panic!("DDS expected")
    };
    assert_eq!(metadata.mip_count, 1);
    assert!(matches!(
        metadata.pixel_format,
        PixelFormat::Legacy {
            bits_per_pixel: 32,
            red_mask: 0xff,
            green_mask: 0xff00,
            blue_mask: 0xff0000,
            alpha_mask: 0xff000000,
            compression: Compression::Uncompressed,
            ..
        }
    ));
    for code in [b"NONE", b"DX10"] {
        let mut bytes = dds(code);
        if code == b"DX10" {
            put32(&mut bytes, 128, u32::MAX);
        }
        let json = serde_json::to_value(inspect_bytes(&path, &bytes).unwrap()).unwrap();
        assert_eq!(json["metadata"]["pixel_format"]["compression"], "unknown");
    }
}

#[test]
fn rdar_reports_real_header_and_index_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("container");
    let report = inspect_bytes(&path, &rdar()).unwrap();
    assert_eq!(report.bytes, 284);
    let AssetFormat::Redengine(metadata) = report.format else {
        panic!("RDAR expected")
    };
    assert_eq!(metadata.version, 12);
    assert_eq!(metadata.declared_bytes, 284);
    assert_eq!(metadata.custom_data_bytes, 0);
    assert_eq!((metadata.index_offset, metadata.index_bytes), (176, 108));
    assert_eq!(
        (metadata.file_table_offset, metadata.file_table_bytes),
        (8, 100)
    );
    assert_eq!(
        (
            metadata.file_count,
            metadata.segment_count,
            metadata.dependency_count
        ),
        (1, 1, 1)
    );
    assert_eq!(metadata.index_crc64, 0x1234_5678_9abc_def0);
    assert_eq!(metadata.encoded_segment_count, 1);
    assert_eq!(metadata.compression, Compression::Unknown);
}

#[test]
fn dds_volume_cube_and_missing_advisory_flags_are_structural() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mut bytes = dds(b"DX10");
    put32(&mut bytes, 8, 0);
    put32(&mut bytes, 24, 8);
    put32(&mut bytes, 112, 0x20_0000);
    put32(&mut bytes, 132, 4);
    put32(&mut bytes, 140, 1);
    let AssetFormat::Dds(metadata) = inspect_bytes(&path, &bytes).unwrap().format else {
        panic!("DDS expected")
    };
    assert_eq!(metadata.depth, 8);
    assert_eq!(metadata.flags, 0);
    put32(&mut bytes, 112, 0);
    put32(&mut bytes, 132, 3);
    put32(&mut bytes, 12, 64);
    put32(&mut bytes, 136, 4);
    let json = serde_json::to_value(inspect_bytes(&path, &bytes).unwrap()).unwrap();
    assert_eq!(json["metadata"]["pixel_format"]["header"]["misc_flag"], 4);
    put32(&mut bytes, 136, 0);
    put32(&mut bytes, 12, 1);
    put32(&mut bytes, 132, 2);
    assert!(inspect_bytes(&path, &bytes).is_ok());
}

#[test]
fn rdar_debug_sections_and_stored_segments_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mut bytes = rdar();
    bytes.extend_from_slice(b"debug");
    put64(&mut bytes, 32, 289);
    put64(&mut bytes, 20, 284);
    put32(&mut bytes, 28, 5);
    put32(&mut bytes, 272, 4);
    let report = inspect_bytes(&path, &bytes).unwrap();
    let AssetFormat::Redengine(metadata) = report.format else {
        panic!("RDAR expected")
    };
    assert_eq!((metadata.debug_offset, metadata.debug_bytes), (284, 5));
    assert_eq!(metadata.encoded_segment_count, 0);
    for offset in [0, 171, 173, 176, 282, 288] {
        put64(&mut bytes, 20, offset);
        assert!(
            inspect_bytes(&path, &bytes).is_err(),
            "accepted debug offset {offset}"
        );
    }
    put64(&mut bytes, 20, 284);
    put64(&mut bytes, 260, 285);
    assert!(inspect_bytes(&path, &bytes).is_err());
    println!("debug boundary cases checked: 7");
}

#[test]
fn rdar_custom_region_is_bounded_without_interpreting_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mut bytes = rdar();
    bytes.splice(172..172, *b"test");
    put32(&mut bytes, 40, 4);
    put64(&mut bytes, 8, 180);
    put64(&mut bytes, 32, 288);
    put64(&mut bytes, 264, 176);
    let AssetFormat::Redengine(metadata) = inspect_bytes(&path, &bytes).unwrap().format else {
        panic!("RDAR expected")
    };
    assert_eq!(metadata.custom_data_bytes, 4);
    assert_eq!(metadata.index_offset, 180);
    put64(&mut bytes, 264, 172);
    assert!(inspect_bytes(&path, &bytes).is_err());
}

#[test]
fn every_truncated_header_and_index_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mut checked = 0;
    for bytes in [dds(b"DXT1"), dds(b"DX10"), rdar()] {
        for end in 0..bytes.len() {
            assert!(
                inspect_bytes(&path, &bytes[..end]).is_err(),
                "accepted truncation at {end}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 560);
    println!("truncated structures checked: {checked}");
}

#[test]
fn malformed_dds_dimensions_mips_and_extensions_fail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mutations = [
        (4, 123),
        (76, 31),
        (12, 0),
        (16, 0),
        (16, u32::MAX),
        (28, 8),
        (28, 0),
        (132, 0),
        (132, 2),
        (140, 0),
        (112, 0x20_0000),
        (136, 4),
    ];
    for (offset, value) in mutations {
        let mut bytes = dds(b"DX10");
        put32(&mut bytes, offset, value);
        assert!(
            inspect_bytes(&path, &bytes).is_err(),
            "accepted DDS field {offset}={value}"
        );
    }
    println!("malformed DDS cases checked: {}", mutations.len());
}

#[test]
fn malformed_rdar_counts_ranges_and_overflow_fail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    let mutations32 = [
        (4, 11),
        (4, 13),
        (16, 27),
        (16, 16 * 1024 * 1024 + 1),
        (40, u32::MAX),
        (40, 5),
        (176, 9),
        (180, 99),
        (192, u32::MAX),
        (196, u32::MAX),
        (200, u32::MAX),
        (220, 2),
        (224, 1),
        (228, 0),
        (228, 2),
        (232, 2),
        (236, 2),
        (268, u32::MAX),
    ];
    for (offset, value) in mutations32 {
        let mut bytes = rdar();
        put32(&mut bytes, offset, value);
        assert!(
            inspect_bytes(&path, &bytes).is_err(),
            "accepted RDAR field {offset}={value}"
        );
    }
    let mutations64 = [
        (8, 0),
        (8, u64::MAX),
        (20, u64::MAX),
        (32, 283),
        (260, u64::MAX),
        (260, 174),
        (260, 0),
    ];
    for (offset, value) in mutations64 {
        let mut bytes = rdar();
        put64(&mut bytes, offset, value);
        assert!(
            inspect_bytes(&path, &bytes).is_err(),
            "accepted RDAR field {offset}={value}"
        );
    }
    println!(
        "malformed RDAR cases checked: {}",
        mutations32.len() + mutations64.len()
    );
}

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root).expect("read disposable tree") {
        let path = entry.expect("read disposable directory entry").path();
        if path.is_dir() {
            result.push((path.clone(), None));
            result.extend(snapshot(&path));
        } else {
            result.push((
                path.clone(),
                Some(fs::read(&path).expect("read disposable file")),
            ));
        }
    }
    result.sort();
    result
}

#[test]
fn inspection_preserves_inputs_parent_tree_and_sentinel_state() {
    let dir = tempfile::tempdir().unwrap();
    for subdir in ["manager", "game", "assets"] {
        fs::create_dir(dir.path().join(subdir)).unwrap();
    }
    fs::write(
        dir.path().join("manager/state.db"),
        b"disposable manager sentinel",
    )
    .unwrap();
    fs::write(
        dir.path().join("game/manual.bin"),
        b"disposable game sentinel",
    )
    .unwrap();
    let inputs = [dds(b"DXT1"), dds(b"DX10"), rdar(), b"unsupported".to_vec()];
    for (index, bytes) in inputs.iter().enumerate() {
        fs::write(dir.path().join(format!("assets/{index}")), bytes).unwrap();
    }
    let before = snapshot(dir.path());
    for index in 0..inputs.len() {
        let path = dir.path().join(format!("assets/{index}"));
        for _ in 0..2 {
            let _ = assets::inspect(&path);
        }
    }
    assert!(assets::inspect(dir.path()).is_err());
    assert!(assets::inspect(&dir.path().join("missing")).is_err());
    assert_eq!(snapshot(dir.path()), before);
    println!("unchanged tree entries checked: {}", before.len());
}

#[test]
fn bad_magic_is_not_inferred_from_filename() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["texture.dds", "container.archive"] {
        let error = inspect_bytes(&dir.path().join(name), b"arbitrary bytes").unwrap_err();
        assert!(error.to_string().contains("unsupported asset signature"));
    }
}
