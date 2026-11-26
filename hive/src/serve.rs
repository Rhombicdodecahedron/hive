//! `hive serve`: the queue runner, a local JSON API (what Clipdeck and other tools call) and a
//! small dashboard, on 127.0.0.1.
//!
//! API (JSON):
//!   GET    /api/phones                     phones, their status and what hive knows of them
//!   POST   /api/phones/{serial}/check      open TikTok, read its account          → job
//!   POST   /api/phones/{serial}/setup      install the UI server                  → job
//!   POST   /api/phones/{serial}/wake|sleep
//!   GET    /api/phones/{serial}/screen.png
//!   POST   /api/posts                      post now: {account, files, caption…}  → job
//!   GET    /api/jobs, /api/jobs/{id}
//!   GET    /api/queue
//!   POST   /api/queue                      plan a post: {…, at}                  → entry
//!   DELETE /api/queue/{id}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow};
use hive_core::{Error, Handle, Media, Post, Serial, Visibility};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::engine::{Hive, PhoneView};
use crate::queue::{Entry, Queue, Status, local, parse_time};

const DASHBOARD: &str = include_str!("dashboard.html");
const KEEP_JOBS: usize = 40;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
struct Job {
    id: u64,
    kind: &'static str,
    label: String,
    started: Timestamp,
    state: JobState,
    lines: Vec<String>,
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_id: Option<String>,
}

struct State {
    hive: Hive,
    jobs: Mutex<Vec<Arc<Mutex<Job>>>>,
    next_id: Mutex<u64>,
    phones: RwLock<Vec<PhoneView>>,
    wake_phones: Mutex<Option<Instant>>,
    retry_at: Mutex<HashMap<String, Instant>>,
}

type Shared = Arc<State>;

/// The fields a post is given as over the API.
#[derive(Debug, Deserialize)]
struct PostRequest {
    account: String,
    files: Vec<PathBuf>,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    sound: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    at: Option<String>,
}

impl PostRequest {
    fn post(&self) -> Result<Post> {
        Ok(Post {
            account: Handle::new(&self.account)?,
            media: Media::from_files(self.files.clone())?,
            caption: self.caption.clone(),
            title: self.title.clone(),
            sound: self.sound.clone().filter(|s| !s.trim().is_empty()),
            visibility: match &self.visibility {
                Some(v) => v.parse()?,
                None => Visibility::Everyone,
            },
            draft: self.draft,
        })
    }
}

fn now_ts() -> Timestamp {
    Timestamp::now()
}

impl State {
    fn start_job(
        self: &Arc<Self>,
        kind: &'static str,
        label: String,
        queue_id: Option<String>,
        work: impl FnOnce(&State, &mut dyn FnMut(&str)) -> Result<Value> + Send + 'static,
    ) -> u64 {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        let job = Arc::new(Mutex::new(Job {
            id,
            kind,
            label,
            started: now_ts(),
            state: JobState::Running,
            lines: vec![],
            result: None,
            queue_id,
        }));
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.insert(0, job.clone());
            jobs.truncate(KEEP_JOBS);
        }
        let me = self.clone();
        thread::spawn(move || {
            let log = job.clone();
            let mut say = move |line: &str| {
                let stamp = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
                log.lock().unwrap().lines.push(format!("{stamp}  {line}"));
            };
            let outcome = work(&me, &mut say);
            let mut j = job.lock().unwrap();
            match outcome {
                Ok(v) => {
                    j.state = JobState::Done;
                    j.result = Some(v);
                }
                Err(e) => {
                    j.lines.push(format!("error: {e:#}"));
                    j.state = JobState::Failed;
                    j.result = Some(json!({"error": format!("{e:#}")}));
                }
            }
            drop(j);
            me.refresh_phones_soon();
        });
        id
    }

    fn refresh_phones_soon(&self) {
        *self.wake_phones.lock().unwrap() = Some(Instant::now());
    }

    fn running_jobs(&self) -> usize {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.lock().unwrap().state == JobState::Running)
            .count()
    }

    fn queue_running(&self, id: &str) -> bool {
        self.jobs.lock().unwrap().iter().any(|j| {
            let j = j.lock().unwrap();
            j.state == JobState::Running && j.queue_id.as_deref() == Some(id)
        })
    }
}

