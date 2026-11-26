use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use hive_adb::AdbDevice;
use hive_core::{Device, Direction, ElementId, Error, Rect, Result, Selector, Ui};
use serde_json::{Value, json};

use crate::apks::{SERVER_PACKAGE, TEST_PACKAGE};

const PORT_ON_PHONE: u16 = 6790;
const START_WAIT: Duration = Duration::from_secs(40);
const W3C_ID: &str = "element-6066-11e4-a52e-4f735466cecf";

/// A UiAutomator2 session on one phone. Dropping it stops the server on the phone.
pub struct Session {
    dev: AdbDevice,
    base: String,
    local_port: u16,
    instrumentation: Option<Child>,
    agent: ureq::Agent,
}

impl Session {
    /// Start the server on the phone (stopping any older one, Appium's included), forward a port
    /// to it and open a session. The server must be installed (see `ensure_installed`).
    pub fn start(dev: AdbDevice) -> Result<Self> {
        dev.shell(&format!(
            "am force-stop {TEST_PACKAGE}; am force-stop {SERVER_PACKAGE}"
        ))?;
        let instrumentation = dev.spawn_shell(&format!(
            "am instrument -w -e disableAnalytics true {TEST_PACKAGE}/androidx.test.runner.AndroidJUnitRunner"
        ))?;
        let local_port = dev.forward(PORT_ON_PHONE)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut session = Session {
            dev,
            base: format!("http://127.0.0.1:{local_port}"),
            local_port,
            instrumentation: Some(instrumentation),
            agent,
        };
        let deadline = Instant::now() + START_WAIT;
        loop {
            if session.get("/status").is_ok() {
                break;
            }
            if let Some(child) = session.instrumentation.as_mut() {
                if let Ok(Some(_)) = child.try_wait() {
                    return Err(Error::Ui(
                        "the UiAutomator2 server stopped at once on the phone: run `hive setup` again".into(),
                    ));
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::Ui(
                    "the UiAutomator2 server didn't answer on the phone".into(),
                ));
            }
            thread::sleep(Duration::from_millis(300));
        }
        let created = session.post(
            "/session",
            json!({"capabilities": {"firstMatch": [{}], "alwaysMatch": {}}}),
        )?;
        let id = created["sessionId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Ui(format!("no session id in {created}")))?;
        session.base = format!("{}/session/{id}", session.base);
        // Don't wait for the app to go still before each look (TikTok never does): every look
        // polls anyway.
        let _ = session.post(
            "/appium/settings",
            json!({"settings": {"waitForIdleTimeout": 100}}),
        );
        Ok(session)
    }

    pub fn device(&self) -> &AdbDevice {
        &self.dev
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn answer(&self, res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
        let mut res = res.map_err(|e| Error::Ui(format!("the phone's UI server: {e}")))?;
        let status = res.status();
        let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body
                .get("value")
                .cloned()
                .unwrap_or(body.clone())
                .pipe_session(&body))
        } else {
            let v = &body["value"];
            let msg = v["message"]
                .as_str()
                .or(v["error"].as_str())
                .unwrap_or("error");
            Err(Error::Ui(format!(
                "{}: {}",
                v["error"].as_str().unwrap_or("error"),
                first_line(msg)
            )))
        }
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.answer(self.agent.get(&self.url(path)).call())
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.answer(self.agent.post(&self.url(path)).send_json(body))
    }

    fn element(v: &Value) -> Option<ElementId> {
        v.get(W3C_ID)
            .or_else(|| v.get("ELEMENT"))
            .and_then(Value::as_str)
            .map(|s| ElementId(s.to_owned()))
    }
}

trait PipeSession {
    fn pipe_session(self, body: &Value) -> Value;
}

