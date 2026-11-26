use std::path::Path;
use std::time::Duration;

use hive_core::{Error, Handle, Result, Selector, Visibility};
use serde::Deserialize;

const DEFAULT: &str = include_str!("../selectors.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub tab: String,
    pub account_label: String,
    pub named_button: String,
    pub switch_sheet: String,
    pub account_choice: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Picker {
    pub create: String,
    pub gallery: String,
    pub videos_tab: String,
    pub photos_tab: String,
    pub grids: Vec<String>,
    pub pick_circle: String,
    pub clear_picks: String,
    pub next: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Editor {
    pub sound_pill: String,
    pub next: String,
    pub sound_tab: String,
    pub icon_button: String,
    pub sound_search: String,
    pub sound_search_go: String,
    pub results_list: String,
    pub nested_list: String,
    pub use_spot: (f64, f64),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostScreen {
    pub post: String,
    pub title: Vec<String>,
    pub caption: Vec<String>,
    pub visibility_row: String,
    pub visibility_sheet: String,
    pub visibility_names: [String; 3],
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timeouts {
    pub launch: f64,
    pub switch: f64,
    pub gallery: f64,
    pub editor: f64,
    pub field: f64,
    pub sound: f64,
    pub posted: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pauses {
    pub min: u64,
    pub max: u64,
    pub pick_min: u64,
    pub pick_max: u64,
}

/// TikTok's app as hive sees it: selectors, timeouts, pauses (see `selectors.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub package: String,
    pub profile: Profile,
    pub picker: Picker,
    pub editor: Editor,
    pub post_screen: PostScreen,
    pub timeouts: Timeouts,
    pub pauses: Pauses,
}

fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

impl Default for App {
    fn default() -> Self {
        toml::from_str(DEFAULT).expect("the built-in selectors.toml is valid")
    }
}

impl App {
    /// The built-in selectors, with `overrides` (a TOML file holding only the keys to change)
    /// laid over them when it exists.
    pub fn load(overrides: Option<&Path>) -> Result<Self> {
        let mut base: toml::Value = toml::from_str(DEFAULT).expect("valid built-in selectors");
        if let Some(path) = overrides.filter(|p| p.is_file()) {
            let text = std::fs::read_to_string(path)?;
            let over: toml::Value = toml::from_str(&text)
                .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
            merge(&mut base, over);
        }
        base.try_into()
            .map_err(|e| Error::Invalid(format!("TikTok selectors: {e}")))
    }

    pub fn secs(t: f64) -> Duration {
        Duration::from_secs_f64(t.max(0.0))
    }

    pub fn account_choice(&self, handle: &Handle) -> Selector {
        // handles are letters, digits, "_" and "."; a "." has to match itself
        let literal = handle.as_str().replace('.', "[.]");
        Selector::ui(self.profile.account_choice.replace("{handle}", &literal))
    }

    pub fn visibility_name(&self, v: Visibility) -> &str {
        let names = &self.post_screen.visibility_names;
        match v {
            Visibility::Everyone => &names[0],
            Visibility::Friends => &names[1],
            Visibility::OnlyYou => &names[2],
        }
    }

    pub fn visibility_choice(&self, v: Visibility) -> Selector {
        let name = self
            .visibility_name(v)
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        Selector::ui(format!("new UiSelector().text(\"{name}\")"))
    }

    /// Which visibility the row names ("Only you can view this post").
    pub fn shown_visibility(&self, row: &str) -> Option<Visibility> {
        let row = row.trim();
        [
            Visibility::Everyone,
            Visibility::Friends,
            Visibility::OnlyYou,
        ]
        .into_iter()
        .find(|v| {
            let name = self.visibility_name(*v);
            row == name || row.starts_with(&format!("{name} "))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_selectors_load_and_overrides_apply() {
        let app = App::default();
        assert_eq!(app.package, "com.zhiliaoapp.musically");
        let dir = std::env::temp_dir().join(format!("hive-tt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tiktok.toml");
        std::fs::write(&file, "package = \"com.ss.android.ugc.trill\"\n[profile]\ntab = 'new UiSelector().text(\"Profil\")'\n").unwrap();
        let app = App::load(Some(&file)).unwrap();
        assert_eq!(app.package, "com.ss.android.ugc.trill");
        assert!(app.profile.tab.contains("Profil"));
        assert_eq!(
            app.profile.switch_sheet,
            App::default().profile.switch_sheet
        );
        let h = Handle::new("gta6.clips").unwrap();
        assert_eq!(
            app.account_choice(&h),
            Selector::ui("new UiSelector().descriptionMatches(\"(?i)@?gta6[.]clips\")")
        );
        assert_eq!(
            app.shown_visibility("Only you can view this post"),
            Some(Visibility::OnlyYou)
        );
    }
}
