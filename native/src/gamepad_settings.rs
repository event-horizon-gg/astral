use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::LazyLock,
};

const BOM: &[u8] = b"\xef\xbb\xbf";

static SET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^SET[ \t]+([a-z0-9_]+)[ \t]+"([^"\\]*)"[ \t]*$"#).unwrap());
static KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^SET[ \t]+([a-z0-9_]+)[ \t]+").unwrap());

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Settings(BTreeMap<String, String>);

fn owned(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    (lower.starts_with("gamepad") || lower.starts_with("softtarget"))
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

impl Settings {
    pub fn enable_controller(&mut self) {
        self.0
            .retain(|key, _| !key.eq_ignore_ascii_case("GamePadEnable"));
        self.0.insert("GamePadEnable".into(), "1".into());
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        let mut seen = HashSet::new();
        if self.0.len() > 250
            || self.0.iter().any(|(key, value)| {
                !owned(key)
                    || !seen.insert(key.to_ascii_lowercase())
                    || value.len() > 128
                    || value
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '"' | '\\'))
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid controller settings.",
            ));
        }
        Ok(())
    }

    pub fn capture(config: &[u8]) -> io::Result<Self> {
        let config = config.strip_prefix(BOM).unwrap_or(config);
        let mut values = BTreeMap::new();
        for line in config.split(|&byte| byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let Some(fields) = SET.captures(line) else {
                continue;
            };
            let key = std::str::from_utf8(&fields[1]).unwrap_or_default();
            if owned(key) {
                let value = std::str::from_utf8(&fields[2]).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid controller setting value.",
                    )
                })?;
                values.insert(key.to_ascii_lowercase(), (key.to_owned(), value.to_owned()));
            }
        }
        let settings = Self(values.into_values().collect());
        settings.validate()?;
        Ok(settings)
    }

    pub fn render(&self, config: &[u8], current: &Self) -> io::Result<Option<Vec<u8>>> {
        self.validate()?;
        if self == current {
            return Ok(None);
        }
        let text = config.strip_prefix(BOM).unwrap_or(config);
        let mut output = Vec::with_capacity(config.len());
        if config.starts_with(BOM) {
            output.extend_from_slice(BOM);
        }
        let newline = if text.windows(2).any(|pair| pair == b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        for line in text.split_inclusive(|&byte| byte == b'\n') {
            let controller = KEY
                .captures(line)
                .is_some_and(|fields| owned(std::str::from_utf8(&fields[1]).unwrap_or_default()));
            if !controller {
                output.extend_from_slice(line);
            }
        }
        for (key, value) in &self.0 {
            if output.len() > config.len() - text.len() && !output.ends_with(b"\n") {
                output.extend_from_slice(newline.as_bytes());
            }
            output.extend_from_slice(format!("SET {key} \"{value}\"{newline}").as_bytes());
        }
        Ok(Some(output))
    }
}
