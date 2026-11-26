//! The TikTok flow against a pretend TikTok: a few screens that answer the built-in selectors
//! the way the real app does, so the flow's logic is checked without a phone.

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use hive_core::{
    Device, Direction, ElementId, Error, Handle, Media, Post, Rect, Result, Selector, Serial, Ui,
    Visibility, keycode,
};
use hive_tiktok::{App, TikTok};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Switcher,
    Camera,
    Picker,
    Editor,
    Posting,
    Done,
}

struct FakeTikTok {
    screen: Screen,
    current: String,
    accounts: Vec<String>,
    gallery: usize,
    picked: Vec<usize>,
    caption: String,
    title: String,
    visibility: String,
    sheet: bool,
    posted: bool,
}

impl FakeTikTok {
    fn new(current: &str, accounts: &[&str]) -> Self {
        Self {
            screen: Screen::Home,
            current: current.into(),
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            gallery: 7,
            picked: vec![],
            caption: String::new(),
            title: String::new(),
            visibility: "Everyone".into(),
            sheet: false,
            posted: false,
        }
    }

    fn cell(i: usize) -> Rect {
        Rect {
            x: (i % 3) as i32 * 360,
            y: 300 + (i / 3) as i32 * 360,
            width: 360,
            height: 360,
        }
    }

    fn circle(i: usize) -> Rect {
        let c = Self::cell(i);
        Rect {
            x: c.x + 300,
            y: c.y + 10,
            width: 50,
            height: 50,
        }
    }
}

fn ids(v: &[&str]) -> Vec<ElementId> {
    v.iter().map(|s| ElementId(s.to_string())).collect()
}

impl Ui for FakeTikTok {
    fn find_all(
        &mut self,
        selector: &Selector,
        within: Option<&ElementId>,
    ) -> Result<Vec<ElementId>> {
        use Screen::*;
        let s = match selector {
            Selector::XPath(_) => {
                return Ok(match (within.map(|e| e.0.as_str()), self.screen) {
                    (Some("grid"), Picker) => (0..self.gallery)
                        .map(|i| ElementId(format!("cell:{i}")))
                        .collect(),
                    _ => vec![],
                });
            }
            Selector::UiAutomator(s) => s.as_str(),
        };
        if within.is_some_and(|w| w.0 == "grid") && s.contains("android.widget.Button") {
            return Ok((0..self.gallery)
                .map(|i| ElementId(format!("circle:{i}")))
                .collect());
        }
        let found = match self.screen {
            Home if s.contains("description(\"Profile\")") => ids(&["tab_profile"]),
            Home if s.contains("textStartsWith(\"@\")") => ids(&["label"]),
            Home if s.contains("clickable(true)") && s.contains("Button") => ids(&["name_btn"]),
            Home if s.contains("description(\"Create\")") => ids(&["create"]),
            Switcher if s.contains("Switch account") => ids(&["sheet"]),
            Switcher if s.contains("descriptionMatches") => self
                .accounts
                .iter()
                .filter(|a| s.contains(&a.replace('.', "[.]")))
                .map(|a| ElementId(format!("choice:{a}")))
                .collect(),
            Camera if s.contains("upload_hot_area") => ids(&["gallery"]),
            Picker if s.contains("text(\"Videos\")") || s.contains("text(\"Photos\")") => {
                ids(&["tab"])
            }
            Picker if s.contains("GridView") => ids(&["grid"]),
            Picker if s.contains("Next (") && !self.picked.is_empty() => ids(&["next_picker"]),
            Editor if s.contains("tv_top_text") => ids(&["pill"]),
            Editor if s.contains("text(\"Next\")") => ids(&["editor_next"]),
            Posting if s.contains("text(\"Post\")") => ids(&["post"]),
            Posting if s.contains("Add a catchy title") => ids(&["title"]),
            Posting if s.contains("Writing a long description") => ids(&["caption"]),
            Posting if s.contains("can view this post\")") && s.contains("textMatches") => {
                ids(&["vis_row"])
            }
            Posting if self.sheet && s.contains("Who can view this post") => ids(&["vis_sheet"]),
            Posting if self.sheet && s.contains("text(\"Friends\")") => ids(&["vis:Friends"]),
            _ if s.contains("index(0)") => ids(&["root"]),
            _ => vec![],
        };
        Ok(found)
    }

