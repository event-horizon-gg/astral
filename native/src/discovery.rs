use crate::{Astral, Page, releases};
use gpui::Context;
use serde::Deserialize;
use std::{cmp::Reverse, collections::HashSet, time::Instant};

const SOURCES: [(Option<&str>, &str); 4] = [
    (None, "All sources"),
    (Some("GitHub"), "GitHub"),
    (Some("Astral"), "Astral"),
    (Some("Manual"), "Manual sources"),
];

pub(crate) struct SourcePicker {
    pub focus: gpui::FocusHandle,
    pub bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    search: crate::type_ahead::TypeAhead,
    pending: Option<uuid::Uuid>,
}

impl SourcePicker {
    pub fn new(cx: &mut gpui::App) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            bounds: Default::default(),
            search: Default::default(),
            pending: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.pending = None;
        self.search.reset();
    }
}

impl Astral {
    pub(crate) fn choose_discovery_source(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture || self.page != Page::Discover || self.discovery_picker.pending.is_some() {
            return;
        }
        let Some(bounds) = self.discovery_picker.bounds.get() else {
            return;
        };
        let labels = SOURCES
            .iter()
            .map(|(_, label)| (*label).to_owned())
            .collect();
        let selected = SOURCES
            .iter()
            .position(|(value, _)| *value == self.discovery_source)
            .unwrap_or(0);
        match crate::native_select::choose(
            window,
            cx,
            labels,
            selected,
            crate::native_select::Anchor {
                bounds,
                font_size: gpui::px(12.),
            },
        ) {
            Ok(task) => self.await_discovery_source(task, window, cx),
            Err(error) => self.show_notification(error, true, cx),
        }
    }

    fn await_discovery_source(
        &mut self,
        choice: impl std::future::Future<Output = Option<usize>> + 'static,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.page != Page::Discover || self.discovery_picker.pending.is_some() {
            return;
        }
        let request = uuid::Uuid::new_v4();
        let client = self.preferences.client;
        self.discovery_picker.pending = Some(request);
        cx.spawn_in(window, async move |this, cx| {
            let choice = choice.await;
            let _ = this.update_in(cx, |app, window, cx| {
                if app.discovery_picker.pending != Some(request) {
                    return;
                }
                app.discovery_picker.pending = None;
                if app.page == Page::Discover && app.preferences.client == client {
                    if let Some((value, _)) = choice.and_then(|index| SOURCES.get(index)) {
                        app.discovery_source = *value;
                    }
                    app.discovery_picker.focus.focus(window);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn discovery_source_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.page != Page::Discover
            || !self.discovery_picker.focus.is_focused(window)
            || self.discovery_picker.pending.is_some()
        {
            return false;
        }
        let labels: Vec<_> = SOURCES.iter().map(|(_, label)| *label).collect();
        let selected = SOURCES
            .iter()
            .position(|(value, _)| *value == self.discovery_source)
            .unwrap_or(0);
        match crate::native_select::closed_key(
            &event.keystroke,
            cfg!(target_os = "macos"),
            &labels,
            selected,
            &mut self.discovery_picker.search,
            Instant::now(),
        ) {
            crate::native_select::KeyResult::Open => self.choose_discovery_source(window, cx),
            crate::native_select::KeyResult::Select(index) => {
                self.discovery_source = SOURCES[index].0;
                cx.notify();
            }
            crate::native_select::KeyResult::Handled => {}
            crate::native_select::KeyResult::Ignore => return false,
        }
        true
    }
}

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
        if self.page != Page::Discover || self.addon_controls_busy() {
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

#[cfg(all(test, feature = "headless-ui-tests"))]
mod ui_tests {
    use super::*;
    use crate::{
        Client,
        ui_tests::{deferred, press},
    };
    use gpui::{TestAppContext, px, size};

    #[gpui::test]
    fn source_prefixes_select_without_toolbar_menus_and_keep_focus(cx: &mut TestAppContext) {
        for light in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut app = Astral::new(
                    [None, None],
                    Some(directory.path()),
                    Some(Client::Retail),
                    light,
                    cx,
                );
                app.page = Page::Discover;
                app
            });
            cx.simulate_resize(size(px(1250.), px(700.)));
            cx.run_until_parked();
            let bounds = cx.debug_bounds("discover-source-trigger").unwrap();
            view.update_in(cx, |app, window, _| {
                app.discovery_picker.focus.focus(window)
            });
            press(cx, "g");
            view.update_in(cx, |app, window, _| {
                assert_eq!(app.discovery_source, Some("GitHub"));
                assert!(app.discovery_picker.focus.is_focused(window));
                assert!(app.menu.is_none());
                assert!(app.discovery_picker.pending.is_none());
            });
            assert_eq!(
                cx.debug_bounds("discover-source-trigger").unwrap().size,
                bounds.size
            );
            if !cfg!(target_os = "macos") {
                press(cx, "end");
                assert_eq!(
                    view.read_with(cx, |app, _| app.discovery_source),
                    Some("Manual")
                );
                press(cx, "home");
                assert_eq!(view.read_with(cx, |app, _| app.discovery_source), None);
            }
        }
    }

    #[gpui::test]
    fn source_popup_result_preserves_cancel_and_cannot_outlive_the_page(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut app = Astral::new(
                [None, None],
                Some(directory.path()),
                Some(Client::Retail),
                false,
                cx,
            );
            app.page = Page::Discover;
            app
        });
        for (choice, expected) in [
            (Some(3), Some("Manual")),
            (None, Some("Manual")),
            (Some(usize::MAX), Some("Manual")),
            (Some(0), None),
        ] {
            view.update_in(cx, |app, window, cx| {
                app.await_discovery_source(std::future::ready(choice), window, cx)
            });
            cx.run_until_parked();
            view.update_in(cx, |app, window, _| {
                assert_eq!(app.discovery_source, expected);
                assert!(app.discovery_picker.pending.is_none());
                assert!(app.discovery_picker.focus.is_focused(window));
            });
        }
        let (choice, finish) = deferred();
        view.update_in(cx, |app, window, cx| {
            app.await_discovery_source(choice, window, cx);
            app.navigate(Page::Library, window, cx);
            app.navigate(Page::Discover, window, cx);
            app.brand_focus.focus(window);
        });
        finish(Some(1));
        cx.run_until_parked();
        view.update_in(cx, |app, window, _| {
            assert_eq!(app.discovery_source, None);
            assert!(app.discovery_picker.pending.is_none());
            assert!(app.brand_focus.is_focused(window));
        });
    }
}
