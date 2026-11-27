#![cfg(windows)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::{Value, json};

struct Vault(PathBuf);

impl Vault {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "Wikra MCP {} {now} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        for folder in [
            "wiki/concepts",
            "raw/share",
            "raw/private",
            ".flash-marking-down",
        ] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        Self(root)
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

fn call(id: u64, name: &str, args: Value) -> Value {
    rpc(id, "tools/call", json!({"name":name,"arguments":args}))
}

fn exchange(root: &Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wikra-mcp"))
        .arg("--vault")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn content(value: &Value) -> &Value {
    &value["result"]["structuredContent"]
}

#[test]
fn real_stdio_round_trip_without_creating_or_migrating_state_files() {
    let vault = Vault::new();
    fs::write(
        vault.0.join("wiki/concepts/Welcome.md"),
        "# Welcome\n\nA shared overview.",
    )
    .unwrap();
    fs::write(vault.0.join("SCHEMA.md"), "# Vault schema").unwrap();
    fs::write(
        vault.0.join(".flash-marking-down/Hidden.md"),
        "# Secret never exported",
    )
    .unwrap();
    let state = vault.0.join(".flash-marking-down");
    let mut calls = vec![
        rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        rpc(2, "ping", json!({})),
        rpc(3, "tools/list", json!({})),
        call(4, "search", json!({"query":"welcome"})),
        call(
            5,
            "read_page",
            json!({"path":".flash-marking-down/Hidden.md"}),
        ),
        rpc(6, "resources/list", json!({})),
        rpc(7, "resources/read", json!({"uri":"wikra://vault/schema"})),
        rpc(8, "resources/read", json!({"uri":"wikra://vault/index"})),
        rpc(9, "unknown", json!({})),
        call(10, "search", json!({"query":"q","limit":21})),
    ];
    let result = exchange(&vault.0, &calls);
    assert_eq!(result.len(), 10);
    assert_eq!(result[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(result[1]["result"], json!({}));
    assert_eq!(
        result[2]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["search", "read_page", "get_evidence"]
    );
    assert_eq!(content(&result[3])["indexState"], "files_only");
    assert_eq!(
        content(&result[3])["hits"][0]["path"],
        "wiki/concepts/Welcome.md"
    );
    assert_eq!(result[4]["result"]["isError"], true);
    assert_eq!(
        result[5]["result"]["resources"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result[6]["result"]["contents"][0]["text"], "# Vault schema");
    assert!(
        result[7]["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Welcome")
    );
    assert_eq!(result[8]["error"]["code"], -32601);
    assert_eq!(result[9]["error"]["code"], -32602);
    assert!(!vault.0.join(".wikra").exists());
    assert_eq!(fs::read_dir(&state).unwrap().count(), 1);

    let db = Connection::open(state.join("state.sqlite")).unwrap();
    db.pragma_update(None, "application_id", i64::from_be_bytes(*b"\0\0\0\0FMD1"))
        .unwrap();
    db.pragma_update(None, "user_version", 7).unwrap();
    drop(db);
    calls.truncate(5);
    let before = fs::read(state.join("state.sqlite")).unwrap();
    let result = exchange(&vault.0, &calls);
    assert_eq!(content(&result[3])["indexState"], "files_only");
    assert_eq!(fs::read(state.join("state.sqlite")).unwrap(), before);
    assert_eq!(fs::read_dir(&state).unwrap().count(), 2);
}

#[test]
fn real_stdio_enforces_quote_only_and_explicit_folder_privacy() {
    let vault = Vault::new();
    let page = "wiki/concepts/Source.md";
    let raw = "raw/share/Evidence.md";
    let front = "---\r\nstatus: active\r\nsupersedes: [Older]\r\nsuperseded_by: []\r\nagent_export:\r\n  kind: skill\r\nwikra_pending_conflicts: [legacy]\r\n---\r\n";
    fs::write(
        vault.0.join(page),
        format!(
            "{front}# Source\r\n\r\n公开短引[^w1]。\r\n\r\n[^w1]: [来源](<{raw}>) —— “逐字短引”\r\n"
        ),
    )
    .unwrap();
    fs::write(
        vault.0.join(raw),
        "Previous context。\r\n\r\n共享搜索词，逐字短引旁有原段。\r\n\r\nNext context。",
    )
    .unwrap();
    fs::write(
        vault.0.join("raw/private/Private.md"),
        "# Private\n\n共享搜索词，保密原文。",
    )
    .unwrap();
    fs::create_dir_all(vault.0.join("notes-obsidian")).unwrap();
    fs::write(
        vault.0.join("notes-obsidian/Arch.md"),
        "# Arch\n\nExisting Obsidian knowledge page outside wiki/.",
    )
    .unwrap();
    drop(wikra::storage::Storage::open(&vault.0, Path::new("raw/notes"), ".wikra").unwrap());
    let state = vault.0.join(".flash-marking-down");
    let db_path = state.join("state.sqlite");
    let requests = [
        rpc(1, "initialize", json!({"protocolVersion":"unrecognized"})),
        call(2, "search", json!({"query":"共享搜索词"})),
        call(3, "read_page", json!({"title":"Source"})),
        call(4, "read_page", json!({"path":raw})),
        call(
            5,
            "get_evidence",
            json!({"citationRef":format!("{page}#w1")}),
        ),
        call(6, "read_page", json!({"path":"raw/private/Private.md"})),
        call(7, "read_page", json!({"path":"notes-obsidian/Arch.md"})),
    ];
    let result = exchange(&vault.0, &requests);
    assert_eq!(result[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(content(&result[1])["indexState"], "db");
    assert_eq!(content(&result[1])["hits"], json!([]));
    assert_eq!(content(&result[2])["frontmatter"], front);
    assert!(
        result[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("这页有")
    );
    assert_eq!(
        content(&result[2])["citations"][0]["citationRef"],
        format!("{page}#w1")
    );
    assert_eq!(result[3]["result"]["isError"], true);
    let evidence = content(&result[4]);
    assert_eq!(evidence["quote"], "逐字短引");
    assert_eq!(evidence["verified"], true);
    assert_eq!(evidence["context"], "quote");
    assert_eq!(evidence["reason"], "raw 未授权");
    assert_eq!(evidence["paragraph"], Value::Null);
    assert!(!result[4].to_string().contains("Previous context"));
    assert_eq!(result[5]["result"]["isError"], true);
    assert!(
        content(&result[6])["body"]
            .as_str()
            .unwrap()
            .contains("Existing Obsidian knowledge page outside wiki/.")
    );

    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO workspace_settings VALUES ('mcp_raw_allow', ?1)",
        [json!(["raw/share", "wiki", "..", ".flash-marking-down"]).to_string()],
    )
    .unwrap();
    drop(conn);
    let before = fs::read(&db_path).unwrap();
    let allowed = exchange(&vault.0, &requests);
    assert_eq!(content(&allowed[1])["hits"].as_array().unwrap().len(), 1);
    assert_eq!(content(&allowed[1])["hits"][0]["path"], raw);
    assert!(
        content(&allowed[3])["body"]
            .as_str()
            .unwrap()
            .contains("共享搜索词")
    );
    let evidence = content(&allowed[4]);
    assert_eq!(evidence["context"], "paragraph");
    assert!(
        evidence["paragraph"]
            .as_str()
            .unwrap()
            .contains("Previous context")
    );
    assert!(
        evidence["paragraph"]
            .as_str()
            .unwrap()
            .contains("Next context")
    );
    assert_eq!(allowed[5]["result"]["isError"], true);
    assert_eq!(fs::read(&db_path).unwrap(), before);
    assert!(!state.join("state.sqlite-wal").exists());
    assert!(!state.join("state.sqlite-shm").exists());
}

#[test]
fn running_stdio_observes_raw_permission_revocation_during_wal_use() {
    let vault = Vault::new();
    let raw = "raw/share/Live.md";
    fs::write(vault.0.join(raw), "# Live\n\nprivate context").unwrap();
    let storage =
        wikra::storage::Storage::open(&vault.0, Path::new("raw/notes"), ".wikra").unwrap();
    let path = vault.0.join(".flash-marking-down/state.sqlite");
    let mut db = Connection::open(path).unwrap();
    db.execute(
        "INSERT INTO workspace_settings VALUES ('mcp_raw_allow', '[\"raw/share\"]')",
        [],
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wikra-mcp"))
        .env("WIKRA_VAULT", &vault.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut exchange = |request: Value| -> Value {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = exchange(rpc(
        1,
        "initialize",
        json!({"protocolVersion":"2025-03-26"}),
    ));
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    let tx = db.transaction().unwrap();
    tx.execute(
        "UPDATE workspace_settings SET value = '[]' WHERE key = 'mcp_raw_allow'",
        [],
    )
    .unwrap();
    let allowed = exchange(call(2, "read_page", json!({"path":raw})));
    assert!(
        content(&allowed)["body"]
            .as_str()
            .unwrap()
            .contains("private context")
    );
    tx.commit().unwrap();
    let denied = exchange(call(3, "read_page", json!({"path":raw})));
    assert_eq!(denied["result"]["isError"], true);
    let search = exchange(call(4, "search", json!({"query":"private context"})));
    assert_eq!(content(&search)["indexState"], "db");
    assert_eq!(content(&search)["hits"], json!([]));
    drop(stdin);
    assert!(child.wait_with_output().unwrap().status.success());
    drop(storage);
}
