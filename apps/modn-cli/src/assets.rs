use crate::commands::AssetCommand;
use anyhow::Result;
use modn_core::archive::assets;
use modn_core::archive::assets::AssetFormat;

pub(super) fn run(command: &AssetCommand, json: bool) -> Result<()> {
    let AssetCommand::Inspect { path } = command;
    let report = assets::inspect(path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!(
        "{} ({} bytes, structural metadata only)",
        path.display(),
        report.bytes
    );
    match &report.format {
        AssetFormat::Dds(metadata) => {
            println!(
                "DDS: {} x {} x {}, {} mip levels",
                metadata.width, metadata.height, metadata.depth, metadata.mip_count
            );
            println!(
                "Pixel format: {}",
                serde_json::to_string(&metadata.pixel_format)?
            );
        }
        AssetFormat::Redengine(metadata) => {
            println!(
                "REDengine RDAR version {}: {} files, {} segments, {} dependencies",
                metadata.version,
                metadata.file_count,
                metadata.segment_count,
                metadata.dependency_count
            );
            println!(
                "Index: {} bytes at {}; {} encoded segments; compression unknown",
                metadata.index_bytes, metadata.index_offset, metadata.encoded_segment_count
            );
        }
        _ => println!("Format: {}", serde_json::to_string(&report.format)?),
    }
    println!("Payload integrity and game compatibility are not verified.");
    Ok(())
}