    fn text(&mut self, el: &ElementId) -> Result<String> {
        Ok(match el.0.as_str() {
            "label" => format!("@{}", self.current),
            "name_btn" => "Alex ▾".into(),
            "next_picker" => format!("Next ({})", self.picked.len()),
            "pill" => "Add sound".into(),
            "vis_row" => format!("{} can view this post", self.visibility),
            c if c.starts_with("circle:") => {
                let i: usize = c[7..].parse().unwrap();
                self.picked
                    .iter()
                    .position(|p| *p == i)
                    .map(|n| (n + 1).to_string())
                    .unwrap_or_default()
            }
            _ => String::new(),
        })
    }

    fn rect(&mut self, el: &ElementId) -> Result<Rect> {
        Ok(match el.0.as_str() {
            "label" => Rect {
                x: 100,
                y: 500,
                width: 300,
                height: 40,
            },
            "name_btn" => Rect {
                x: 100,
                y: 440,
                width: 300,
                height: 50,
            },
            "grid" => Rect {
                x: 0,
                y: 300,
                width: 1080,
                height: 1800,
            },
            c if c.starts_with("cell:") => Self::cell(c[5..].parse().unwrap()),
            c if c.starts_with("circle:") => Self::circle(c[7..].parse().unwrap()),
            _ => Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
        })
    }

    fn click(&mut self, el: &ElementId) -> Result<()> {
        use Screen::*;
        match el.0.as_str() {
            "name_btn" => self.screen = Switcher,
            c if c.starts_with("choice:") => {
                self.current = c[7..].into();
                self.screen = Home;
            }
            "create" => self.screen = Camera,
            "gallery" => self.screen = Picker,
            "next_picker" => self.screen = Editor,
            "editor_next" => self.screen = Posting,
            "vis_row" => self.sheet = true,
            "vis:Friends" => self.visibility = "Friends".into(),
            "post" => {
                self.posted = true;
                self.screen = Done;
            }
            _ => {}
        }
        Ok(())
    }

    fn set_text(&mut self, el: &ElementId, text: &str) -> Result<()> {
        match el.0.as_str() {
            "caption" => self.caption = text.into(),
            "title" => self.title = text.into(),
            _ => {}
        }
        Ok(())
    }

    fn tap(&mut self, x: i32, y: i32) -> Result<()> {
        if self.screen == Screen::Picker {
            let hit = (0..self.gallery).find(|i| {
                let r = Self::circle(*i);
                r.x <= x && x <= r.right() && r.y <= y && y <= r.bottom()
            });
            if let Some(i) = hit {
                self.picked.push(i);
            }
        }
        Ok(())
    }

    fn drag(&mut self, _: (i32, i32), _: (i32, i32), _: u32) -> Result<()> {
        Ok(())
    }
    fn swipe(&mut self, _: Rect, _: Direction) -> Result<()> {
        Ok(())
    }
    fn key(&mut self, code: u32) -> Result<()> {
        if code == keycode::BACK {
            if self.sheet {
                self.sheet = false;
            } else if self.screen == Screen::Switcher {
                self.screen = Screen::Home;
            }
        }
        Ok(())
    }
    fn screen_size(&mut self) -> Result<(i32, i32)> {
        Ok((1080, 2400))
    }
    fn source(&mut self) -> Result<String> {
        Ok("<hierarchy/>".into())
    }
    fn is_alive(&mut self) -> bool {
        true
    }
}

struct FakeDevice {
    serial: Serial,
    calls: RefCell<Vec<String>>,
}

