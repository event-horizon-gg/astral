use std::borrow::Cow;

pub struct Icons;

impl gpui::AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "github" => Some(include_bytes!("../../assets/providers/github.svg")),
            "curseforge" => Some(include_bytes!("../../assets/providers/curseforge.svg")),
            _ => crate::ui_icons::load(path),
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}
