//! The whole pipeline against a made-up video and a pretend LLM server: words in, clips out.
//! Skipped when ffmpeg isn't installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use hive_studio::llm::{Anthropic, OpenAi};
use hive_studio::{Llm, Options, Part, Studio, Word};
use serde_json::{Value, json};

fn ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hive-studio-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 60 s of a bright test pattern with a tone, and 20 s of black from 60 s.
fn video(dir: &Path) -> PathBuf {
    let path = dir.join("source.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=60",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=640x360:r=30:d=20",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=80",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "2:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let words: Vec<Word> = (0..75)
        .map(|i| Word {
            w: if i % 7 == 6 {
                "boum.".into()
            } else {
                format!("mot{i}")
            },
            s: i as f64 + 0.1,
            e: i as f64 + 0.6,
        })
        .collect();
    hive_studio::words::save(&hive_studio::words::path_for(&path), &words).unwrap();
    path
}

/// A pretend server: answers each POST with the next canned body, and keeps what it was sent.
fn server(answers: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", server.server_addr().to_ip().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    thread::spawn(move || {
        let mut answers = answers.into_iter();
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            log.lock().unwrap().push(json!({"url": req.url(), "body": serde_json::from_str::<Value>(&body).unwrap_or(Value::Null)}));
            let answer = answers.next().unwrap_or(json!({}));
            req.respond(tiny_http::Response::from_string(answer.to_string()))
                .unwrap();
        }
    });
    (addr, seen)
}

fn chat(text: &str) -> Value {
    json!({"choices": [{"message": {"content": text}}]})
}

#[test]
fn makes_the_clips_the_llm_picks_and_leaves_out_the_dark_one() {
    if !ffmpeg() {
        eprintln!("ffmpeg isn't installed: skipped");
        return;
    }
    let dir = temp("auto");
    let src = video(&dir);
    let moments = json!({"moments": [
        {"segments": [["00:02.0", "00:12.0"], ["00:30.0", "00:38.0"]], "hook": "Quand ça part en vrille dans GTA 6 ?", "caption": "Boum 💥 #gta6 #fail", "kind": "fail", "why": "it explodes", "score": 9},
        {"segments": [["01:02.0", "01:19.0"]], "hook": "La nuit noire", "caption": "On voit rien #gta6", "score": 8},
        {"segments": [["00:40.0", "00:56.0"]], "hook": "Le deuxième", "caption": "Deux #gta6", "score": 6},
        {"segments": [["00:41.0", "00:55.0"]], "hook": "Le même", "caption": "doublon", "score": 5}
    ]});
    let (base, seen) = server(vec![
        chat(&format!("Voici :\n```json\n{moments}\n```")),
        chat(r#"{"keep": true, "reason": "something happens"}"#),
        chat(r#"{"keep": true, "reason": "fine"}"#),
    ]);
    let llm: Box<dyn Llm> = Box::new(OpenAi::new(
        &format!("{base}/v1"),
        Some("k".into()),
        "test-model",
    ));
    let studio = Studio::new(dir.join("work"), Some(llm), None);
    let out = dir.join("clips");
    let opts = Options {
        count: 2,
        ..Options::default()
    };
    let made = studio
        .auto(&src, &out, &opts, &|line| eprintln!("  {line}"))
        .unwrap();

    assert_eq!(
        made.len(),
        2,
        "two clips: the dark one and the duplicate are left out"
    );
    assert_eq!(
        made[0].file.file_name().unwrap(),
        "01-quand-ca-part-en-vrille-dans-gta-6.mp4"
    );
    assert!(
        (made[0].seconds - 18.0).abs() < 1.5,
        "two parts, about 18 s: {}",
        made[0].seconds
    );
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(&made[0].file)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1080,1920");
    let captions = std::fs::read_to_string(out.join("captions.md")).unwrap();
    assert!(captions.contains("## 01-quand-ca-part-en-vrille-dans-gta-6.mp4\nBoum 💥 #gta6 #fail"));
    assert!(out.join("moments.json").is_file());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "one pick, then one look per bright moment");
    assert_eq!(seen[0]["url"], "/v1/chat/completions");
    assert!(
        seen[0]["body"]["messages"][1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("00:00.1 mot0 mot1")
    );
    let image = &seen[1]["body"]["messages"][1]["content"][0];
    assert!(
        image["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,iVBOR")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn anthropic_sends_pictures_as_base64_blocks() {
    let (base, seen) = server(vec![json!({"content": [{"type": "text", "text": "ok"}]})]);
    let llm = Anthropic::with_base(&base, "key".into(), "claude-test");
    let answer = llm
        .ask(
            "sys",
            &[Part::Png(b"\x89PNG".to_vec()), Part::Text("hi".into())],
        )
        .unwrap();
    assert_eq!(answer, "ok");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0]["url"], "/v1/messages");
    assert_eq!(seen[0]["body"]["system"], "sys");
    assert_eq!(
        seen[0]["body"]["messages"][0]["content"][0]["source"]["data"],
        "iVBORw=="
    );
}

#[test]
fn a_refusing_llm_says_what_to_check() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", server.server_addr().to_ip().unwrap());
    thread::spawn(move || {
        for req in server.incoming_requests() {
            let _ = req.respond(
                tiny_http::Response::from_string(r#"{"error": {"message": "bad key"}}"#)
                    .with_status_code(401),
            );
        }
    });
    let err = OpenAi::new(&base, None, "m")
        .ask("s", &[Part::Text("x".into())])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("bad key") && err.contains("check the API key"),
        "{err}"
    );
}