/// Start the queue entry's post as a job; the job writes the outcome back to the queue.
fn run_entry(state: &Shared, entry: Entry) {
    let id = entry.id.clone();
    let label = format!(
        "{} → @{} (planned {})",
        entry.post.media.files()[0].display(),
        entry.post.account,
        local(entry.at)
    );
    let _ = Queue::new(&state.hive.home).update(|es| {
        if let Some(e) = es.iter_mut().find(|e| e.id == id) {
            e.status = Status::Posting;
        }
        Ok(())
    });
    let qid = id.clone();
    state.start_job("post", label, Some(id), move |st, say| {
        let outcome = st.hive.post(&entry.post, None, say);
        let queue = Queue::new(&st.hive.home);
        let mut retry = None;
        let result = queue.update(|es| {
            let Some(e) = es.iter_mut().find(|e| e.id == qid) else {
                return Ok(json!({}));
            };
            match &outcome {
                Ok(p) if p.report.posted => {
                    e.status = Status::Posted;
                    e.phone = Some(p.phone.to_string());
                    e.done_at = Some(now_ts());
                    e.note.clear();
                }
                Ok(p) => {
                    e.status = Status::OnPhone;
                    e.phone = Some(p.phone.to_string());
                    e.note = p.report.problems.join("; ");
                }
                Err(err) if err.is_waiting() => {
                    e.status = Status::Pending;
                    e.note = format!("waiting for a phone: {err}");
                    retry = Some(Duration::from_secs(60));
                }
                Err(err) => {
                    e.tries += 1;
                    e.note = err.to_string();
                    e.status = if e.tries >= 2 {
                        Status::Failed
                    } else {
                        Status::Pending
                    };
                    retry = Some(Duration::from_secs(120));
                }
            }
            Ok(serde_json::to_value(&*e)?)
        })?;
        if let Some(after) = retry {
            st.retry_at
                .lock()
                .unwrap()
                .insert(qid.clone(), Instant::now() + after);
        }
        match outcome {
            Ok(p) => {
                for n in &p.report.notes {
                    say(n);
                }
                for pb in &p.report.problems {
                    say(&format!("to finish on the phone: {pb}"));
                }
                say(if p.report.posted {
                    "posted"
                } else {
                    "left on the phone to finish"
                });
                Ok(result)
            }
            Err(e) if e.is_waiting() => {
                say(&format!("waiting: {e}"));
                Ok(result)
            }
            Err(e) => Err(anyhow!(e)),
        }
    });
}

/// Every few seconds: start the due posts, skip the ones too late.
fn scheduler(state: Shared) {
    let _ = Queue::new(&state.hive.home).update(|es| {
        for e in es.iter_mut().filter(|e| e.status == Status::Posting) {
            e.status = Status::Pending; // hive stopped mid-post: try again
        }
        Ok(())
    });
    loop {
        let late = state
            .hive
            .config()
            .map(|c| c.late_limit_hours)
            .unwrap_or(6.0);
        let now = now_ts();
        let queue = Queue::new(&state.hive.home);
        let due: Vec<Entry> = queue
            .update(|es| {
                let mut due = Vec::new();
                for e in es
                    .iter_mut()
                    .filter(|e| e.status == Status::Pending && e.at <= now)
                {
                    let behind = now.duration_since(e.at).as_secs_f64() / 3600.0;
                    if behind > late {
                        e.status = Status::Skipped;
                        e.note = format!(
                            "more than {late} h late{}",
                            if e.note.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", e.note)
                            }
                        );
                    } else {
                        due.push(e.clone());
                    }
                }
                Ok(due)
            })
            .unwrap_or_default();
        for e in due {
            let waiting = state
                .retry_at
                .lock()
                .unwrap()
                .get(&e.id)
                .is_some_and(|t| *t > Instant::now());
            if !waiting && !state.queue_running(&e.id) {
                run_entry(&state, e);
            }
        }
        thread::sleep(Duration::from_secs(5));
    }
}

/// Read the phones every few seconds (sooner after a job), so the API never waits on adb.
fn watcher(state: Shared) {
    loop {
        if let Ok(phones) = state.hive.phones() {
            *state.phones.write().unwrap() = phones;
        }
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if state.wake_phones.lock().unwrap().take().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
}

/// With --reload: start again when the hive binary changes (after `cargo install`), once
/// nothing is running.
#[cfg(unix)]
fn reloader(state: Shared) {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let stamp = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let first: Option<SystemTime> = stamp(&exe);
    loop {
        thread::sleep(Duration::from_secs(2));
        if stamp(&exe) != first && state.running_jobs() == 0 {
            thread::sleep(Duration::from_secs(1)); // let the new binary finish landing
            eprintln!("hive changed: restarting");
            let err = std::process::Command::new(&exe)
                .args(std::env::args_os().skip(1))
                .exec();
            eprintln!("couldn't restart: {err}");
            return;
        }
    }
}

fn json_response(status: u16, body: &impl Serialize) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    Response::from_data(bytes)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}

fn error(status: u16, message: impl std::fmt::Display) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, &json!({"error": message.to_string()}))
}

fn serial(raw: &str) -> Result<Serial> {
    Ok(Serial::new(raw)?)
}

fn handle(state: &Shared, mut req: Request) {
    let method = req.method().clone();
    let url = req.url().split('?').next().unwrap_or("/").to_owned();
    let parts: Vec<&str> = url.trim_matches('/').split('/').collect();
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);

    if method == Method::Get && (url == "/" || url == "/index.html") {
        let r = Response::from_string(DASHBOARD)
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
        let _ = req.respond(r);
        return;
    }
    if method == Method::Get
        && parts.len() == 4
        && parts[..2] == ["api", "phones"]
        && parts[3] == "screen.png"
    {
        let png = serial(parts[2]).and_then(|s| Ok(state.hive.device(&s).screencap()?));
        let _ = match png {
            Ok(data) => req.respond(
                Response::from_data(data)
                    .with_header(Header::from_bytes("Content-Type", "image/png").unwrap())
                    .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
            ),
            Err(e) => req.respond(error(404, e)),
        };
        return;
    }

    let res = route(state, &method, &parts, &body);
    let _ = req.respond(match res {
        Ok((status, v)) => json_response(status, &v),
        Err(e) => {
            let code = if e
                .downcast_ref::<Error>()
                .is_some_and(|e| matches!(e, Error::Invalid(_)))
            {
                400
            } else {
                500
            };
            error(code, format!("{e:#}"))
        }
    });
}

