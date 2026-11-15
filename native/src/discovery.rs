use crate::{Astral, Page, releases};
use gpui::Context;
use serde::Deserialize;
use std::{cmp::Reverse, collections::HashSet, time::Instant};

#[derive(Deserialize)]
pub struct Project {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    stargazers_count: u64,
    archived: bool,
    disabled: bool,
    fork: bool,
}

#[derive(Deserialize)]
struct SearchResponse {
    items: Vec<Project>,
}

pub struct Search {
    query: String,
    id: uuid::Uuid,
    result: Option<(Instant, Result<Vec<Project>, String>)>,
}

fn clean_query(query: &str) -> Result<String, String> {
    let cleaned: String = query
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if !(2..=80).contains(&cleaned.chars().count()) {
        return Err("Enter between 2 and 80 characters to search GitHub.".into());
    }
    Ok(cleaned)
}

fn valid_project(project: &Project) -> bool {
    let Some((owner, repo)) = project.full_name.split_once('/') else {
        return false;
    };
    !project.archived
        && !project.disabled
        && !project.fork
        && owner.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        && owner
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        && !matches!(repo, "" | "." | "..")
        && repo
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
}

fn search(query: &str) -> Result<Vec<Project>, String> {
    let agent = releases::client();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for qualifier in ["topic:world-of-warcraft", "language:Lua"] {
        let response: SearchResponse = agent
            .get("https://api.github.com/search/repositories")
            .query(
                "q",
                format!("{query} in:name,description {qualifier} fork:false archived:false"),
            )
            .query("per_page", "20")
            .header("User-Agent", "Astral-Native")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(403 | 429) => {
                    "GitHub search limit reached. Try again in a minute.".into()
                }
                _ => format!("Couldn't search GitHub. {error}"),
            })?
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_json()
            .map_err(|error| format!("Couldn't read GitHub results. {error}"))?;
        items.extend(response.items.into_iter().filter(|project| {
            valid_project(project) && seen.insert(project.full_name.to_ascii_lowercase())
        }));
    }
    items.sort_by_key(|project| {
        (
            !project.name.eq_ignore_ascii_case(query),
            Reverse(project.stargazers_count),
        )
    });
    items.truncate(30);
    Ok(items)
}

impl Astral {
    pub fn search_github(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Discover || self.discovery_search_busy() || self.addon_batch_running()
        {
            return;
        }
        let query = match clean_query(&self.query) {
            Ok(query) => query,
            Err(error) => {
                self.github_search = Some(Search {
                    query: self.query.clone(),
                    id: uuid::Uuid::new_v4(),
                    result: Some((Instant::now(), Err(error))),
                });
                cx.notify();
                return;
            }
        };
        if self.github_search.as_ref().is_some_and(|state| {
            state.query == query
                && matches!(&state.result, Some((at, Ok(_))) if at.elapsed().as_secs() < 600)
        }) {
            return;
        }
        let id = uuid::Uuid::new_v4();
        self.github_search = Some(Search {
            query: query.clone(),
            id,
            result: None,
        });
        let task = cx
            .background_executor()
            .spawn(async move { search(&query) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if let Some(state) = app.github_search.as_mut().filter(|state| state.id == id) {
                    state.result = Some((Instant::now(), result));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn discovery_search_busy(&self) -> bool {
        self.github_search
            .as_ref()
            .is_some_and(|state| state.result.is_none())
    }

    pub(crate) fn discovery_search_error(&self) -> Option<&str> {
        self.github_search
            .as_ref()?
            .result
            .as_ref()?
            .1
            .as_ref()
            .err()
            .map(String::as_str)
    }

    pub(crate) fn discovery_projects(&self) -> &[Project] {
        self.github_search
            .as_ref()
            .and_then(|state| state.result.as_ref())
            .and_then(|(_, result)| result.as_ref().ok())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}
