use crate::{
    Definition, gamepad_settings,
    settings_backup::{self, Entry},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::{self, Read, Write},
};

const MAX_RAW: usize = 24 * 1024 * 1024;
pub const MAX_PAYLOAD: usize = 1_400_000;
const MAX_FILES: usize = 2_000;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub schema: u8,
    pub files: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addon_selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<LibraryAddon>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<gamepad_settings::Settings>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryAddon {
    pub folder: String,
    pub name: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Selection {
    pub schema: u8,
    pub scopes: BTreeMap<String, BTreeMap<String, bool>>,
}

#[derive(Serialize)]
pub struct Encoded {
    pub payload: String,
    pub hash: String,
    pub files: usize,
}

pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Bundle {
    pub fn normalized_hash(&self) -> io::Result<String> {
        Ok(checksum(&serde_json::to_vec(self)?))
    }
    pub fn encode(&self, catalog: &[Definition]) -> io::Result<Encoded> {
        self.validate(catalog)?;
        let raw = serde_json::to_vec(self)?;
        if raw.len() > MAX_RAW {
            return Err(invalid("This setup exceeds the settings sync limit."));
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        let payload = STANDARD.encode(encoder.finish()?);
        if payload.len() > MAX_PAYLOAD {
            return Err(invalid("This setup exceeds the cloud snapshot limit."));
        }
        Ok(Encoded {
            payload,
            hash: checksum(&raw),
            files: self.files.len() + usize::from(self.schema == 2),
        })
    }

    pub fn decode(
        payload: &str,
        expected_hash: Option<&str>,
        catalog: &[Definition],
    ) -> io::Result<Self> {
        Self::decode_with_hash(payload, expected_hash, catalog).map(|(bundle, _)| bundle)
    }

    pub fn decode_with_hash(
        payload: &str,
        expected_hash: Option<&str>,
        catalog: &[Definition],
    ) -> io::Result<(Self, String)> {
        if payload.is_empty() || payload.len() > MAX_PAYLOAD {
            return Err(invalid("Invalid or oversized cloud snapshot."));
        }
        let bytes = STANDARD
            .decode(payload)
            .map_err(|_| invalid("Invalid cloud snapshot encoding."))?;
        let mut raw = Vec::new();
        MultiGzDecoder::new(bytes.as_slice())
            .take(MAX_RAW as u64 + 1)
            .read_to_end(&mut raw)?;
        if raw.len() > MAX_RAW {
            return Err(invalid(
                "The cloud snapshot expands beyond the settings limit.",
            ));
        }
        let hash = checksum(&raw);
        if expected_hash.is_some_and(|expected| hash != expected) {
            return Err(invalid("The cloud backup checksum does not match."));
        }
        let mut bundle: Self = serde_json::from_slice(&raw)?;
        bundle.validate(catalog)?;
        bundle
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        if let Some(library) = &mut bundle.library {
            library.sort_by(|left, right| left.folder.cmp(&right.folder));
        }
        Ok((bundle, hash))
    }

    pub fn validate(&self, catalog: &[Definition]) -> io::Result<()> {
        match self.schema {
            1 if self.addon_selection.is_none()
                && self.library.is_none()
                && self.input.is_none() => {}
            2 if self.addon_selection.is_some() => {}
            _ => return Err(invalid("Unsupported cloud snapshot format.")),
        }
        if self.files.len() > MAX_FILES {
            return Err(invalid(
                "The cloud snapshot contains too many settings files.",
            ));
        }
        let mut seen = HashSet::new();
        for entry in &self.files {
            if !portable_file(&entry.path, catalog) || !seen.insert(entry.path.to_lowercase()) {
                return Err(invalid("Invalid or duplicate file in cloud snapshot."));
            }
            entry.decode()?;
        }
        if let Some(selection) = &self.addon_selection {
            selection.validate()?;
        }
        if let Some(library) = &self.library {
            validate_library(library)?;
        }
        if let Some(input) = &self.input {
            input.validate()?;
        }
        Ok(())
    }
}

pub fn portable_file(path: &str, catalog: &[Definition]) -> bool {
    if !settings_backup::allowed(path, catalog) {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or_default();
    if name.eq_ignore_ascii_case("AddOns.txt") || name == "cooldownmanager.txt" {
        return false;
    }
    name.strip_suffix(".lua").is_none_or(|addon| {
        addon.starts_with("EllesmereUI")
            || addon.starts_with("Blizzard_")
            || (addon != "AstralTalents"
                && catalog
                    .iter()
                    .any(|definition| definition.module_names().any(|folder| folder == addon)))
    })
}

impl Selection {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema != 1 || !self.scopes.contains_key("") || self.scopes.len() > 500 {
            return Err(invalid("Invalid addon selections."));
        }
        for (scope, addons) in &self.scopes {
            if !scope.is_empty()
                && (scope.split('/').count() != 2
                    || scope.split('/').any(|part| {
                        part.eq_ignore_ascii_case("SavedVariables")
                            || !settings_backup::safe_component(part)
                    }))
            {
                return Err(invalid("Invalid character in addon selections."));
            }
            let mut seen = HashSet::new();
            if addons.len() > 2_000
                || addons
                    .keys()
                    .any(|name| !valid_addon(name, 128) || !seen.insert(name.to_lowercase()))
            {
                return Err(invalid("Invalid addon selections."));
            }
        }
        let mut seen = HashSet::new();
        if self
            .scopes
            .keys()
            .any(|scope| !seen.insert(scope.to_lowercase()))
        {
            return Err(invalid("Duplicate character in addon selections."));
        }
        Ok(())
    }
}

pub fn valid_addon(name: &str, limit: usize) -> bool {
    name.trim() == name
        && name.encode_utf16().count() <= limit
        && settings_backup::safe_component(name)
        && !name.to_ascii_lowercase().starts_with("blizzard_")
}

pub fn validate_library(library: &[LibraryAddon]) -> io::Result<()> {
    let mut seen = HashSet::new();
    if library.len() > 250
        || library.iter().any(|addon| {
            !valid_addon(&addon.folder, 100)
                || addon.name.encode_utf16().count() > 120
                || addon.name.chars().any(char::is_control)
                || !seen.insert(addon.folder.to_lowercase())
        })
    {
        return Err(invalid("Invalid addon collection."));
    }
    Ok(())
}

fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
