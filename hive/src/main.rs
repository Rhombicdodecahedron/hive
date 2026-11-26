//! hive: plug Android phones in, and they post your videos and slideshows to TikTok.

mod engine;
mod home;
mod mcp;
mod queue;
mod serve;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use hive_core::{Handle, Media, Post, Serial, Visibility};

use crate::engine::Hive;
use crate::queue::{Queue, local, parse_time};

#[derive(Parser)]
#[command(
    name = "hive",
    version,
    about = "Plug Android phones in, and they post your videos and slideshows to TikTok."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The phones plugged in, and whether each one can post.
    Phones,
    /// Install hive's UI server on the phones (done by itself on first use).
    Setup {
        #[arg(long)]
        phone: Option<Serial>,
    },
    /// Open TikTok on a phone and read the account it is on.
    Check {
        #[arg(long)]
        phone: Option<Serial>,
    },
    /// Post a video, or pictures as a slideshow. With --at, plan it for `hive serve`.
    Post {
        /// One video, or pictures in the order they should show.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// The TikTok account to post on (@handle).
        #[arg(long, short)]
        account: Handle,
        /// The description, #hashtags included.
        #[arg(long, short, default_value = "", conflicts_with = "caption_file")]
        caption: String,
        /// Read the description from a file.
        #[arg(long)]
        caption_file: Option<PathBuf>,
        /// A slideshow's title.
        #[arg(long)]
        title: Option<String>,
        /// Search TikTok's sounds for this and use the first one.
        #[arg(long)]
        sound: Option<String>,
        /// everyone, friends or only-you.
        #[arg(long, default_value = "everyone")]
        visibility: Visibility,
        /// Fill everything in but leave Post for you to tap.
        #[arg(long)]
        draft: bool,
        /// Use this phone only.
        #[arg(long)]
        phone: Option<Serial>,
        /// Plan it: "2026-11-27 18:00" (this computer's time zone).
        #[arg(long)]
        at: Option<String>,
    },
    /// The planned posts (`hive queue rm <id>` to remove one).
    Queue {
        #[command(subcommand)]
        action: Option<QueueAction>,
    },
    /// Run the planned posts, and serve the dashboard and the API.
    Serve {
        /// Where to listen (default from the config: 127.0.0.1:7878).
        #[arg(long)]
        address: Option<String>,
        /// Start again by itself when the hive binary is replaced.
        #[arg(long)]
        reload: bool,
    },
    /// Which phone posts for an account (or every account and its phone).
    Bind {
        account: Option<Handle>,
        /// The phone's serial; leave out to let any phone holding the account post.
        phone: Option<Serial>,
    },
    /// A picture of the phone's screen, or its UI elements as XML (to fix a selector).
    Screen {
        #[arg(long)]
        phone: Option<Serial>,
        #[arg(long)]
        xml: bool,
        #[arg(long, short, default_value = "screen.png")]
        out: PathBuf,
    },
    /// Use a phone over Wi-Fi: after `adb tcpip 5555` or Android's wireless debugging.
    Connect { address: String },
    /// Where hive keeps its config and data.
    Paths,
    /// Serve Hive's tools over MCP (stdio), for Claude and other MCP clients.
    Mcp,
}

#[derive(Subcommand)]
enum QueueAction {
    Rm { id: String },
}

fn say(line: &str) {
    println!("  {line}");
}