fn route(state: &Shared, method: &Method, parts: &[&str], body: &str) -> Result<(u16, Value)> {
    let parse = || -> Result<PostRequest> {
        serde_json::from_str(body).context("the body isn't a post (JSON)")
    };
    Ok(match (method, parts) {
        (Method::Get, ["api", "phones"]) => {
            (200, serde_json::to_value(&*state.phones.read().unwrap())?)
        }
        (Method::Post, ["api", "phones", s, action]) => {
            let s = serial(s)?;
            match *action {
                "wake" => {
                    state.hive.wake(&s)?;
                    state.refresh_phones_soon();
                    (200, json!({"ok": true}))
                }
                "sleep" => {
                    state.hive.sleep(&s)?;
                    state.refresh_phones_soon();
                    (200, json!({"ok": true}))
                }
                "check" => {
                    let id =
                        state.start_job("check", format!("check {s}"), None, move |st, say| {
                            let h = st.hive.check(&s, say)?;
                            say(&format!("TikTok is on @{h}"));
                            Ok(json!({"account": h}))
                        });
                    (202, json!({"job": id}))
                }
                "setup" => {
                    let id =
                        state.start_job("setup", format!("setup {s}"), None, move |st, say| {
                            let installed = st.hive.setup(&s)?;
                            say(if installed {
                                "UI server installed"
                            } else {
                                "UI server already there"
                            });
                            Ok(json!({"installed": installed}))
                        });
                    (202, json!({"job": id}))
                }
                _ => (404, json!({"error": "unknown phone action"})),
            }
        }
        (Method::Post, ["api", "posts"]) => {
            let req = parse()?;
            let post = req.post()?;
            let only = req.phone.as_deref().map(Serial::new).transpose()?;
            let label = format!("{} → @{}", post.media.files()[0].display(), post.account);
            let id = state.start_job("post", label, None, move |st, say| {
                let done = st.hive.post(&post, only.as_ref(), say)?;
                say(if done.report.posted {
                    "posted"
                } else {
                    "left on the phone to finish"
                });
                Ok(serde_json::to_value(done)?)
            });
            (202, json!({"job": id}))
        }
        (Method::Get, ["api", "jobs"]) => {
            let jobs: Vec<Job> = state
                .jobs
                .lock()
                .unwrap()
                .iter()
                .map(|j| j.lock().unwrap().clone())
                .collect();
            (200, serde_json::to_value(jobs)?)
        }
        (Method::Get, ["api", "jobs", id]) => {
            let id: u64 = id.parse().unwrap_or(0);
            let found = state
                .jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| j.lock().unwrap().id == id)
                .cloned();
            match found {
                Some(j) => (200, serde_json::to_value(&*j.lock().unwrap())?),
                None => (404, json!({"error": "no such job"})),
            }
        }
        (Method::Get, ["api", "queue"]) => (
            200,
            serde_json::to_value(Queue::new(&state.hive.home).entries()?)?,
        ),
        (Method::Post, ["api", "queue"]) => {
            let req = parse()?;
            let at = parse_time(req.at.as_deref().context("a planned post needs \"at\"")?)?;
            (
                201,
                serde_json::to_value(Queue::new(&state.hive.home).add(req.post()?, at)?)?,
            )
        }
        (Method::Delete, ["api", "queue", id]) => (
            200,
            serde_json::to_value(Queue::new(&state.hive.home).remove(id)?)?,
        ),
        _ => (404, json!({"error": "not found"})),
    })
}

pub fn serve(hive: Hive, address: Option<String>, reload: bool) -> Result<()> {
    let address = address.unwrap_or(hive.config()?.address);
    let server = Server::http(&address).map_err(|e| anyhow!("can't listen on {address}: {e}"))?;
    let state: Shared = Arc::new(State {
        phones: RwLock::new(hive.phones().unwrap_or_default()),
        hive,
        jobs: Mutex::new(vec![]),
        next_id: Mutex::new(0),
        wake_phones: Mutex::new(None),
        retry_at: Mutex::new(HashMap::new()),
    });
    {
        let s = state.clone();
        thread::spawn(move || watcher(s));
        let s = state.clone();
        thread::spawn(move || scheduler(s));
        #[cfg(unix)]
        if reload {
            let s = state.clone();
            thread::spawn(move || reloader(s));
        }
        #[cfg(not(unix))]
        let _ = reload;
    }
    println!("hive is serving on http://{address}  (Ctrl+C to stop)");
    for req in server.incoming_requests() {
        let s = state.clone();
        thread::spawn(move || handle(&s, req));
    }
    Ok(())
}
