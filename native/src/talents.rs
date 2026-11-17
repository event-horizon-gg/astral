use regex::Regex;
use std::{fmt::Write as _, path::Path, sync::LazyLock};
use time::OffsetDateTime;

const SPECS: [(u32, &str, &str, &str); 4] = [
    (102, "balance", "dps", "keeper-of-the-grove"),
    (103, "feral", "dps", "druid-of-the-claw"),
    (104, "guardian", "tank", "elunes-chosen"),
    (105, "restoration", "healer", "wildstalker"),
];
const MODES: [&str; 3] = ["Mythic+", "Raid", "Delves"];
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static PATCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"interior-sidebar-header-text-subtitle">\s*Patch (\d+)\.(\d+)\.(\d+)"#)
        .expect("valid patch pattern")
});
static ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\[tr\](.*?)\[/tr\]").expect("valid row pattern"));
static CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[copy="(Mythic\+|Raid|Delves)"\]([A-Za-z0-9+/]{60,512})\[/copy\]"#)
        .expect("valid code pattern")
});
static HERO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"symbol=wow-hero-talent-([a-z-]+)\]").expect("valid hero pattern")
});

fn hero_name(hero: &str) -> Option<&'static str> {
    Some(match hero {
        "keeper-of-the-grove" => "Keeper of the Grove",
        "elunes-chosen" => "Elune's Chosen",
        "druid-of-the-claw" => "Druid of the Claw",
        "wildstalker" => "Wildstalker",
        _ => return None,
    })
}

fn matches_spec(code: &str, id: u32) -> bool {
    // The first 24 bits contain the format version and specialization ID.
    let header = code
        .bytes()
        .take(4)
        .enumerate()
        .fold(0, |header, (i, byte)| {
            let index = ALPHABET
                .iter()
                .position(|symbol| *symbol == byte)
                .unwrap_or(0) as u32;
            header | (index << (i * 6))
        });
    header == (id << 8 | 2)
}

fn generate(
    interface: u32,
    date: &str,
    mut get_guide: impl FnMut(&str, &str) -> Result<String, String>,
) -> Result<String, String> {
    let mut lua = format!(
        "local _, ns = ...\nns.interface = {interface}\nns.checked = \"{date}\"\nns.builds = {{\n"
    );
    for (id, name, role, fallback_hero) in SPECS {
        let page = get_guide(name, role)?;
        let patch = PATCH.captures(&page).and_then(|parts| {
            let part = |index: usize| parts.get(index)?.as_str().parse::<u32>().ok();
            Some(
                part(1)?
                    .checked_mul(10000)?
                    .checked_add(part(2)?.checked_mul(100)?)?
                    .checked_add(part(3)?)?,
            )
        });
        if patch != Some(interface) {
            return Err(format!(
                "The {name} talent guide does not match this WoW patch."
            ));
        }
        let guide = page.replace("\\\"", "\"").replace("\\/", "/");
        let rows: Vec<_> = ROW
            .captures_iter(&guide)
            .filter_map(|row| {
                let row = row.get(1)?.as_str();
                let code = CODE.captures(row)?;
                let hero = HERO.captures(row)?.get(1)?.as_str();
                Some((
                    code.get(1)?.as_str(),
                    code.get(2)?.as_str(),
                    hero,
                    hero_name(hero)?,
                    row.contains("(Best)"),
                ))
            })
            .collect();
        writeln!(lua, "    [{id}] = {{").expect("write to string");
        for mode in MODES {
            let candidates = || rows.iter().filter(|row| row.0 == mode);
            let (_, code, _, hero, _) = candidates()
                .find(|row| row.4)
                .or_else(|| candidates().find(|row| row.2 == fallback_hero))
                .filter(|row| matches_spec(row.1, id))
                .ok_or_else(|| format!("No valid {mode} build in the {name} guide."))?;
            writeln!(lua, "        {{ \"{hero}\", \"{code}\" }},").expect("write to string");
        }
        lua.push_str("    },\n");
    }
    lua.push_str("}\n");
    Ok(lua)
}

pub fn refresh(root: &Path) -> Result<(), String> {
    refresh_changed(root).map(|_| ())
}

pub fn refresh_changed(root: &Path) -> Result<bool, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .build()
        .into();
    let date = OffsetDateTime::now_utc().date().to_string();
    crate::installer::refresh_talents(root, |interface| {
        generate(interface, &date, |name, role| {
            let url = format!(
                "https://www.wowhead.com/guide/classes/druid/{name}/talent-builds-pve-{role}"
            );
            agent
                .get(&url)
                .header("User-Agent", "Astral-Native")
                .call()
                .and_then(|mut response| {
                    response
                        .body_mut()
                        .with_config()
                        .limit(2 * 1024 * 1024)
                        .read_to_string()
                })
                .map_err(|error| format!("Couldn't load the {name} talent guide. {error}"))
        })
    })
}
