use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE_LIMIT: u64 = 8 * 1024 * 1024;

fn resolve(base: &Path, parts: &[&str], extension: &str) -> Option<PathBuf> {
    let mut path = base.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        let name = if index + 1 == parts.len() {
            format!("{part}{extension}")
        } else {
            part.to_string()
        };
        let exact = path.join(&name);
        path = if exact.exists() {
            exact
        } else {
            path.read_dir()
                .ok()?
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&name)
                })?
                .path()
        };
    }
    let file = path.canonicalize().ok()?;
    file.starts_with(base).then_some(file)
}

// BLP2's block-compressed mip uses the same DXT payload as DDS. Give the
// image crate a DDS header so it can decode the payload with its existing decoder.
fn blp_as_dds(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..4)? != b"BLP2" || bytes.get(8) != Some(&2) {
        return None;
    }
    let word = |at| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let width = word(12)?;
    let height = word(16)?;
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let fourcc = match bytes[10] {
        0 => b"DXT1",
        1 => b"DXT3",
        7 => b"DXT5",
        _ => return None,
    };
    let start = usize::try_from(word(20)?).ok()?;
    let length = usize::try_from(word(84)?).ok()?;
    let payload = bytes.get(start..start.checked_add(length)?)?;
    let mut dds = vec![0; 128];
    dds[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x1007),
        (12, height),
        (16, width),
        (76, 32),
        (80, 4),
        (108, 0x1000),
    ] {
        dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dds[84..88].copy_from_slice(fourcc);
    dds.extend_from_slice(payload);
    Some(dds)
}

pub fn load(base: &Path, reference: &str) -> Option<Arc<gpui::Image>> {
    let parts: Vec<_> = reference.split(['/', '\\']).collect();
    if parts.len() < 4
        || !parts[0].eq_ignore_ascii_case("Interface")
        || !parts[1].eq_ignore_ascii_case("AddOns")
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains(':'))
    {
        return None;
    }
    let extensions: &[&str] = if Path::new(parts.last()?).extension().is_some() {
        &[""]
    } else {
        &[".tga", ".blp", ".png", ".jpg", ".jpeg"]
    };
    let file = extensions
        .iter()
        .find_map(|ext| resolve(base, &parts[2..], ext))?;
    let format = file.extension()?.to_str()?.to_ascii_lowercase();
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(file)
        .ok()?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > FILE_LIMIT {
        return None;
    }
    let format = match format.as_str() {
        "blp" => {
            bytes = blp_as_dds(&bytes)?;
            image::ImageFormat::Dds
        }
        "tga" => image::ImageFormat::Tga,
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        _ => return None,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    display_image(decoded)
}

// The sprite atlas has no mipmaps. Filter large textures before upload so
// minification retains smooth edges, with two pixels per point at 64pt.
fn display_image(decoded: image::DynamicImage) -> Option<Arc<gpui::Image>> {
    let decoded = if decoded.width() > 128 || decoded.height() > 128 {
        decoded.resize(128, 128, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut png = Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        png.into_inner(),
    )))
}

pub fn bundled(bytes: &'static [u8]) -> Arc<gpui::Image> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()
        .and_then(display_image)
        .expect("bundled artwork must be a valid PNG")
}
