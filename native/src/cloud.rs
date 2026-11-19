use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt, time::Duration};

const ORIGIN: &str = "https://astral-sync.astral-joaoseixas.workers.dev";

#[cfg(test)]
#[path = "cloud_worker_tests.rs"]
mod worker_tests;

#[derive(Clone, Deserialize, Serialize)]
pub struct Session {
    token: String,
    pub name: String,
}

impl Session {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !opaque_id(&self.token)
            || self.name.trim().is_empty()
            || self.name.len() > 320
            || self.name.chars().any(char::is_control)
        {
            return Err("Invalid Battle.net session.");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Error {
    pub status: Option<u16>,
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub struct Pair {
    id: String,
    verifier: String,
    pub url: String,
}

impl Pair {
    fn from_response(response: PairResponse, verifier: String) -> Result<Self, Error> {
        if !opaque_id(&response.id)
            || response.url != format!("{ORIGIN}/auth/start?id={}", response.id)
        {
            return Err(Error::new("Astral returned an invalid sign-in address."));
        }
        Ok(Self {
            id: response.id,
            verifier,
            url: response.url,
        })
    }
}

#[derive(Deserialize)]
struct PairResponse {
    id: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PollResponse {
    Session(Session),
    Pending { pending: bool },
}

#[derive(Deserialize)]
pub struct Overview {
    pub devices: Vec<Device>,
    pub history: Vec<HistoryEntry>,
    pub revision: u64,
    pub usage: Option<CloudUsage>,
}

#[derive(Deserialize)]
pub struct CloudUsage {
    pub paused: bool,
}

#[derive(Clone, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub created: String,
    pub files: Option<usize>,
    pub bytes: Option<u64>,
    pub device_label: Option<String>,
    pub current: bool,
    pub revision: u64,
}

#[derive(Deserialize)]
pub struct HistoricalSnapshot {
    pub payload: String,
    pub hash: Option<String>,
}

#[derive(Deserialize)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub current: bool,
    pub created: Option<i64>,
    pub last_seen: Option<i64>,
}

#[derive(Deserialize)]
pub struct RemoteSnapshot {
    pub revision: u64,
    pub hash: Option<String>,
    pub payload: Option<String>,
}

#[derive(Deserialize)]
pub struct SavedSnapshot {
    pub revision: u64,
}

#[derive(Deserialize)]
pub struct SharedLibrary {
    pub addons: Option<Vec<crate::sync_format::LibraryAddon>>,
    pub revision: u64,
}

#[derive(Clone, Deserialize, serde::Serialize)]
pub struct CloudProfile {
    pub id: String,
    pub name: String,
}

impl CloudProfile {
    pub fn profile(&self) -> Result<crate::settings_backup::Profile, Error> {
        crate::settings_backup::Profile::from_cloud_id(&self.id)
            .ok_or_else(|| Error::new("Astral returned an invalid cloud profile."))
    }
}

#[derive(Deserialize)]
struct CloudProfiles {
    profiles: Vec<CloudProfile>,
}

#[derive(Clone)]
pub struct Service {
    agent: ureq::Agent,
    #[cfg(test)]
    test_origin: Option<String>,
}

impl Service {
    pub fn profiles(
        &self,
        session: &Session,
        client: crate::Client,
    ) -> Result<Vec<CloudProfile>, Error> {
        let result: CloudProfiles = self.request(
            "GET",
            &profile_manager_route(client, None),
            None,
            Some(session),
        )?;
        for profile in &result.profiles {
            profile.profile()?;
        }
        Ok(result.profiles)
    }

    pub fn save_profile(
        &self,
        session: &Session,
        client: crate::Client,
        id: Option<crate::settings_backup::Profile>,
        name: &str,
    ) -> Result<CloudProfile, Error> {
        let name = name.trim();
        if name.is_empty() || name.encode_utf16().count() > 40 || name.chars().any(char::is_control)
        {
            return Err(Error::new(
                "Enter a profile name between 1 and 40 characters.",
            ));
        }
        let method = if id.is_some() { "PATCH" } else { "POST" };
        let result: CloudProfile = self.request(
            method,
            &profile_manager_route(client, id),
            Some(json!({ "name": name })),
            Some(session),
        )?;
        let returned = result.profile()?;
        if id.map_or(
            !matches!(returned, crate::settings_backup::Profile::Custom(_)),
            |id| id != returned,
        ) {
            return Err(Error::new("Astral returned an unexpected profile."));
        }
        Ok(result)
    }

    pub fn delete_profile(
        &self,
        session: &Session,
        client: crate::Client,
        id: crate::settings_backup::Profile,
    ) -> Result<(), Error> {
        if !matches!(id, crate::settings_backup::Profile::Custom(_)) {
            return Err(Error::new("Regular and Handheld are built-in profiles."));
        }
        self.request::<Value>(
            "DELETE",
            &profile_manager_route(client, Some(id)),
            Some(json!({ "confirm": id.cloud_id() })),
            Some(session),
        )
        .map(|_| ())
    }

    pub fn new() -> Self {
        Self {
            #[cfg(test)]
            test_origin: None,
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .into(),
        }
    }

    #[cfg(test)]
    fn for_loopback(origin: &str) -> Self {
        let uri: ureq::http::Uri = origin.parse().expect("loopback URI");
        assert_eq!(uri.scheme_str(), Some("http"));
        assert_eq!(uri.host(), Some("127.0.0.1"));
        assert!(uri.port_u16().is_some());
        assert_eq!(uri.path(), "/");
        assert!(uri.query().is_none());
        Self {
            test_origin: Some(origin.trim_end_matches('/').into()),
            agent: ureq::Agent::config_builder()
                .https_only(false)
                .proxy(None)
                .max_redirects(0)
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .into(),
        }
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        route: &str,
        body: Option<Value>,
        session: Option<&Session>,
    ) -> Result<T, Error> {
        #[cfg(test)]
        let origin = self.test_origin.as_deref().unwrap_or(ORIGIN);
        #[cfg(not(test))]
        let origin = ORIGIN;
        let mut request = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{origin}{route}"))
            .header("Accept", "application/json");
        if let Some(session) = session {
            session.validate().map_err(Error::new)?;
            request = request.header("Authorization", format!("Bearer {}", session.token));
        }
        let bytes = match body {
            Some(body) => {
                request = request.header("Content-Type", "application/json");
                serde_json::to_vec(&body)
                    .map_err(|_| Error::new("Couldn’t prepare the Astral request."))?
            }
            None => Vec::new(),
        };
        let request = request
            .body(bytes)
            .map_err(|_| Error::new("Invalid Astral request."))?;
        let response = self.agent.run(request).map_err(|_| {
            Error::new("Couldn’t reach Astral. Check your connection and try again.")
        })?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        if !(200..300).contains(&status) {
            let message = body
                .with_config()
                .limit(16_384)
                .read_json::<Value>()
                .ok()
                .and_then(|error| error.get("error")?.as_str().map(str::to_owned))
                .filter(|message| message.len() <= 320 && !message.chars().any(char::is_control))
                .unwrap_or_else(|| "Astral couldn’t complete the request.".into());
            return Err(Error {
                status: Some(status),
                message,
            });
        }
        body.with_config()
            .limit(1_500_000)
            .read_json()
            .map_err(|_| Error::new("Astral returned an invalid response."))
    }

    pub fn begin(&self, label: &str) -> Result<Pair, Error> {
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| Error::new("Couldn’t start a secure sign-in."))?;
        let verifier = URL_SAFE_NO_PAD.encode(random);
        let challenge = pair_challenge(&verifier);
        let response: PairResponse = self.request(
            "POST",
            "/auth/device",
            Some(json!({ "challenge": challenge, "label": label })),
            None,
        )?;
        Pair::from_response(response, verifier)
    }

    pub fn poll(&self, pair: &Pair) -> Result<Option<Session>, Error> {
        let response = self.request(
            "POST",
            "/auth/device/poll",
            Some(json!({ "id": pair.id, "verifier": pair.verifier })),
            None,
        )?;
        match response {
            PollResponse::Session(session) => {
                session.validate().map_err(Error::new)?;
                Ok(Some(session))
            }
            PollResponse::Pending { pending: true } => Ok(None),
            PollResponse::Pending { pending: false } => {
                Err(Error::new("Astral returned an invalid sign-in response."))
            }
        }
    }

    pub fn logout(&self, session: &Session) -> Result<(), Error> {
        self.request::<Value>("POST", "/auth/logout", None, Some(session))
            .map(|_| ())
    }

    pub fn overview(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
    ) -> Result<Overview, Error> {
        self.request(
            "GET",
            &format!("/cloud/{}", profile_scope(client, profile)),
            None,
            Some(session),
        )
    }

    pub fn historical_snapshot(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
        id: &str,
    ) -> Result<HistoricalSnapshot, Error> {
        self.request(
            "GET",
            &history_route(client, profile, id)?,
            None,
            Some(session),
        )
    }

    pub fn promote_snapshot(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
        id: &str,
        revision: u64,
        hash: &str,
    ) -> Result<SavedSnapshot, Error> {
        self.request(
            "POST",
            &format!("{}/restore", history_route(client, profile, id)?),
            Some(json!({ "revision": revision, "hash": hash })),
            Some(session),
        )
    }

    pub fn delete_account_data(&self, session: &Session) -> Result<(), Error> {
        self.request::<Value>(
            "DELETE",
            "/account/data",
            Some(json!({ "confirm": "DELETE" })),
            Some(session),
        )
        .map(|_| ())
    }

    pub fn revoke_device(&self, session: &Session, id: &str) -> Result<(), Error> {
        if !(32..=64).contains(&id.len()) || !url_token(id) {
            return Err(Error::new("Invalid device."));
        }
        self.request::<Value>("DELETE", &format!("/devices/{id}"), None, Some(session))
            .map(|_| ())
    }

    pub fn current_snapshot(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
    ) -> Result<RemoteSnapshot, Error> {
        self.request("GET", &snapshot_route(client, profile), None, Some(session))
    }

    pub fn upload_snapshot(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
        revision: u64,
        snapshot: &crate::sync_format::Encoded,
    ) -> Result<SavedSnapshot, Error> {
        self.request(
            "PUT",
            &snapshot_route(client, profile),
            Some(json!({
                "revision": revision,
                "payload": snapshot.payload,
                "hash": snapshot.hash,
                "files": snapshot.files,
                "device": device_label(),
            })),
            Some(session),
        )
    }

    pub fn shared_library(
        &self,
        session: &Session,
        client: crate::Client,
    ) -> Result<SharedLibrary, Error> {
        self.request("GET", &library_route(client), None, Some(session))
    }

    pub fn profile_library(
        &self,
        session: &Session,
        client: crate::Client,
        profile: crate::settings_backup::Profile,
    ) -> Result<SharedLibrary, Error> {
        self.request(
            "GET",
            &format!("/profiles/{}/library", profile_scope(client, profile)),
            None,
            Some(session),
        )
    }

    pub fn save_library(
        &self,
        session: &Session,
        client: crate::Client,
        revision: u64,
        addons: &[crate::sync_format::LibraryAddon],
    ) -> Result<SharedLibrary, Error> {
        self.request(
            "PUT",
            &library_route(client),
            Some(json!({ "revision": revision, "addons": addons })),
            Some(session),
        )
    }
}

fn library_route(client: crate::Client) -> String {
    format!("/profiles/{}/library", client.label().to_ascii_lowercase())
}

fn profile_manager_route(
    client: crate::Client,
    id: Option<crate::settings_backup::Profile>,
) -> String {
    let mut route = format!("/sync-profiles/{}", client.label().to_ascii_lowercase());
    if let Some(id) = id {
        route.push('/');
        route.push_str(&id.cloud_id());
    }
    route
}

fn snapshot_route(client: crate::Client, profile: crate::settings_backup::Profile) -> String {
    format!("/profiles/{}/current", profile_scope(client, profile))
}

fn history_route(
    client: crate::Client,
    profile: crate::settings_backup::Profile,
    id: &str,
) -> Result<String, Error> {
    if !opaque_id(id) {
        return Err(Error::new("Invalid cloud backup."));
    }
    Ok(format!(
        "/profiles/{}/history/{id}",
        profile_scope(client, profile)
    ))
}

fn profile_scope(client: crate::Client, profile: crate::settings_backup::Profile) -> String {
    let flavor = client.label().to_ascii_lowercase();
    match profile {
        crate::settings_backup::Profile::Regular => flavor,
        crate::settings_backup::Profile::Handheld => format!("{flavor}-handheld"),
        crate::settings_backup::Profile::Custom(id) => format!("{flavor}-{}", id.simple()),
    }
}

impl Session {
    pub fn fingerprint(&self) -> String {
        format!("{:x}", Sha256::digest(self.token.as_bytes()))
    }
}

fn url_token(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn opaque_id(value: &str) -> bool {
    value.len() == 43 && url_token(value)
}

fn pair_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn device_label() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|name| !name.trim().is_empty())
        .or_else(|| {
            #[cfg(target_os = "macos")]
            {
                let output = std::process::Command::new("/bin/hostname").output().ok()?;
                if output.status.success() {
                    return String::from_utf8(output.stdout)
                        .ok()
                        .map(|name| name.trim().to_owned());
                }
            }
            None
        })
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "Mac".into()
            } else {
                "Windows PC".into()
            }
        })
        .chars()
        .filter(|ch| !ch.is_control())
        .take(40)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_session_metadata_accepts_the_worker_timestamps_and_older_records() {
        let device: Device = serde_json::from_value(json!({
            "id": "d".repeat(43), "label": "ROG Ally", "current": false,
            "created": 1794500000_i64, "last_seen": 1794680000_i64
        }))
        .unwrap();
        assert_eq!(device.last_seen, Some(1794680000));
        let older: Device = serde_json::from_value(
            json!({ "id": "c".repeat(43), "label": "Mac", "current": true }),
        )
        .unwrap();
        assert_eq!(older.created.or(older.last_seen), None);
    }

    #[test]
    fn browser_pairing_accepts_only_the_worker_route_for_the_returned_pair() {
        let id = "v".repeat(43);
        let verifier = "w".repeat(43);
        let url = format!("{ORIGIN}/auth/start?id={id}");
        let pair = Pair::from_response(
            PairResponse {
                id: id.clone(),
                url: url.clone(),
            },
            verifier.clone(),
        )
        .unwrap();
        assert_eq!(pair.url, url);
        for url in [
            format!("https://example.invalid/auth/start?id={id}"),
            format!("{ORIGIN}/auth/start?id={id}&next=https://example.invalid"),
            format!("{ORIGIN}/auth/start?id={}", "x".repeat(43)),
        ] {
            assert!(
                Pair::from_response(
                    PairResponse {
                        id: id.clone(),
                        url
                    },
                    verifier.clone()
                )
                .is_err()
            );
        }
        assert_eq!(
            pair_challenge(&"v".repeat(43)),
            "7w_YNF9DSfIdPf_pRjSq646_kPr-2-o9NAl16JGghdM"
        );
    }
}
