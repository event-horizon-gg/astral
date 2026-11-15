use async_zip::base::read::mem::ZipFileReader;
use futures_lite::io::AsyncReadExt;
use std::{collections::HashSet, fs, io::Write, path::Path};

const LIMIT: u64 = 2 * 1024 * 1024 * 1024;

pub(super) async fn extract(
    bytes: Vec<u8>,
    directory: &Path,
    accepts: impl Fn(&str, bool) -> bool,
    executable: &str,
) -> Result<(), String> {
    crate::packages::check_directory(&bytes)?;
    let archive = ZipFileReader::new(bytes)
        .await
        .map_err(|error| error.to_string())?;
    let mut expanded = 0u64;
    let mut seen = HashSet::new();
    for (index, entry) in archive.file().entries().iter().enumerate() {
        let raw = entry
            .filename()
            .as_str()
            .map_err(|_| "Invalid application ZIP path.")?;
        let dir = raw.ends_with('/');
        let name = raw.strip_suffix('/').unwrap_or(raw);
        let mode = entry.external_file_attribute() >> 16;
        let kind = mode & 0o170000;
        if !accepts(name, dir)
            || !name.split('/').all(crate::packages::safe_component)
            || !name.is_ascii()
            || !seen.insert(name.to_ascii_lowercase())
            || (dir && !matches!(kind, 0 | 0o040000))
            || (!dir && !matches!(kind, 0 | 0o100000))
        {
            return Err("Unsafe application ZIP entry.".into());
        }
        expanded = expanded
            .checked_add(entry.uncompressed_size())
            .filter(|size| *size <= LIMIT)
            .ok_or("The application ZIP exceeds its expanded size limit.")?;
        let destination = directory.join(name);
        if dir {
            fs::create_dir_all(destination).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| error.to_string())?;
        let mut reader = archive
            .reader_with_entry(index)
            .await
            .map_err(|error| error.to_string())?;
        let mut remaining = entry.uncompressed_size();
        let mut buffer = [0; 32 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            remaining = remaining
                .checked_sub(count as u64)
                .ok_or("The ZIP entry exceeded its listed size.")?;
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
        }
        if remaining != 0 || reader.compute_hash() != entry.crc32() {
            return Err("The application ZIP entry is damaged.".into());
        }
        #[cfg(unix)]
        if name == executable {
            use std::os::unix::fs::PermissionsExt;
            output
                .set_permissions(fs::Permissions::from_mode(0o755))
                .map_err(|error| error.to_string())?;
        }
    }
    if !seen.contains(&executable.to_ascii_lowercase()) || !directory.join(executable).is_file() {
        return Err("The application executable is missing from the ZIP.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};

    #[test]
    fn windows_update_package_contains_only_the_root_executable() {
        futures_lite::future::block_on(async {
            for (entries, accepted) in [
                (vec!["astral-native.exe"], true),
                (vec!["astral-native.exe", "extra.dll"], false),
                (vec!["astral-native.exe", "ASTRAL-NATIVE.EXE"], false),
                (vec!["../astral-native.exe"], false),
            ] {
                let mut data = Vec::new();
                let mut zip = ZipFileWriter::new(&mut data);
                for name in entries {
                    zip.write_entry_whole(
                        ZipEntryBuilder::new(name.into(), Compression::Deflate),
                        b"test executable",
                    )
                    .await
                    .unwrap();
                }
                zip.close().await.unwrap();
                let stage = tempfile::tempdir().unwrap();
                let result = extract(
                    data,
                    stage.path(),
                    |name, dir| !dir && name == "astral-native.exe",
                    "astral-native.exe",
                )
                .await;
                assert_eq!(result.is_ok(), accepted);
            }
        });
    }
}