impl Device for FakeDevice {
    fn serial(&self) -> &Serial {
        &self.serial
    }
    fn shell(&self, script: &str) -> Result<String> {
        self.calls.borrow_mut().push(script.into());
        Ok(if script.contains("input_method") {
            "0".into()
        } else {
            String::new()
        })
    }
    fn shell_timeout(&self, script: &str, _: Duration) -> Result<String> {
        self.shell(script)
    }
    fn push(&self, _: &Path, _: &str) -> Result<()> {
        Ok(())
    }
}

fn quick_app() -> App {
    let mut app = App::default();
    app.pauses.min = 0;
    app.pauses.max = 0;
    app.pauses.pick_min = 0;
    app.pauses.pick_max = 0;
    let t = &mut app.timeouts;
    (
        t.launch, t.switch, t.gallery, t.editor, t.field, t.sound, t.posted,
    ) = (0.5, 0.5, 0.5, 0.5, 0.3, 0.3, 0.5);
    app
}

fn post(account: &str, media: Media) -> Post {
    Post {
        account: Handle::new(account).unwrap(),
        media,
        caption: "GTA 5 vs GTA 6 ✈️\n#gta6 #lasalle".into(),
        title: Some("Les avions".into()),
        sound: None,
        visibility: Visibility::Friends,
        draft: false,
    }
}

fn device() -> FakeDevice {
    FakeDevice {
        serial: Serial::new("PHONE1").unwrap(),
        calls: RefCell::new(vec![]),
    }
}

#[test]
fn switches_account_picks_in_order_and_posts() {
    let app = quick_app();
    let mut ui = FakeTikTok::new("animedailyclips", &["animedailyclips", "momentsdegta6"]);
    let dev = device();
    let mut said = Vec::new();
    let mut say = |s: &str| said.push(s.to_owned());
    let media = Media::Slideshow {
        images: vec![
            "1.jpg".into(),
            "2.jpg".into(),
            "3.jpg".into(),
            "4.jpg".into(),
        ],
    };
    let report = TikTok::new(&mut ui, &dev, &app, &mut say)
        .publish(&post("momentsdegta6", media))
        .unwrap();
    assert!(report.posted, "{report:?}");
    assert!(report.problems.is_empty(), "{report:?}");
    assert_eq!(ui.current, "momentsdegta6");
    assert_eq!(ui.picked, [0, 1, 2, 3]);
    assert_eq!(ui.caption, "GTA 5 vs GTA 6 ✈️\n#gta6 #lasalle");
    assert_eq!(ui.title, "Les avions");
    assert_eq!(ui.visibility, "Friends");
    assert_eq!(report.visibility, Some(Visibility::Friends));
    assert!(dev.calls.borrow()[0].starts_with("monkey -p com.zhiliaoapp.musically"));
}

#[test]
fn an_account_the_phone_lacks_is_never_posted_on() {
    let app = quick_app();
    let mut ui = FakeTikTok::new("animedailyclips", &["animedailyclips"]);
    let dev = device();
    let mut say = |_: &str| {};
    let err = TikTok::new(&mut ui, &dev, &app, &mut say)
        .publish(&post(
            "momentsdegta6",
            Media::Video {
                file: "a.mp4".into(),
            },
        ))
        .unwrap_err();
    match err {
        Error::NotOnPhone {
            handle, current, ..
        } => {
            assert_eq!(handle.as_str(), "momentsdegta6");
            assert_eq!(current, "@animedailyclips");
        }
        other => panic!("{other:?}"),
    }
    assert!(!ui.posted);
    assert_eq!(ui.screen, Screen::Home, "the switcher was closed again");
}

#[test]
fn a_draft_stops_before_post() {
    let app = quick_app();
    let mut ui = FakeTikTok::new("momentsdegta6", &["momentsdegta6"]);
    let dev = device();
    let mut say = |_: &str| {};
    let mut p = post(
        "momentsdegta6",
        Media::Video {
            file: "a.mp4".into(),
        },
    );
    p.draft = true;
    let report = TikTok::new(&mut ui, &dev, &app, &mut say)
        .publish(&p)
        .unwrap();
    assert!(!report.posted && !ui.posted);
    assert_eq!(ui.picked, [0]);
    assert_eq!(ui.screen, Screen::Posting);
}