impl PipeSession for Value {
    /// New session answers put the id at the top; keep it reachable as `sessionId`.
    fn pipe_session(mut self, body: &Value) -> Value {
        if let (Some(id), Some(obj)) = (
            body.get("sessionId").filter(|v| v.is_string()),
            self.as_object_mut(),
        ) {
            obj.entry("sessionId").or_insert(id.clone());
        }
        self
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn strategy(selector: &Selector) -> (&'static str, &str) {
    match selector {
        Selector::UiAutomator(s) => ("-android uiautomator", s),
        Selector::XPath(s) => ("xpath", s),
    }
}

impl Ui for Session {
    fn find_all(
        &mut self,
        selector: &Selector,
        within: Option<&ElementId>,
    ) -> Result<Vec<ElementId>> {
        let (using, value) = strategy(selector);
        let mut body = json!({"strategy": using, "selector": value});
        if let Some(ctx) = within {
            body["context"] = json!(ctx.0);
        }
        match self.post("/elements", body) {
            Ok(Value::Array(found)) => Ok(found.iter().filter_map(Session::element).collect()),
            Ok(_) => Ok(Vec::new()),
            // a context element gone stale while the app redraws: nothing found there now
            Err(Error::Ui(m)) if m.contains("no such element") || m.contains("stale") => {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }

    fn text(&mut self, el: &ElementId) -> Result<String> {
        Ok(self
            .get(&format!("/element/{}/text", el.0))?
            .as_str()
            .unwrap_or("")
            .to_owned())
    }

    fn rect(&mut self, el: &ElementId) -> Result<Rect> {
        let v = self.get(&format!("/element/{}/rect", el.0))?;
        let n = |k: &str| v[k].as_f64().unwrap_or(0.0) as i32;
        Ok(Rect {
            x: n("x"),
            y: n("y"),
            width: n("width"),
            height: n("height"),
        })
    }

    fn click(&mut self, el: &ElementId) -> Result<()> {
        self.post(&format!("/element/{}/click", el.0), json!({}))
            .map(drop)
    }

    fn set_text(&mut self, el: &ElementId, text: &str) -> Result<()> {
        self.post(&format!("/element/{}/clear", el.0), json!({}))?;
        self.post(
            &format!("/element/{}/value", el.0),
            json!({"text": text, "replace": true}),
        )
        .map(drop)
    }

    fn tap(&mut self, x: i32, y: i32) -> Result<()> {
        self.post(
            "/appium/gestures/click",
            json!({"offset": {"x": x, "y": y}}),
        )
        .map(drop)
    }

    fn drag(&mut self, from: (i32, i32), to: (i32, i32), speed: u32) -> Result<()> {
        self.post(
            "/appium/gestures/drag",
            json!({"start": {"x": from.0, "y": from.1}, "end": {"x": to.0, "y": to.1}, "speed": speed}),
        )
        .map(drop)
    }

    fn swipe(&mut self, area: Rect, direction: Direction) -> Result<()> {
        let direction = match direction {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
        };
        self.post(
            "/appium/gestures/swipe",
            json!({"area": {"left": area.x, "top": area.y, "width": area.width, "height": area.height},
                   "direction": direction, "percent": 1.0}),
        )
        .map(drop)
    }

    fn key(&mut self, keycode: u32) -> Result<()> {
        self.post("/appium/device/press_keycode", json!({"keycode": keycode}))
            .map(drop)
    }

    fn screen_size(&mut self) -> Result<(i32, i32)> {
        let v = self.get("/window/current/size")?;
        Ok((
            v["width"].as_i64().unwrap_or(1080) as i32,
            v["height"].as_i64().unwrap_or(2400) as i32,
        ))
    }

    fn source(&mut self) -> Result<String> {
        Ok(self.get("/source")?.as_str().unwrap_or("").to_owned())
    }

    fn is_alive(&mut self) -> bool {
        self.find_all(&Selector::ui("new UiSelector().index(0)"), None)
            .is_ok()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.agent.delete(&self.base).call();
        if let Some(mut child) = self.instrumentation.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.dev.shell(&format!(
            "am force-stop {TEST_PACKAGE}; am force-stop {SERVER_PACKAGE}"
        ));
        self.dev.unforward(self.local_port);
    }
}