fn pick_phone(hive: &Hive, phone: Option<Serial>) -> Result<Serial> {
    Ok(match phone {
        Some(p) => p,
        None => hive.only_phone()?,
    })
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Paths => {
            let home = home::Home::locate()?;
            println!("config: {}", home.config_dir.display());
            println!("data:   {}", home.data_dir.display());
        }
        Command::Phones => {
            let hive = Hive::open()?;
            let phones = hive.phones()?;
            if phones.is_empty() {
                println!(
                    "No phone plugged in. Plug one in with USB debugging on (Developer options)."
                );
            }
            for v in phones {
                let p = &v.phone;
                let state = match (&v.blocker, v.busy) {
                    (_, true) => "posting right now".to_owned(),
                    (Some(b), _) => b.clone(),
                    (None, _) => "ready".to_owned(),
                };
                println!("{}  {}  {state}", p.label(), p.serial);
                if let Some(s) = &p.status {
                    let lock = match (s.lock.secure, s.lock.showing) {
                        (Some(true), _) => "code",
                        (Some(false), _) => "no code",
                        _ => "?",
                    };
                    println!(
                        "    Android {}, battery {}%{}, screen {}, lock: {lock}, TikTok {}",
                        s.android,
                        s.battery.map_or("?".into(), |b| b.to_string()),
                        if s.charging { " charging" } else { "" },
                        if s.awake == Some(true) { "on" } else { "off" },
                        if s.tiktok.is_some() {
                            "installed"
                        } else {
                            "missing"
                        },
                    );
                }
                if let Some(c) = &v.notes.current {
                    let seen: Vec<String> =
                        v.notes.accounts.keys().map(|h| format!("@{h}")).collect();
                    println!("    TikTok on @{c}; accounts seen: {}", seen.join(", "));
                }
                if !v.bound.is_empty() {
                    let bound: Vec<String> = v.bound.iter().map(|h| format!("@{h}")).collect();
                    println!("    posts for {}", bound.join(", "));
                }
            }
        }
        Command::Setup { phone } => {
            let hive = Hive::open()?;
            let phones = match phone {
                Some(p) => vec![p],
                None => hive
                    .phones()?
                    .into_iter()
                    .filter(|v| v.phone.state == "device")
                    .map(|v| v.phone.serial)
                    .collect(),
            };
            if phones.is_empty() {
                bail!("no phone plugged in");
            }
            for p in phones {
                let installed = hive.setup(&p)?;
                println!(
                    "{p}: UI server {}",
                    if installed {
                        "installed"
                    } else {
                        "already there"
                    }
                );
            }
        }
        Command::Check { phone } => {
            let hive = Hive::open()?;
            let p = pick_phone(&hive, phone)?;
            let h = hive.check(&p, &mut say)?;
            println!("TikTok on {p} is on @{h}");
        }
        Command::Post {
            files,
            account,
            caption,
            caption_file,
            title,
            sound,
            visibility,
            draft,
            phone,
            at,
        } => {
            let caption = match caption_file {
                Some(f) => std::fs::read_to_string(&f)
                    .with_context(|| format!("reading {}", f.display()))?,
                None => caption,
            };
            let files = files
                .into_iter()
                .map(|f| {
                    std::fs::canonicalize(&f).with_context(|| format!("{} not found", f.display()))
                })
                .collect::<Result<Vec<_>>>()?;
            let post = Post {
                account,
                media: Media::from_files(files)?,
                caption,
                title,
                sound,
                visibility,
                draft,
            };
            let hive = Hive::open()?;
            if let Some(at) = at {
                let entry = Queue::new(&hive.home).add(post, parse_time(&at)?)?;
                println!(
                    "planned for {} ({}): `hive serve` will post it",
                    local(entry.at),
                    entry.id
                );
                return Ok(());
            }
            println!("posting on @{}", post.account);
            let cancel = hive_core::Cancel::new();
            let on_stop = cancel.clone();
            // first Ctrl+C: stop at the next step and clean the phone up; second: quit now
            let _ = ctrlc::set_handler(move || {
                if on_stop.is_cancelled() {
                    std::process::exit(130);
                }
                eprintln!("\ncancelling at the next step (Ctrl+C again to quit now)");
                on_stop.cancel();
            });
            let done = hive.post(&post, phone.as_ref(), &cancel, &mut say)?;
            for n in &done.report.notes {
                say(n);
            }
            for p in &done.report.problems {
                println!("  to finish on the phone: {p}");
            }
            println!(
                "{}",
                if done.report.posted {
                    "posted"
                } else {
                    "left on the phone for you to finish"
                }
            );
        }
        Command::Queue { action } => {
            let hive = Hive::open()?;
            let q = Queue::new(&hive.home);
            match action {
                Some(QueueAction::Rm { id }) => {
                    let e = q.remove(&id)?;
                    println!("removed {} ({})", e.id, local(e.at));
                }
                None => {
                    let entries = q.entries()?;
                    if entries.is_empty() {
                        println!("nothing planned");
                    }
                    for e in entries {
                        let status = format!("{:?}", e.status).to_lowercase();
                        let first = e.post.media.files()[0]
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        println!(
                            "{}  {}  {status:<8} @{}  {first}  {}",
                            e.id,
                            local(e.at),
                            e.post.account,
                            e.note
                        );
                    }
                }
            }
        }
        Command::Serve { address, reload } => serve::serve(Hive::open()?, address, reload)?,
        Command::Mcp => mcp::run()?,
        Command::Bind { account, phone } => {
            let home = home::Home::locate()?;
            let mut config = home.config()?;
            match account {
                None => {
                    if config.accounts.is_empty() {
                        println!("no account bound: any phone holding an account posts for it");
                    }
                    for (h, a) in &config.accounts {
                        println!(
                            "@{h}  {}",
                            a.phone
                                .as_ref()
                                .map_or("any phone".into(), |p| p.to_string())
                        );
                    }
                }
                Some(h) => {
                    config.accounts.entry(h.clone()).or_default().phone = phone.clone();
                    home.save_config(&config)?;
                    match phone {
                        Some(p) => println!("@{h} posts from {p}"),
                        None => println!("@{h} posts from any phone that holds it"),
                    }
                }
            }
        }
        Command::Screen { phone, xml, out } => {
            let hive = Hive::open()?;
            let p = pick_phone(&hive, phone)?;
            if xml {
                let out = if out.as_os_str() == "screen.png" {
                    PathBuf::from("screen.xml")
                } else {
                    out
                };
                std::fs::write(&out, hive.screen_xml(&p)?)?;
                println!("saved {}", out.display());
            } else {
                std::fs::write(&out, hive.device(&p).screencap()?)?;
                println!("saved {}", out.display());
            }
        }
        Command::Connect { address } => {
            let hive = Hive::open()?;
            println!("{}", hive.adb.connect(&address)?.trim());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hive: {e:#}");
            ExitCode::FAILURE
        }
    }
}
