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
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow};
use hive_core::{Cancel, Error, Handle, Media, Post, Serial, Visibility};
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
    Cancelled,
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
    /// Asked to stop: it stops at its next step, unless Post was already tapped.
    cancelled: bool,
    #[serde(skip)]
    cancel: Cancel,
}

struct State {
    hive: Hive,
    jobs: Mutex<Vec<Arc<Mutex<Job>>>>,
    next_id: Mutex<u64>,
    phones: RwLock<Vec<PhoneView>>,
    wake_phones: Mutex<Option<Instant>>,
    retry_at: Mutex<HashMap<String, Instant>>,
    /// Goes up on every change the dashboard shows; `/api/state?since=` waits on it.
    rev: Mutex<u64>,
    changed: Condvar,
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
        work: impl FnOnce(&State, &Cancel, &mut dyn FnMut(&str)) -> Result<Value> + Send + 'static,
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
            cancelled: false,
            cancel: Cancel::new(),
        }));
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.insert(0, job.clone());
            jobs.truncate(KEEP_JOBS);
        }
        self.bump();
        let me = self.clone();
        thread::spawn(move || {
            let log = job.clone();
            let notify = me.clone();
            let mut say = move |line: &str| {
                let stamp = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
                log.lock().unwrap().lines.push(format!("{stamp}  {line}"));
                notify.bump();
            };
            let cancel = job.lock().unwrap().cancel.clone();
            let outcome = work(&me, &cancel, &mut say);
            let mut j = job.lock().unwrap();
            match outcome {
                Err(e)
                    if e.downcast_ref::<Error>()
                        .is_some_and(|e| matches!(e, Error::Cancelled)) =>
                {
                    j.lines.push("cancelled: nothing was posted".into());
                    j.state = JobState::Cancelled;
                    j.result = Some(json!({"cancelled": true}));
                }
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
            me.bump();
            me.refresh_phones_soon();
        });
        id
    }

    fn bump(&self) {
        *self.rev.lock().unwrap() += 1;
        self.changed.notify_all();
    }

    /// Everything the dashboard shows, in one answer.
    fn snapshot(&self) -> Result<Value> {
        let phones = self.phones.read().unwrap().clone();
        let jobs: Vec<Value> = self
            .jobs
            .lock()
            .unwrap()
            .iter()
            .take(12)
            .map(|j| {
                let mut j = j.lock().unwrap().clone();
                let keep = j.lines.len().saturating_sub(80);
                j.lines.drain(..keep);
                serde_json::to_value(j).unwrap_or_default()
            })
            .collect();
        let day_ago = now_ts() - jiff::SignedDuration::from_hours(24);
        let queue: Vec<Entry> = Queue::new(&self.hive.home)
            .entries()?
            .into_iter()
            .filter(|e| match e.status {
                Status::Posted | Status::Skipped => e.done_at.unwrap_or(e.at) > day_ago,
                _ => true,
            })
            .collect();
        let mut accounts: Vec<String> = self
            .hive
            .config()?
            .accounts
            .keys()
            .map(|h| h.to_string())
            .collect();
        for p in &phones {
            accounts.extend(p.notes.accounts.keys().map(|h| h.to_string()));
        }
        accounts.sort();
        accounts.dedup();
        Ok(json!({
            "rev": *self.rev.lock().unwrap(),
            "phones": phones,
            "jobs": jobs,
            "queue": queue,
            "accounts": accounts,
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }

    /// Wait until something changed after `since` (or `wait` ran out).
    fn wait_change(&self, since: u64, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut rev = self.rev.lock().unwrap();
        while *rev <= since {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            rev = self.changed.wait_timeout(rev, left).unwrap().0;
        }
    }

    /// Stop a running job at its next step. False when there is no such running job.
    fn cancel_job(&self, id: u64) -> bool {
        let jobs = self.jobs.lock().unwrap();
        let Some(job) = jobs.iter().find(|j| j.lock().unwrap().id == id) else {
            return false;
        };
        let mut j = job.lock().unwrap();
        if j.state != JobState::Running {
            return false;
        }
        j.cancel.cancel();
        j.cancelled = true;
        j.lines.push("cancelling at the next step…".into());
        drop(j);
        drop(jobs);
        self.bump();
        true
    }

    fn job_for_queue(&self, queue_id: &str) -> Option<u64> {
        self.jobs.lock().unwrap().iter().find_map(|j| {
            let j = j.lock().unwrap();
            (j.state == JobState::Running && j.queue_id.as_deref() == Some(queue_id))
                .then_some(j.id)
        })
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
    state.start_job("post", label, Some(id), move |st, cancel, say| {
        let outcome = st.hive.post(&entry.post, None, cancel, say);
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
                Err(Error::Cancelled) => {
                    e.status = Status::Skipped;
                    e.note = "cancelled while posting: nothing was posted".into();
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
    let queue_file = state.hive.home.queue_file();
    let stamp = || {
        std::fs::metadata(&queue_file)
            .and_then(|m| m.modified())
            .ok()
    };
    let mut queue_seen = stamp();
    loop {
        if let Ok(phones) = state.hive.phones() {
            let changed = serde_json::to_string(&phones).ok()
                != serde_json::to_string(&*state.phones.read().unwrap()).ok();
            if changed {
                *state.phones.write().unwrap() = phones;
                state.bump();
            }
        }
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if stamp() != queue_seen {
                queue_seen = stamp();
                state.bump();
            }
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

fn query<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Files dropped on the dashboard, kept a week.
fn uploads_dir(home: &crate::home::Home) -> PathBuf {
    home.data_dir.join("uploads")
}

fn clean_uploads(home: &crate::home::Home) {
    let week = Duration::from_secs(7 * 24 * 3600);
    if let Ok(dir) = std::fs::read_dir(uploads_dir(home)) {
        for f in dir.flatten() {
            let old = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if old.is_some_and(|age| age > week) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

fn save_upload(state: &Shared, req: &mut Request, name: &str) -> Result<Value> {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ext_ok = Media::from_files(vec![PathBuf::from(&clean)]).is_ok();
    if clean.is_empty() || !ext_ok {
        return Err(anyhow!(Error::Invalid(format!(
            "not a video or a picture: {name}"
        ))));
    }
    let dir = uploads_dir(&state.hive.home);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "{}-{clean}",
        jiff::Zoned::now().strftime("%Y%m%d-%H%M%S%.3f")
    ));
    let mut file = std::fs::File::create(&path)?;
    std::io::copy(req.as_reader(), &mut file)?;
    Ok(json!({"path": path, "name": name}))
}

fn handle(state: &Shared, mut req: Request) {
    let method = req.method().clone();
    let full = req.url().to_owned();
    let url = full.split('?').next().unwrap_or("/").to_owned();
    let parts: Vec<&str> = url.trim_matches('/').split('/').collect();

    if method == Method::Get && (url == "/" || url == "/index.html") {
        let r = Response::from_string(DASHBOARD)
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
            .with_header(Header::from_bytes("Cache-Control", "no-cache").unwrap());
        let _ = req.respond(r);
        return;
    }
    if method == Method::Get && url == "/api/state" {
        let since: u64 = query(&full, "since")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let wait = query(&full, "wait")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0u64)
            .min(30);
        if since > 0 {
            state.wait_change(since, Duration::from_secs(wait));
        }
        let _ = req.respond(match state.snapshot() {
            Ok(v) => json_response(200, &v),
            Err(e) => error(500, format!("{e:#}")),
        });
        return;
    }
    if method == Method::Post && url == "/api/uploads" {
        let name = decode(query(&full, "name").unwrap_or(""));
        let res = save_upload(state, &mut req, &name);
        let _ = req.respond(match res {
            Ok(v) => json_response(201, &v),
            Err(e) => error(400, format!("{e:#}")),
        });
        return;
    }
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
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
                        state.start_job("check", format!("check {s}"), None, move |st, _, say| {
                            let h = st.hive.check(&s, say)?;
                            say(&format!("TikTok is on @{h}"));
                            Ok(json!({"account": h}))
                        });
                    (202, json!({"job": id}))
                }
                "setup" => {
                    let id =
                        state.start_job("setup", format!("setup {s}"), None, move |st, _, say| {
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
            let id = state.start_job("post", label, None, move |st, cancel, say| {
                let done = st.hive.post(&post, only.as_ref(), cancel, say)?;
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
        (Method::Delete, ["api", "queue", id]) => match state.job_for_queue(id) {
            // being posted right now: stop it; the job marks the entry skipped
            Some(job) => {
                state.cancel_job(job);
                (202, json!({"cancelling": job}))
            }
            None => (
                200,
                serde_json::to_value(Queue::new(&state.hive.home).remove(id)?)?,
            ),
        },
        (Method::Post, ["api", "jobs", id, "cancel"]) => {
            let id: u64 = id.parse().unwrap_or(0);
            if state.cancel_job(id) {
                (202, json!({"cancelling": id}))
            } else {
                (409, json!({"error": "that job isn't running"}))
            }
        }
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
        rev: Mutex::new(1),
        changed: Condvar::new(),
    });
    clean_uploads(&state.hive.home);
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
