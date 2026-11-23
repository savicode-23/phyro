use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

#[test]
fn current_discovery_and_legacy_initialize_expose_the_same_proposal_only_tools() {
    for modern in [false, true] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_phyro-cli"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let meta = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"studio","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
        let mut send = |id: Option<u32>, method: &str, mut params: Value| {
            if modern {
                params["_meta"] = meta.clone();
            }
            let mut req = json!({"jsonrpc":"2.0","method":method,"params":params});
            if let Some(id) = id {
                req["id"] = json!(id);
            }
            writeln!(input, "{req}").unwrap();
            input.flush().unwrap();
            if id.is_none() {
                return Value::Null;
            }
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["id"], json!(id), "{reply}");
            assert!(reply.get("error").is_none(), "{reply}");
            reply["result"].clone()
        };
        if modern {
            let info = send(Some(1), "server/discover", json!({}));
            assert!(info["supportedVersions"]
                .as_array()
                .unwrap()
                .contains(&json!("2026-07-28")));
        } else {
            let info = send(
                Some(1),
                "initialize",
                json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"studio","version":"1"},"capabilities":{}}),
            );
            assert_eq!(info["protocolVersion"], "2025-11-25");
            send(None, "notifications/initialized", json!({}));
        }
        let discovery = send(Some(2), "tools/list", json!({}));
        let tools = discovery["tools"].as_array().unwrap();
        for name in ["begin_draft", "append_draft_step", "finish_draft"] {
            assert!(tools.iter().any(|tool| tool["name"] == name));
        }
        let propose = tools.iter().find(|t| t["name"] == "propose_edits").unwrap();
        assert_eq!(
            propose["inputSchema"]["properties"]["operations"]["items"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        let denied = send(
            Some(3),
            "tools/call",
            json!({"name":"get_context","arguments":{}}),
        );
        assert_eq!(denied["isError"], true);
        let pairing = send(
            Some(4),
            "tools/call",
            json!({"name":"connect","arguments":{}}),
        );
        assert_eq!(
            pairing["structuredContent"]["state"], "pairing",
            "{pairing}"
        );
        assert_eq!(
            send(
                Some(5),
                "tools/call",
                json!({"name":"connect","arguments":{}})
            )["structuredContent"]["code"],
            pairing["structuredContent"]["code"]
        );
        drop(input);
        assert!(child.wait().unwrap().success());
    }
}
