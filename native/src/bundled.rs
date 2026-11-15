use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
use futures_lite::future::block_on;

macro_rules! addon_files {
    ($folder:literal, $($file:literal),+ $(,)?) => {
        &[
            $((concat!($folder, "/", $file),
                include_bytes!(concat!("../../addons/", $folder, "/", $file)))),+
        ]
    };
}

type Files = &'static [(&'static str, &'static [u8])];

fn files(folder: &str) -> Option<Files> {
    match folder {
        "AstralClock" => Some(addon_files!("AstralClock", "AstralClock.toc", "Clock.lua")),
        "AstralForeverStyle" => Some(addon_files!(
            "AstralForeverStyle",
            "AstralForeverStyle.toc",
            "Style.lua",
        )),
        "AstralProfiles" => Some(addon_files!(
            "AstralProfiles",
            "AstralProfiles.toc",
            "Profiles.lua"
        )),
        "AstralTalents" => Some(addon_files!(
            "AstralTalents",
            "AstralTalents.toc",
            "Builds.lua",
            "Talents.lua",
        )),
        _ => None,
    }
}

pub fn version(folder: &str) -> Option<String> {
    let (_, toc) = files(folder)?.first()?;
    let text = std::str::from_utf8(toc).ok()?;
    crate::library::toc_metadata(text).remove("Version")
}

pub fn package(folder: &str) -> Result<Vec<u8>, String> {
    let files = files(folder).ok_or("This addon is missing from the Astral build.")?;
    block_on(async {
        let mut bytes = Vec::new();
        let mut writer = ZipFileWriter::new(&mut bytes);
        for &(name, contents) in files {
            writer
                .write_entry_whole(
                    ZipEntryBuilder::new(name.into(), Compression::Stored),
                    contents,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        writer.close().await.map_err(|error| error.to_string())?;
        Ok(bytes)
    })
}
