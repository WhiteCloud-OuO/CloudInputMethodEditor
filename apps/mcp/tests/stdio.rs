//! 真实 stdio 握手、工具列表和一次词库查询。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

fn sample_dict() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sample/dict.tsv")
}

fn child(args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_cloudime-mcp-server"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 cloudime-mcp-server")
}

fn request(
    reader: &mut BufReader<std::process::ChildStdout>,
    writer: &mut std::process::ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let message = if params.is_null() {
        json!({"jsonrpc": "2.0", "id": id, "method": method})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    };
    writeln!(writer, "{message}").expect("写 stdio");
    let mut line = String::new();
    reader.read_line(&mut line).expect("读 stdio");
    serde_json::from_str(&line).expect("响应是 JSON")
}

fn initialize(
    reader: &mut BufReader<std::process::ChildStdout>,
    writer: &mut std::process::ChildStdin,
) {
    let response = request(
        reader,
        writer,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "cloudime-mcp-test", "version": "0"}
        }),
    );
    assert_eq!(response["result"]["serverInfo"]["name"], "cloudime");
    writeln!(
        writer,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .expect("写 initialized");
}

fn lookup(
    reader: &mut BufReader<std::process::ChildStdout>,
    writer: &mut std::process::ChildStdin,
    id: u64,
    pinyin: &str,
) -> Value {
    request(
        reader,
        writer,
        id,
        "tools/call",
        json!({
            "name": "cloudime_lookup_pinyin",
            "arguments": {"pinyin": pinyin, "limit": 10}
        }),
    )
}

#[test]
fn talks_mcp_over_stdio_and_reads_the_bundled_dictionary() {
    let mut process = child(&["--dict", sample_dict().to_str().unwrap()]);
    let mut writer = process.stdin.take().unwrap();
    let mut reader = BufReader::new(process.stdout.take().unwrap());
    initialize(&mut reader, &mut writer);

    let tools = request(&mut reader, &mut writer, 2, "tools/list", Value::Null);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("tools 是数组")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"cloudime_lookup_pinyin"));
    assert!(names.contains(&"cloudime_segment_pinyin"));
    assert!(names.contains(&"cloudime_dictionary_info"));

    let response = lookup(&mut reader, &mut writer, 3, "kaifa");
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["preferred"], "kai'fa");
    assert!(
        result["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "开发"),
        "{result}"
    );

    drop(writer);
    assert!(process.wait().unwrap().success());
}

#[test]
fn imports_third_party_dictionaries_but_skips_the_user_word_bank() {
    let root = std::env::temp_dir().join(format!("cloudime-mcp-test-{}", std::process::id()));
    let bank = root.join("WordBank");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&bank).unwrap();
    std::fs::copy(sample_dict(), bank.join("Dict.tsv")).unwrap();
    std::fs::write(bank.join("extra.tsv"), "云朵\tyun duo\t100\n").unwrap();
    std::fs::write(bank.join("UserWordBank.db"), "私密\tsi mi\t999\n").unwrap();

    let mut process = child(&["--word-bank", bank.to_str().unwrap()]);
    let mut writer = process.stdin.take().unwrap();
    let mut reader = BufReader::new(process.stdout.take().unwrap());
    initialize(&mut reader, &mut writer);

    let response = lookup(&mut reader, &mut writer, 2, "yunduo");
    let result = &response["result"]["structuredContent"];
    assert!(
        result["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "云朵"),
        "第三方词库应参与查询：{result}"
    );

    let response = lookup(&mut reader, &mut writer, 3, "simi");
    let result = &response["result"]["structuredContent"];
    assert!(
        !result["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "私密"),
        "不能读 UserWordBank.db：{result}"
    );

    let response = request(
        &mut reader,
        &mut writer,
        4,
        "tools/call",
        json!({"name": "cloudime_dictionary_info", "arguments": {}}),
    );
    let dictionaries = response["result"]["structuredContent"]["dictionaries"]
        .as_array()
        .unwrap();
    assert!(
        dictionaries
            .iter()
            .any(|dictionary| dictionary["kind"] == "main")
    );
    assert!(
        dictionaries
            .iter()
            .any(|dictionary| dictionary["kind"] == "imported")
    );
    assert!(
        dictionaries
            .iter()
            .all(|dictionary| dictionary["name"] != "UserWordBank.db")
    );

    drop(writer);
    assert!(process.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&root);
}
