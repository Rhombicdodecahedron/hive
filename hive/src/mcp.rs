//! `hive mcp`: Hive as an MCP server (stdio), so Claude and other MCP clients can see the
//! phones, post, plan posts and follow them.
//!
//! Posting takes a minute or two, so the tools go through `hive serve` (started here when it
//! isn't running): a post becomes a job, and `hive_job` follows it.
//!
//! Claude Code:     claude mcp add hive -- hive mcp
//! Claude Desktop:  {"mcpServers": {"hive": {"command": "hive", "args": ["mcp"]}}}

use std::fs;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::home::Home;

const PROTOCOL: &str = "2025-06-18";

#[cfg(not(feature = "studio"))]
const INSTRUCTIONS: &str = "Hive posts to TikTok from Android phones plugged into this computer. Files are paths on this computer. A post takes 1 to 2 minutes: start it with hive_post, then follow it with hive_job. Never post without the user asking for it.";
#[cfg(feature = "studio")]
const INSTRUCTIONS: &str = "Hive posts to TikTok from Android phones plugged into this computer, and makes vertical clips from long videos. Files are paths on this computer. A post takes 1 to 2 minutes: start it with hive_post, then follow it with hive_job. Never post without the user asking for it. To make clips yourself: read hive_transcript, pick moments where something visibly happens and pays off (15 to 40 s), look at them with hive_frames (leave out dark scenes, menus, maps), then cut each with hive_make_clip.";

struct Api {
    base: String,
    agent: ureq::Agent,
}

impl Api {
    fn new(address: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: format!("http://{address}"),
            agent,
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let res = match (method, body) {
            ("GET", _) => self.agent.get(&url).call(),
            ("DELETE", _) => self.agent.delete(&url).call(),
            (_, Some(b)) => self.agent.post(&url).send_json(b),
            (_, None) => self.agent.post(&url).send_empty(),
        };
        let mut res =
            res.map_err(|e| anyhow!("hive serve isn't answering at {}: {e}", self.base))?;
        let status = res.status();
        let v: Value = res.body_mut().read_json().unwrap_or(Value::Null);
        if status.is_success() {
            Ok(v)
        } else {
            Err(anyhow!(
                "{}",
                v["error"].as_str().unwrap_or("hive serve refused it")
            ))
        }
    }

    /// Make sure `hive serve` answers, starting it in the background when it doesn't.
    fn ensure(&self, home: &Home) -> Result<()> {
        if self.call("GET", "/api/phones", None).is_ok() {
            return Ok(());
        }
        fs::create_dir_all(&home.data_dir)?;
        let log = fs::File::create(home.data_dir.join("serve.log"))?;
        let mut cmd = Command::new(std::env::current_exe()?);
        cmd.arg("serve")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // keeps running after the MCP client closes
        }
        cmd.spawn().context("starting hive serve")?;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(250));
            if self.call("GET", "/api/phones", None).is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!(
            "started hive serve but it isn't answering; see {}",
            home.data_dir.join("serve.log").display()
        ))
    }
}

fn post_schema(planned: bool) -> Value {
    let mut props = json!({
        "account": {"type": "string", "description": "The TikTok account to post on, like @yourpage"},
        "files": {"type": "array", "items": {"type": "string"},
                  "description": "Absolute paths on this computer: one video, or pictures for a slideshow in order"},
        "caption": {"type": "string", "description": "The description, #hashtags included"},
        "title": {"type": "string", "description": "A slideshow's title"},
        "sound": {"type": "string", "description": "Search TikTok's sounds for this and use the first result"},
        "visibility": {"type": "string", "enum": ["everyone", "friends", "only_you"]},
        "draft": {"type": "boolean", "description": "Fill everything in but leave Post for a person to tap"},
        "phone": {"type": "string", "description": "Only this phone (adb serial)"}
    });
    let mut required = vec!["account", "files"];
    if planned {
        props["at"] = json!({"type": "string", "description": "When: \"2026-11-27 18:00\" (the computer's time zone) or RFC 3339"});
        required.push("at");
    }
    json!({"type": "object", "properties": props, "required": required})
}

