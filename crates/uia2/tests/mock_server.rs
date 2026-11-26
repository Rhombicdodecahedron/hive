//! The session against a stand-in for the phone's UiAutomator2 server and a fake adb.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::thread;

use hive_adb::Adb;
use hive_core::{Selector, Serial, Ui};
use hive_uia2::Session;
use serde_json::{Value, json};

#[test]
fn finds_reads_and_taps() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let url = req.url().to_owned();
            let compact = serde_json::from_str::<Value>(&body)
                .map(|v| v.to_string())
                .unwrap_or(body);
            log.lock()
                .unwrap()
                .push(format!("{} {url} {compact}", req.method()));
            let value: Value = match (req.method().as_str(), url.as_str()) {
                ("GET", "/status") => json!({"ready": true}),
                ("POST", "/session") => json!({"sessionId": "S1", "capabilities": {}}),
                ("POST", u) if u.ends_with("/elements") => {
                    json!([{"ELEMENT": "e1", "element-6066-11e4-a52e-4f735466cecf": "e1"}])
                }
                ("GET", u) if u.ends_with("/text") => json!("@momentsdegta6"),
                ("GET", u) if u.ends_with("/rect") => {
                    json!({"x": 10, "y": 20, "width": 300, "height": 40})
                }
                _ => Value::Null,
            };
            let out = json!({"sessionId": "S1", "value": value}).to_string();
            req.respond(tiny_http::Response::from_string(out)).unwrap();
        }
    });

    let dir = std::env::temp_dir().join(format!("hive-uia2-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("adb");
    fs::write(&exe, format!("#!/bin/sh\ncase \"$*\" in *\"forward tcp:0\"*) echo {port} ;; *\"am instrument\"*) sleep 30 ;; esac\n")).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();

    let dev = Adb::with_exe(exe).device(Serial::new("PHONE1").unwrap());
    let mut s = Session::start(dev).unwrap();
    let found = s
        .find_all(
            &Selector::ui("new UiSelector().textStartsWith(\"@\")"),
            None,
        )
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(s.text(&found[0]).unwrap(), "@momentsdegta6");
    assert_eq!(s.rect(&found[0]).unwrap().center(), (160, 40));
    s.tap(5, 6).unwrap();
    s.set_text(&found[0], "hello #gta6").unwrap();
    drop(s);

    let calls = seen.lock().unwrap().join("\n");
    assert!(calls.contains("POST /session/S1/elements {\"selector\":\"new UiSelector().textStartsWith(\\\"@\\\")\",\"strategy\":\"-android uiautomator\"}"), "{calls}");
    assert!(
        calls.contains("POST /session/S1/appium/gestures/click {\"offset\":{\"x\":5,\"y\":6}}"),
        "{calls}"
    );
    assert!(
        calls.contains(
            "POST /session/S1/element/e1/value {\"replace\":true,\"text\":\"hello #gta6\"}"
        ),
        "{calls}"
    );
    assert!(calls.contains("DELETE /session/S1"), "{calls}");
}
