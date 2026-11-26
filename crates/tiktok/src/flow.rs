use std::thread;
use std::time::{Duration, Instant};

use hive_core::{
    Cancel, Device, Direction, ElementId, Error, Handle, Media, Post, PostReport, Rect, Result,
    Selector, Ui, Visibility, keycode,
};
use rand::Rng;

use crate::app::App;

/// The picker's box and its selection circles, each with its text.
type Circles = (Rect, Vec<(Rect, String)>);

const POLL: Duration = Duration::from_millis(300);
/// Times a list is read again when the app redraws it mid-read.
const REDRAWS: u32 = 5;

/// TikTok on one phone, driven step by step. Every step looks before it taps, and anything it
/// can't do becomes a problem in the report rather than a guess: with any problem, Post is never
/// tapped, and the phone is left on the post screen for a person to finish.
pub struct TikTok<'a, U: Ui, D: Device> {
    ui: &'a mut U,
    dev: &'a D,
    app: &'a App,
    say: &'a mut dyn FnMut(&str),
    cancel: Cancel,
}

fn sel(s: &str) -> Selector {
    Selector::ui(s)
}

fn first_line(e: &Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_owned()
}

/// Grid cells top to bottom, then left to right, as a person reads them.
fn in_order(mut cells: Vec<(ElementId, Rect)>) -> Vec<(ElementId, Rect)> {
    cells.sort_by_key(|(_, r)| (r.y, r.x));
    cells
}

/// Whether the cells reach the picker's bottom: there may be more below.
fn fills(grid: Rect, boxes: &[Rect]) -> bool {
    boxes
        .iter()
        .map(Rect::bottom)
        .max()
        .is_some_and(|b| b >= grid.bottom())
}

const PICK_FIX: &str = "pick the files yourself in TikTok's picker (the newest ones), in order";
const VISIBILITY_FIX: &str = "choose who can see the post yourself";

impl<'a, U: Ui, D: Device> TikTok<'a, U, D> {
    pub fn new(ui: &'a mut U, dev: &'a D, app: &'a App, say: &'a mut dyn FnMut(&str)) -> Self {
        Self {
            ui,
            dev,
            app,
            say,
            cancel: Cancel::new(),
        }
    }

    /// Stop at the next step once `cancel` is cancelled (never after Post is tapped).
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    // --- looking and tapping ----------------------------------------------------------------

    fn pause_range(&self, min: u64, max: u64) {
        if max > 0 {
            let ms = rand::rng().random_range(min.min(max)..=max);
            thread::sleep(Duration::from_millis(ms));
        }
    }

    fn pause(&self) {
        self.pause_range(self.app.pauses.min, self.app.pauses.max);
    }