fn tools() -> Value {
    #[allow(unused_mut)]
    let mut tools = json!([
        {"name": "hive_phones", "description": "The Android phones plugged in: whether each can post right now (and why not), battery, lock, the TikTok accounts seen on it.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "hive_post", "description": "Post a video or a slideshow on a TikTok account now, with whichever phone holds the account. Returns a job; it takes 1 to 2 minutes, follow it with hive_job.",
         "inputSchema": post_schema(false)},
        {"name": "hive_plan", "description": "Plan a post for a given time; hive serve posts it then.",
         "inputSchema": post_schema(true)},
        {"name": "hive_queue", "description": "The planned posts and what happened to them.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "hive_cancel", "description": "Stop a post: a planned one (`id`, removed from the plan, or stopped if it's being posted) or a running job (`job`). A post stops at its next step and is never cancelled once Post was tapped.",
         "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "description": "A planned post's id"}, "job": {"type": "integer", "description": "A running job's id"}}}},
        {"name": "hive_job", "description": "What a post (or a check) did so far, line by line, and its result once done. Waits up to `wait_seconds` (default 25) for it to finish.",
         "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "wait_seconds": {"type": "integer"}}, "required": ["id"]}},
        {"name": "hive_check", "description": "Wake a phone, open TikTok and read which account it is on. Returns a job.",
         "inputSchema": {"type": "object", "properties": {"phone": {"type": "string", "description": "adb serial"}}, "required": ["phone"]}}
    ]);
    #[cfg(feature = "studio")]
    if let (Some(list), Value::Array(more)) = (tools.as_array_mut(), crate::clips::mcp_tools()) {
        list.extend(more);
    }
    tools
}

fn call_tool(api: &Api, name: &str, args: &Value) -> Result<Value> {
    Ok(match name {
        "hive_phones" => api.call("GET", "/api/phones", None)?,
        "hive_post" => api.call("POST", "/api/posts", Some(args.clone()))?,
        "hive_plan" => api.call("POST", "/api/queue", Some(args.clone()))?,
        "hive_queue" => api.call("GET", "/api/queue", None)?,
        "hive_cancel" => match (args["id"].as_str(), args["job"].as_u64()) {
            (_, Some(job)) => {
                api.call("POST", &format!("/api/jobs/{job}/cancel"), Some(json!({})))?
            }
            (Some(id), None) => api.call("DELETE", &format!("/api/queue/{id}"), None)?,
            _ => return Err(anyhow!("give the planned post's `id` or the running `job`")),
        },
        "hive_check" => {
            let phone = args["phone"].as_str().context("phone is required")?;
            api.call(
                "POST",
                &format!("/api/phones/{phone}/check"),
                Some(json!({})),
            )?
        }
        "hive_job" => {
            let id = args["id"].as_u64().context("id is required")?;
            let wait = Duration::from_secs(args["wait_seconds"].as_u64().unwrap_or(25).min(55));
            let deadline = Instant::now() + wait;
            loop {
                let job = api.call("GET", &format!("/api/jobs/{id}"), None)?;
                if job["state"] != "running" || Instant::now() >= deadline {
                    break job;
                }
                thread::sleep(Duration::from_millis(700));
            }
        }
        other => return Err(anyhow!("no tool named {other}")),
    })
}

fn reply(out: &mut impl Write, id: &Value, result: Value) -> Result<()> {
    writeln!(
        out,
        "{}",
        json!({"jsonrpc": "2.0", "id": id, "result": result})
    )?;
    out.flush()?;
    Ok(())
}

fn reply_error(out: &mut impl Write, id: &Value, code: i64, message: &str) -> Result<()> {
    writeln!(
        out,
        "{}",
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
    )?;
    out.flush()?;
    Ok(())
}

pub fn run() -> Result<()> {
    let home = Home::locate()?;
    let api = Api::new(&home.config()?.address);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            reply_error(&mut out, &Value::Null, -32700, "not JSON")?;
            continue;
        };
        let id = msg.get("id").cloned();
        let method = msg["method"].as_str().unwrap_or("");
        let Some(id) = id else { continue }; // a notification: nothing to answer
        match method {
            "initialize" => {
                let version = msg["params"]["protocolVersion"]
                    .as_str()
                    .unwrap_or(PROTOCOL)
                    .to_owned();
                reply(
                    &mut out,
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "hive", "version": env!("CARGO_PKG_VERSION")},
                        "instructions": INSTRUCTIONS
                    }),
                )?;
            }
            "ping" => reply(&mut out, &id, json!({}))?,
            "tools/list" => reply(&mut out, &id, json!({"tools": tools()}))?,
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
                // the studio's tools work on files here, without hive serve
                #[cfg(feature = "studio")]
                if let Some(result) = crate::clips::mcp_call(&home, name, &args) {
                    let body = match result {
                        Ok(content) => json!({"content": content}),
                        Err(e) => {
                            json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true})
                        }
                    };
                    reply(&mut out, &id, body)?;
                    continue;
                }
                let result = api.ensure(&home).and_then(|_| call_tool(&api, name, &args));
                let body = match result {
                    Ok(v) => {
                        json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v)?}], "structuredContent": if v.is_object() { v.clone() } else { json!({"items": v}) }})
                    }
                    Err(e) => {
                        json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true})
                    }
                };
                reply(&mut out, &id, body)?;
            }
            _ => reply_error(&mut out, &id, -32601, &format!("unknown method {method}"))?,
        }
    }
    Ok(())
}
