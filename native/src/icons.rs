use std::borrow::Cow;

pub struct Icons;

impl gpui::AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let shape = match path {
            "search" => r#"<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>"#,
            "chevron" => r#"<path d="m6 9 6 6 6-6"/>"#,
            "more" => {
                r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#
            }
            "refresh" => {
                r#"<path d="M20 7a9 9 0 0 0-15-1L3 8m0-5v5h5m-4 9a9 9 0 0 0 15 1l2-2m0 5v-5h-5"/>"#
            }
            "folder" => {
                r#"<path d="M3 18V6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v2M3 18l3-8h15l-3 8z"/>"#
            }
            "close" => r#"<path d="m6 6 12 12M18 6 6 18"/>"#,
            "check" => r#"<path d="m5 12 4 4L19 6"/>"#,
            "sun" => {
                r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M19 5l-1.5 1.5m-11 11L5 19"/>"#
            }
            "moon" => r#"<path d="M20.5 14a8.5 8.5 0 0 1-10.5-10.5A8.5 8.5 0 1 0 20.5 14Z"/>"#,
            "github" => {
                return Ok(Some(Cow::Borrowed(include_bytes!(
                    "../../assets/providers/github.svg"
                ))));
            }
            _ => return Ok(None),
        };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{shape}</svg>"#
        );
        Ok(Some(Cow::Owned(svg.into_bytes())))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}
