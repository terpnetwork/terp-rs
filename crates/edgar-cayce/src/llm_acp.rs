//! Minimal ACP client for one chat turn over stdio.
//!
//! Wire format: ACP is newline-delimited JSON-RPC 2.0 (one JSON object per
//! line), NOT LSP-style `Content-Length` framing. opencode 1.18.x silently
//! ignores Content-Length-framed input, which is why every turn used to hang
//! until the profile timeout.
//!
//! Flow: `initialize` → `session/new {cwd, mcpServers}` → `session/set_model`
//! → `session/prompt`. Text arrives as `session/update` notifications with
//! `sessionUpdate = "agent_message_chunk"`; the `session/prompt` response
//! (`stopReason`) ends the turn.
//!
//! The agent may ask us for permission (`session/request_permission`) or for
//! fs/terminal access. We always reject: RLM's WASIX guest is the only code
//! executor. The child runs in its own process group and the whole group is
//! SIGKILLed when the turn ends, times out, or is cancelled.

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tracing::{debug, warn};

use crate::llm::{flatten_messages, Message};

pub struct AcpSpec<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub model: Option<&'a str>,
    pub timeout: Duration,
}

/// Kills the child's whole process group on drop (timeout, error, or the
/// caller's future being cancelled), so no `opencode acp` is left orphaned.
struct GroupKill(Option<u32>);

impl Drop for GroupKill {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            kill_group(pid);
        }
    }
}

fn kill_group(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub async fn chat_acp(spec: AcpSpec<'_>, messages: &[Message]) -> Result<String> {
    let prompt = flatten_messages(messages);
    let mut cmd = Command::new(spec.command);
    cmd.args(spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    if let Some(cwd) = spec.cwd {
        cmd.current_dir(cwd);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn ACP `{}`", spec.command))?;
    let _guard = GroupKill(child.id());
    let stdin = child.stdin.take().context("ACP stdin")?;
    let stdout = child.stdout.take().context("ACP stdout")?;

    // Drain stderr so a chatty agent cannot block on a full pipe; keep a tail
    // for error messages.
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = child.stderr.take() {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut t = tail.lock().unwrap();
                t.push_str(&String::from_utf8_lossy(&buf[..n]));
                if t.len() > 2048 {
                    let cut = crate::util::floor_char_boundary(&t, t.len() - 1024);
                    let keep = t[cut..].to_string();
                    *t = keep;
                }
            }
        });
    }

    let cwd = spec
        .cwd
        .map(str::to_string)
        .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| "/".into());
    let result = tokio::time::timeout(
        spec.timeout,
        drive_session(stdin, stdout, &cwd, spec.model, &prompt),
    )
    .await;
    let _ = child.start_kill();
    drop(_guard);
    let _ = child.wait().await;
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => {
            let tail = stderr_tail.lock().unwrap().trim().to_string();
            if tail.is_empty() {
                Err(e)
            } else {
                debug!(stderr = %tail, "ACP child stderr");
                Err(e)
            }
        }
        Err(_) => bail!(
            "ACP timed out after {}ms ({})",
            spec.timeout.as_millis(),
            spec.command
        ),
    }
}

struct Conn {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    text: String,
}

async fn drive_session(
    stdin: ChildStdin,
    stdout: ChildStdout,
    cwd: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<String> {
    let mut c = Conn {
        stdin,
        reader: BufReader::new(stdout),
        next_id: 1,
        text: String::new(),
    };
    c.rpc(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientInfo": {"name": "edgar-rlm", "version": "0.1.0"},
            "clientCapabilities": {
                "fs": {"readTextFile": false, "writeTextFile": false},
                "terminal": false,
            }
        }),
    )
    .await
    .context("ACP initialize")?;

    let created = c
        .rpc("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await
        .context("ACP session/new")?;
    let session_id = created["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("ACP session/new missing sessionId"))?
        .to_string();

    if let Some(m) = model.filter(|s| !s.is_empty()) {
        if let Err(e) = c
            .rpc(
                "session/set_model",
                json!({"sessionId": session_id, "modelId": m}),
            )
            .await
        {
            // Newer agents expose the model as a session config option.
            debug!(error = %e, "ACP session/set_model failed; trying set_config_option");
            c.rpc(
                "session/set_config_option",
                json!({"sessionId": session_id, "configId": "model", "value": m}),
            )
            .await
            .with_context(|| format!("ACP could not select model {m}"))?;
        }
    }

    c.text.clear();
    let res = c
        .rpc(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": prompt}],
            }),
        )
        .await
        .context("ACP session/prompt")?;
    let stop = res["stopReason"].as_str().unwrap_or("");
    if c.text.trim().is_empty() {
        bail!("ACP prompt returned no assistant text (stopReason={stop})");
    }
    if stop != "end_turn" && !stop.is_empty() {
        warn!(stop_reason = stop, "ACP turn ended early; using partial text");
    }
    Ok(std::mem::take(&mut c.text))
}