    /// The first element one of `selectors` (tried in order) finds, waiting up to `timeout`.
    fn find(&mut self, selectors: &[Selector], timeout: f64) -> Result<Option<ElementId>> {
        let deadline = Instant::now() + App::secs(timeout);
        loop {
            self.cancel.check()?;
            for s in selectors {
                if let Some(e) = self.ui.find_all(s, None)?.into_iter().next() {
                    return Ok(Some(e));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(POLL);
        }
    }

    fn find1(&mut self, selector: &str, timeout: f64) -> Result<Option<ElementId>> {
        self.find(&[sel(selector)], timeout)
    }

    fn tap(&mut self, el: &ElementId) -> Result<()> {
        self.pause();
        self.cancel.check()?;
        self.ui.click(el)
    }

    fn tap_at(&mut self, x: i32, y: i32, picking: bool) -> Result<()> {
        if picking {
            self.pause_range(self.app.pauses.pick_min, self.app.pauses.pick_max);
        } else {
            self.pause();
        }
        self.cancel.check()?;
        self.ui.tap(x, y)
    }

    fn back(&mut self) -> Result<()> {
        self.pause();
        self.ui.key(keycode::BACK)
    }

    /// Whether `selector` stops finding anything within `timeout`.
    fn gone(&mut self, selector: &str, timeout: f64) -> Result<bool> {
        let deadline = Instant::now() + App::secs(timeout);
        while !self.ui.find_all(&sel(selector), None)?.is_empty() {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(POLL);
        }
        Ok(true)
    }

    // --- the app and its account ------------------------------------------------------------

    /// Open TikTok (or bring it to the front).
    pub fn launch(&mut self) -> Result<()> {
        let out = self.dev.shell(&format!(
            "monkey -p {} -c android.intent.category.LAUNCHER 1",
            self.app.package
        ))?;
        if out.contains("No activities found") {
            return Err(Error::App(format!(
                "TikTok ({}) isn't installed on the phone",
                self.app.package
            )));
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<()> {
        self.dev
            .shell(&format!("am force-stop {}", self.app.package))?;
        self.launch()
    }

    /// The Profile tab scrolled to the top, and its "@handle" label. With `restart`, TikTok stuck
    /// on a screen without its tabs (the camera, a half-made post) is restarted once.
    fn profile_top(&mut self, restart: bool) -> Result<Option<(ElementId, String)>> {
        let t = self.app.timeouts.clone();
        let mut tab = self.find1(&self.app.profile.tab.clone(), t.switch)?;
        if tab.is_none() && restart {
            self.restart()?;
            tab = self.find1(&self.app.profile.tab.clone(), t.launch)?;
        }
        let Some(tab) = tab else {
            return Err(Error::App(
                "couldn't find TikTok's Profile tab: is the app on its home screen, and in English?".into(),
            ));
        };
        self.tap(&tab)?;
        for _ in 0..3 {
            if let Some(label) = self.find1(&self.app.profile.account_label.clone(), t.field)? {
                let text = self.ui.text(&label)?.trim().to_owned();
                return Ok(Some((label, text)));
            }
            let (w, h) = self.ui.screen_size()?;
            self.ui.swipe(
                Rect {
                    x: w / 10,
                    y: h / 4,
                    width: w * 8 / 10,
                    height: h / 2,
                },
                Direction::Down,
            )?;
        }
        Ok(None)
    }

    /// The account TikTok is on.
    pub fn current_account(&mut self) -> Result<Handle> {
        self.launch()?;
        match self.profile_top(true)? {
            Some((_, text)) => Handle::new(&text),
            None => Err(Error::App(
                "couldn't read which account TikTok is on".into(),
            )),
        }
    }

    /// The account's name: the button with text just above the "@handle", over its middle ("Edit"
    /// sits beside the name, just as high).
    fn name_above(&mut self, label: &ElementId) -> Result<Option<ElementId>> {
        let l = self.ui.rect(label)?;
        let middle = l.x + l.width / 2;
        let mut best: Option<(ElementId, i32)> = None;
        for b in self
            .ui
            .find_all(&sel(&self.app.profile.named_button), None)?
        {
            let r = self.ui.rect(&b)?;
            let over = r.x <= middle && middle <= r.right();
            if over
                && (l.y - 80..=l.y + 10).contains(&r.bottom())
                && !self.ui.text(&b)?.trim().is_empty()
                && best.as_ref().is_none_or(|(_, bottom)| r.bottom() > *bottom)
            {
                best = Some((b, r.bottom()));
            }
        }
        Ok(best.map(|(b, _)| b))
    }

    /// Make sure TikTok is on `handle`, switching among the app's accounts. Never leaves a post
    /// on another account: fails with `NotOnPhone` when the app doesn't hold it.
    pub fn switch_to(&mut self, handle: &Handle) -> Result<()> {
        let t = self.app.timeouts.clone();
        let Some((label, text)) = self.profile_top(true)? else {
            return Err(Error::App(
                "couldn't read which account TikTok is on".into(),
            ));
        };
        if Handle::new(&text).ok().as_ref() == Some(handle) {
            return Ok(());
        }
        (self.say)(&format!("TikTok is on {text}: switching to @{handle}"));
        let switcher = self.name_above(&label)?;
        let mut choice = None;
        if let Some(s) = &switcher {
            self.tap(s)?;
            if self
                .find1(&self.app.profile.switch_sheet.clone(), t.field)?
                .is_some()
            {
                choice = self.find(&[self.app.account_choice(handle)], t.field)?;
            }
        }
        let Some(choice) = choice else {
            if switcher.is_some() {
                self.back()?;
            }
            return Err(Error::NotOnPhone {
                handle: handle.clone(),
                serial: self.dev.serial().clone(),
                current: text,
            });
        };
        self.tap(&choice)?;
        let deadline = Instant::now() + App::secs(t.switch);
        loop {
            if let Some((_, now)) = self.profile_top(false)? {
                if Handle::new(&now).ok().as_ref() == Some(handle) {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::App(format!(
                    "TikTok didn't switch to @{handle}: switch to it in the app yourself"
                )));
            }
            thread::sleep(POLL);
        }
    }

    // --- the picker ---------------------------------------------------------------------------

    /// How many files the picker's "Next (n)" says are picked.
    fn picked_count(&mut self) -> Result<usize> {
        let Some(next) = self.find1(&self.app.picker.next.clone(), 0.0)? else {
            return Ok(0);
        };
        let text = self.ui.text(&next)?;
        Ok(text
            .split(['(', ')'])
            .nth(1)
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0))
    }

    fn picked_becomes(&mut self, n: usize) -> Result<bool> {
        let deadline = Instant::now() + App::secs(self.app.timeouts.field);
        while self.picked_count()? != n {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(POLL);
        }
        Ok(true)
    }

    /// The biggest list on the screen: the picker.
    fn biggest_grid(&mut self) -> Result<Option<(ElementId, Rect)>> {
        let mut best: Option<(ElementId, Rect)> = None;
        for s in self.app.picker.grids.clone() {
            for g in self.ui.find_all(&sel(&s), None)? {
                let r = self.ui.rect(&g)?;
                if best.as_ref().is_none_or(|(_, b)| r.area() > b.area()) {
                    best = Some((g, r));
                }
            }
        }
        Ok(best)
    }

    /// The picker's box and its cells, in reading order, once at least `count` are there or they
    /// fill the picker. The picker redraws while it loads, which loses the cells mid-read.
    fn cell_boxes(&mut self, count: usize) -> Result<(Rect, Vec<Rect>)> {
        let deadline = Instant::now() + App::secs(self.app.timeouts.gallery);
        let mut redraws = 0;
        loop {
            let read: Result<(Rect, Vec<Rect>)> = (|| {
                let Some((grid, rect)) = self.biggest_grid()? else {
                    return Ok((Rect::default(), vec![]));
                };
                let mut cells = Vec::new();
                for c in self
                    .ui
                    .find_all(&Selector::XPath("/*/*".into()), Some(&grid))?
                {
                    let r = self.ui.rect(&c)?;
                    cells.push((c, r));
                }
                Ok((rect, in_order(cells).into_iter().map(|(_, r)| r).collect()))
            })();
            match read {
                Ok((grid, boxes)) => {
                    if boxes.len() >= count || fills(grid, &boxes) || Instant::now() >= deadline {
                        return Ok((grid, boxes));
                    }
                }
                Err(_) if redraws < REDRAWS => redraws += 1,
                Err(e) => return Err(e),
            }
            thread::sleep(POLL);
        }
    }

    /// The picker's box, and the selection circles whole on the screen, in reading order, each
    /// with its text: its pick's number, or "".
    fn circles(&mut self) -> Result<Circles> {
        let deadline = Instant::now() + App::secs(self.app.timeouts.gallery);
        loop {
            let read: Result<Option<Circles>> = (|| {
                let Some((grid, rect)) = self.biggest_grid()? else {
                    return Ok(None);
                };
                let mut circles = Vec::new();
                for c in self
                    .ui
                    .find_all(&sel(&self.app.picker.pick_circle.clone()), Some(&grid))?
                {
                    let r = self.ui.rect(&c)?;
                    let t = self.ui.text(&c)?.trim().to_owned();
                    circles.push((c, r, t));
                }
                circles.sort_by_key(|(_, r, _)| (r.y, r.x));
                // a circle cut off by an edge is narrower one way than the other
                let whole = circles
                    .iter()
                    .map(|(_, r, _)| r.width.max(r.height))
                    .max()
                    .unwrap_or(0);
                Ok(Some((
                    rect,
                    circles
                        .into_iter()
                        .filter(|(_, r, _)| r.width >= whole - 2 && r.height >= whole - 2)
                        .map(|(_, r, t)| (r, t))
                        .collect(),
                )))
            })();
            if let Ok(Some(found)) = read {
                return Ok(found);
            }
            if Instant::now() >= deadline {
                return Ok((Rect::default(), vec![]));
            }
            thread::sleep(POLL);
        }
    }

    /// Select the first `count` cells in reading order, one at a time: each tap has to take
    /// ("Next (n)" counts it) before the next. The picker moves under the taps (a tray comes up
    /// over its bottom row, a cut-off cell scrolls into view), so the circles are read again when
    /// needed, and the next cell is the one after the circle that reads the last pick's number.
    fn select_cells(
        &mut self,
        count: usize,
        mut grid: Rect,
        boxes: &[Rect],
    ) -> Result<Option<String>> {
        if boxes.len() < count && !fills(grid, boxes) {
            return Ok(Some(format!(
                "TikTok's picker showed {} of the {count} files: {PICK_FIX}",
                boxes.len()
            )));
        }
        let (mut picked, mut scrolls) = (0usize, 0usize);
        let mut circles: Vec<(Rect, String)> = Vec::new();
        let mut after: Option<usize> = None;
        while picked < count {
            if picked < 2 || after.is_none_or(|a| a >= circles.len()) {
                (grid, circles) = self.circles()?;
                after = if picked == 0 {
                    (!circles.is_empty()).then_some(0)
                } else {
                    circles
                        .iter()
                        .position(|(_, t)| t == &picked.to_string())
                        .map(|i| i + 1)
                };
            }
            let Some(next) = after.filter(|a| *a < circles.len()) else {
                if scrolls >= count {
                    return Ok(Some(format!(
                        "TikTok's picker showed {picked} of the {count} files: {PICK_FIX}"
                    )));
                }
                // a slow drag up by half the picker: its grid ignores swipes, and a drag doesn't
                // fling on past the last pick
                let x = grid.x + grid.width / 2;
                let start = grid.y + grid.height * 3 / 4;
                self.ui
                    .drag((x, start), (x, start - grid.height / 2), 800)?;
                scrolls += 1;
                after = None;
                continue;
            };
            let (x, y) = circles[next].0.center();
            self.tap_at(x, y, true)?;
            if !self.picked_becomes(picked + 1)? {
                let n = self.picked_count()?;
                return Ok(Some(format!(
                    "TikTok says {n} files are picked, not {}: {PICK_FIX}",
                    picked + 1
                )));
            }
            picked += 1;
            let bottom_row = circles.iter().map(|(r, _)| r.y).max().unwrap_or(0);
            after = if circles[next].0.y >= bottom_row - 2 {
                None
            } else {
                Some(next + 1)
            };
        }
        Ok(None)
    }

    /// Open the picker on the right tab and pick the newest `count` files. None once picked and
    /// Next tapped, else what went wrong.
    fn pick(&mut self, video: bool, count: usize) -> Result<Option<String>> {
        let t = self.app.timeouts.clone();
        let p = self.app.picker.clone();
        for (what, s) in [("Create", &p.create), ("gallery", &p.gallery)] {
            let Some(b) = self.find1(s, t.launch)? else {
                return Ok(Some(format!(
                    "TikTok's {what} button wasn't there: {PICK_FIX}"
                )));
            };
            self.tap(&b)?;
        }
        let tab = if video { &p.videos_tab } else { &p.photos_tab };
        if let Some(tab) = self.find1(tab, t.field)? {
            self.tap(&tab)?;
        }
        let (grid, boxes) = self.cell_boxes(count)?;
        let already = self.picked_count()?;
        if already > 0 {
            if let Some(clear) = self.find1(&p.clear_picks, t.field)? {
                self.tap(&clear)?;
            }
            if self.picked_count()? > 0 {
                return Ok(Some(format!(
                    "TikTok's picker already had {already} files picked: unpick them, then retry"
                )));
            }
        }
        if let Some(problem) = self.select_cells(count, grid, &boxes)? {
            return Ok(Some(problem));
        }
        let Some(next) = self.find1(&p.next, t.field)? else {
            return Ok(Some(format!(
                "TikTok's Next button wasn't there: {PICK_FIX}"
            )));
        };
        self.tap(&next)?;
        Ok(None)
    }

    // --- the editor and the post screen -------------------------------------------------------

    fn hide_keyboard(&mut self) -> Result<()> {
        let shown = self
            .dev
            .shell("dumpsys input_method | grep -c 'mInputShown=true'")
            .unwrap_or_default();
        if shown.trim().parse::<u32>().unwrap_or(0) > 0 {
            self.ui.key(keycode::BACK)?;
        }
        Ok(())
    }

    /// Replace what a box holds with `text`, then put the keyboard away so it covers nothing.
    /// TikTok turns typed #hashtags into hashtags by itself.
    fn type_into(&mut self, el: &ElementId, text: &str) -> Result<()> {
        self.tap(el)?;
        self.ui.set_text(el, text)?;
        self.hide_keyboard()
    }

    fn pill(&mut self) -> Result<String> {
        match self.find1(&self.app.editor.sound_pill.clone(), self.app.timeouts.field)? {
            Some(p) => Ok(self.ui.text(&p)?.trim().to_owned()),
            None => Ok(String::new()),
        }
    }

    fn right_of(&mut self, anchor: &ElementId) -> Result<Option<ElementId>> {
        let a = self.ui.rect(anchor)?;
        let middle = a.y + a.height / 2;
        let mut best: Option<(ElementId, i32)> = None;
        for icon in self.ui.find_all(&sel(&self.app.editor.icon_button), None)? {
            let r = self.ui.rect(&icon)?;
            if r.x >= a.right()
                && r.y <= middle
                && middle <= r.bottom()
                && best.as_ref().is_none_or(|(_, x)| r.x > *x)
            {
                best = Some((icon, r.x));
            }
        }
        Ok(best.map(|(e, _)| e))
    }

    /// The first row of the sound search's results: a tall row of the biggest list that isn't
    /// the "Videos with related sounds" strip.
    fn first_result(&mut self) -> Result<Option<Rect>> {
        let deadline = Instant::now() + App::secs(self.app.timeouts.sound);
        loop {
            let mut biggest: Option<(ElementId, Rect)> = None;
            for l in self
                .ui
                .find_all(&sel(&self.app.editor.results_list), None)?
            {
                let r = self.ui.rect(&l)?;
                if biggest.as_ref().is_none_or(|(_, b)| r.area() > b.area()) {
                    biggest = Some((l, r));
                }
            }
            if let Some((list, _)) = biggest {
                for row in self
                    .ui
                    .find_all(&Selector::XPath("/*/*".into()), Some(&list))?
                {
                    let r = self.ui.rect(&row)?;
                    let nested = !self
                        .ui
                        .find_all(&sel(&self.app.editor.nested_list), Some(&row))?
                        .is_empty();
                    if r.height >= 100 && !nested {
                        return Ok(Some(r));
                    }
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(POLL);
        }
    }

    fn back_to_editor(&mut self) -> Result<()> {
        for _ in 0..3 {
            self.back()?;
            if self
                .find1(&self.app.editor.next.clone(), self.app.timeouts.field)?
                .is_some()
            {
                break;
            }
        }
        Ok(())
    }

    /// Search TikTok's sounds and use the first result, read back from the editor.
    fn sound_step(&mut self, report: &mut PostReport, sound: &str, fix: &str) -> Result<()> {
        let (t, e) = (self.app.timeouts.clone(), self.app.editor.clone());
        let Some(pill) = self.find1(&e.sound_pill, t.field)? else {
            report
                .problems
                .push(format!("the editor's sound button wasn't there: {fix}"));
            return Ok(());
        };
        self.tap(&pill)?;
        let icon = match self.find1(&e.sound_tab, t.sound)? {
            Some(tab) => self.right_of(&tab)?,
            None => None,
        };
        let Some(icon) = icon else {
            report
                .problems
                .push(format!("TikTok's sound search wasn't there: {fix}"));
            return self.back();
        };
        self.tap(&icon)?;
        let Some(search) = self.find1(&e.sound_search, t.sound)? else {
            report
                .problems
                .push(format!("TikTok's sound search wasn't there: {fix}"));
            return self.back_to_editor();
        };
        self.type_into(&search, sound)?;
        match self.find1(&e.sound_search_go, t.field)? {
            Some(go) => self.tap(&go)?,
            None => self.ui.key(keycode::ENTER)?,
        }
        let Some(row) = self.first_result()? else {
            report
                .problems
                .push(format!("no sound found for {sound:?}: {fix}"));
            return self.back_to_editor();
        };
        let (fx, fy) = e.use_spot;
        self.tap_at(
            row.x + (f64::from(row.width) * fx) as i32,
            row.y + (f64::from(row.height) * fy) as i32,
            false,
        )?;
        self.back_to_editor()?;
        let used = self.pill()?;
        if used.is_empty() {
            report
                .problems
                .push(format!("couldn't tell which sound TikTok used: {fix}"));
        } else {
            report.sound = Some(used);
        }
        Ok(())
    }

    fn visibility_step(&mut self, report: &mut PostReport, wanted: Visibility) -> Result<()> {
        let (t, ps) = (self.app.timeouts.clone(), self.app.post_screen.clone());
        let Some(row) = self.find1(&ps.visibility_row, t.field)? else {
            if wanted != Visibility::Everyone {
                report.problems.push(format!(
                    "TikTok's \"Who can view this post\" wasn't there: {VISIBILITY_FIX}"
                ));
            }
            return Ok(());
        };
        let shown = self.ui.text(&row)?;
        if self.app.shown_visibility(&shown) == Some(wanted) {
            report.visibility = Some(wanted);
            return Ok(());
        }
        self.tap(&row)?;
        let option = self.find(&[self.app.visibility_choice(wanted)], t.field)?;
        if let Some(o) = &option {
            self.tap(o)?;
        }
        if self.find1(&ps.visibility_sheet, 0.0)?.is_some() {
            self.back()?; // the sheet stays up after a choice
        }
        let name = self.app.visibility_name(wanted).to_owned();
        if option.is_none() {
            report
                .problems
                .push(format!("TikTok's list has no \"{name}\": {VISIBILITY_FIX}"));
            return Ok(());
        }
        // the row is redrawn as the sheet goes: look a few times until it reads the choice
        let deadline = Instant::now() + App::secs(t.field);
        let mut shown = String::new();
        for look in 1.. {
            shown = match self.find1(&ps.visibility_row, 0.0)? {
                Some(r) => self.ui.text(&r)?.trim().to_owned(),
                None => String::new(),
            };
            report.visibility = self.app.shown_visibility(&shown);
            if report.visibility == Some(wanted) || (look >= 3 && Instant::now() >= deadline) {
                break;
            }
            thread::sleep(POLL);
        }
        if report.visibility != Some(wanted) {
            let shown = if shown.is_empty() {
                "nothing".to_owned()
            } else {
                shown
            };
            report.problems.push(format!(
                "\"Who can view this post\" reads {shown}, not {name}: {VISIBILITY_FIX}"
            ));
        }
        Ok(())
    }

    /// Sound, Next, title, description and who can see it. A step that fails is a problem for a
    /// person to finish; once the phone stops answering, the rest is skipped.
    fn fill_editor(&mut self, report: &mut PostReport, post: &Post) -> Result<()> {
        let (t, e, ps) = (
            self.app.timeouts.clone(),
            self.app.editor.clone(),
            self.app.post_screen.clone(),
        );
        match self.find(&[sel(&e.sound_pill), sel(&e.next)], t.editor) {
            Ok(Some(_)) => {}
            _ => report
                .problems
                .push("TikTok's editor didn't open: check the phone".into()),
        }
        type Step<'s, 'a, U, D> =
            Box<dyn FnOnce(&mut TikTok<'a, U, D>, &mut PostReport) -> Result<()> + 's>;
        let mut steps: Vec<(&str, Step<'_, 'a, U, D>)> = Vec::new();
        if let Some(sound) = post.sound.clone() {
            steps.push((
                "add the sound",
                Box::new(move |me, r| me.sound_step(r, &sound, "add one yourself")),
            ));
        } else {
            steps.push((
                "read the sound",
                Box::new(|me, r| {
                    let picked = me.pill()?;
                    if !picked.is_empty() && !picked.eq_ignore_ascii_case("add sound") {
                        r.notes
                            .push(format!("TikTok added the sound \"{picked}\" by itself"));
                    }
                    Ok(())
                }),
            ));
        }
        let next_sel = e.next.clone();
        let post_sel = ps.post.clone();
        let editor_wait = t.editor;
        steps.push((
            "go on to the post screen",
            Box::new(move |me, r| {
                let Some(next) = me.find1(&next_sel, me.app.timeouts.field)? else {
                    r.problems
                        .push("the editor's Next button wasn't there: tap Next yourself".into());
                    return Ok(());
                };
                me.tap(&next)?;
                if me.find1(&post_sel, editor_wait)?.is_none() {
                    r.problems
                        .push("TikTok's post screen didn't open: check the phone".into());
                }
                Ok(())
            }),
        ));
        if let Some(title) = post.title.clone().filter(|t| !t.trim().is_empty()) {
            let boxes: Vec<Selector> = ps.title.iter().map(|s| sel(s)).collect();
            steps.push((
                "type the title",
                Box::new(move |me, r| match me.find(&boxes, me.app.timeouts.field)? {
                    Some(b) => me.type_into(&b, &title),
                    None => {
                        r.notes.push(
                            "no title box on this post screen: the title was left out".into(),
                        );
                        Ok(())
                    }
                }),
            ));
        }
        let caption = post.caption.trim().to_owned();
        if !caption.is_empty() {
            let boxes: Vec<Selector> = ps.caption.iter().map(|s| sel(s)).collect();
            steps.push((
                "type the description",
                Box::new(move |me, r| match me.find(&boxes, me.app.timeouts.field)? {
                    Some(b) => me.type_into(&b, &caption),
                    None => {
                        r.problems.push(
                            "the description box wasn't there: paste the description yourself"
                                .into(),
                        );
                        Ok(())
                    }
                }),
            ));
        }
        let wanted = post.visibility;
        steps.push((
            "choose who can see it",
            Box::new(move |me, r| me.visibility_step(r, wanted)),
        ));

        for (what, step) in steps {
            if let Err(err) = step(self, report) {
                if matches!(err, Error::Cancelled) {
                    return Err(err);
                }
                if !self.ui.is_alive() {
                    report
                        .problems
                        .push(format!("the phone stopped answering: {}", first_line(&err)));
                    return Ok(());
                }
                report
                    .problems
                    .push(format!("couldn't {what} ({})", first_line(&err)));
            }
        }
        Ok(())
    }

    /// Tap Post, only when nothing is left for a person to finish. Posted once the post screen
    /// goes away.
    fn post_step(&mut self, report: &mut PostReport) -> Result<()> {
        if !report.problems.is_empty() {
            report
                .notes
                .push("Post not tapped: finish the above on the phone, then tap it".into());
            return Ok(());
        }
        let (post, t) = (self.app.post_screen.post.clone(), self.app.timeouts.clone());
        let Some(button) = self.find1(&post, t.field)? else {
            report
                .problems
                .push("TikTok's Post button wasn't there: tap it yourself".into());
            return Ok(());
        };
        self.tap(&button)?;
        if self.gone(&post, t.posted)? {
            report.posted = true;
        } else {
            report
                .problems
                .push("tapped Post, but TikTok stayed on the post screen: check the phone".into());
        }
        Ok(())
    }

    /// Make the post, its files already in the phone's gallery (newest first): open TikTok on the
    /// account, pick them, fill everything in, and tap Post (unless it's a draft).
    pub fn publish(&mut self, post: &Post) -> Result<PostReport> {
        self.open_on(&post.account)?;
        self.publish_here(post)
    }

    /// Open TikTok on `handle` (see [`TikTok::switch_to`]).
    pub fn open_on(&mut self, handle: &Handle) -> Result<()> {
        self.launch()?;
        self.switch_to(handle)
    }

    /// [`TikTok::publish`] once TikTok is already on the post's account.
    pub fn publish_here(&mut self, post: &Post) -> Result<PostReport> {
        let video = matches!(post.media, Media::Video { .. });
        (self.say)("picking the files");
        let mut report = PostReport::default();
        if let Some(problem) = self.pick(video, post.media.count())? {
            report.problems.push(problem);
            report
                .problems
                .push("nothing was typed: pick the files and fill the post in yourself".into());
            return Ok(report);
        }
        (self.say)("filling the post in");
        self.fill_editor(&mut report, post)?;
        if post.draft {
            report.notes.push("draft: Post left for you to tap".into());
        } else {
            self.cancel.check()?;
            (self.say)("posting");
            match self.post_step(&mut report) {
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => report.problems.push(format!(
                    "couldn't tap Post ({}): check the phone",
                    first_line(&e)
                )),
                Ok(()) => {}
            }
        }
        Ok(report)
    }

    /// The screen's elements as XML, for fixing a selector.
    pub fn screen_xml(&mut self) -> Result<String> {
        self.ui.source()
    }
}