impl Conn {
    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        loop {
            let msg = self.read().await?;
            let is_request = msg.get("method").is_some();
            if !is_request && msg.get("id") == Some(&json!(id)) {
                if let Some(err) = msg.get("error") {
                    let message = err["message"].as_str().unwrap_or("");
                    if message.contains("401")
                        || message.contains("403")
                        || message.to_ascii_lowercase().contains("auth")
                    {
                        bail!("auth 401 ACP {method}: {message}");
                    }
                    bail!("ACP {method} error: {err}");
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            let Some(m) = msg.get("method").and_then(|m| m.as_str()) else {
                continue;
            };
            if let Some(rid) = msg.get("id").cloned() {
                let reply = if m == "session/request_permission" {
                    json!({"jsonrpc": "2.0", "id": rid, "result": {"outcome": deny_outcome(&msg)}})
                } else {
                    json!({"jsonrpc": "2.0", "id": rid,
                           "error": {"code": -32601, "message": "edgar-rlm: client tools disabled"}})
                };
                debug!(method = m, "ACP agent request denied");
                self.write(&reply).await?;
                continue;
            }
            if m == "session/update" {
                let u = &msg["params"]["update"];
                if u["sessionUpdate"].as_str() == Some("agent_message_chunk") {
                    if let Some(t) = u["content"]["text"].as_str() {
                        self.text.push_str(t);
                    }
                }
            }
        }
    }

    async fn write(&mut self, v: &Value) -> Result<()> {
        let mut body = serde_json::to_vec(v)?;
        body.push(b'\n');
        self.stdin.write_all(&body).await.context("ACP stdin")?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read(&mut self) -> Result<Value> {
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await.context("ACP stdout")?;
            if n == 0 {
                bail!("ACP stdout closed");
            }
            let t = line.trim();
            if t.is_empty() || !t.starts_with('{') {
                continue; // tolerate stray log lines / headers
            }
            match serde_json::from_str(t) {
                Ok(v) => return Ok(v),
                Err(e) => debug!(error = %e, "ACP non-JSON line skipped"),
            }
        }
    }
}

/// Pick a reject option if the agent offered one; otherwise cancel.
pub(crate) fn deny_outcome(req: &Value) -> Value {
    let opts = req["params"]["options"].as_array();
    let reject = opts.and_then(|o| {
        o.iter()
            .find(|x| x["kind"].as_str() == Some("reject_once"))
            .or_else(|| {
                o.iter()
                    .find(|x| x["kind"].as_str().unwrap_or("").starts_with("reject"))
            })
    });
    match reject.and_then(|r| r["optionId"].as_str()) {
        Some(id) => json!({"outcome": "selected", "optionId": id}),
        None => json!({"outcome": "cancelled"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_picks_reject_option() {
        let req = json!({"params": {"options": [
            {"optionId": "a", "kind": "allow_once"},
            {"optionId": "r", "kind": "reject_once"}]}});
        assert_eq!(deny_outcome(&req), json!({"outcome": "selected", "optionId": "r"}));
        assert_eq!(deny_outcome(&json!({})), json!({"outcome": "cancelled"}));
    }

    /// Fake ACP agent speaking NDJSON: proves framing, set_model, permission
    /// denial and chunk collection end to end.
    #[tokio::test]
    async fn ndjson_round_trip_with_fake_agent() {
        let script = r#"
import sys, json
def send(o): sys.stdout.write(json.dumps(o)+"\n"); sys.stdout.flush()
for line in sys.stdin:
    m = json.loads(line)
    if "method" not in m: 
        assert m["result"]["outcome"]["outcome"] == "selected"; continue
    i, meth = m.get("id"), m["method"]
    if meth == "initialize": send({"jsonrpc":"2.0","id":i,"result":{"protocolVersion":1}})
    elif meth == "session/new": send({"jsonrpc":"2.0","id":i,"result":{"sessionId":"s1"}})
    elif meth == "session/set_model": send({"jsonrpc":"2.0","id":i,"result":{}})
    elif meth == "session/prompt":
        send({"jsonrpc":"2.0","id":99,"method":"session/request_permission","params":{"options":[{"optionId":"no","kind":"reject_once"}]}})
        for t in ["ibc-go ", "v8"]:
            send({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":t}}}})
        send({"jsonrpc":"2.0","id":i,"result":{"stopReason":"end_turn"}})
"#;
        if std::process::Command::new("python3").arg("-V").output().is_err() {
            return;
        }
        let args = vec!["-c".to_string(), script.to_string()];
        let text = chat_acp(
            AcpSpec {
                command: "python3",
                args: &args,
                cwd: None,
                model: Some("opencode/x"),
                timeout: Duration::from_secs(10),
            },
            &[Message {
                role: "user".into(),
                content: "q".into(),
            }],
        )
        .await
        .unwrap();
        assert_eq!(text, "ibc-go v8");
    }
}
