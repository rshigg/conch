// Transport Module - HTTP/SSE communication with OpenCode server via reqwest

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// Connection status for the OpenCode server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connected,
    Reconnecting,
}

/// A tool execution event parsed from SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEvent {
    pub tool: String,
    pub input: serde_json::Value,
    pub state: String,
}

/// Session info from OpenCode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub slug: Option<String>,
    pub directory: Option<String>,
    pub title: Option<String>,
}

/// Events emitted from the SSE stream to the TUI.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    Connected,
    SessionStatus { session_id: String, busy: bool },
    Tool(ToolEvent),
    Heartbeat,
}

/// HTTP + SSE client for OpenCode.
pub struct OpenCodeClient {
    base_url: String,
    http: reqwest::Client,
    session_id: Option<String>,
}

impl OpenCodeClient {
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
            session_id: None,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Health check: GET /global/health
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/global/health", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    /// List sessions: GET /session
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let url = format!("{}/session", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("list sessions failed: {}", resp.status()));
        }
        let body: serde_json::Value = resp.json().await?;
        // The response is an array of session objects
        let sessions: Vec<SessionInfo> = match body {
            serde_json::Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect(),
            _ => Vec::new(),
        };
        Ok(sessions)
    }

    /// Create a new session: POST /session
    pub async fn create_session(&mut self) -> Result<String> {
        let url = format!("{}/session", self.base_url);
        let resp = self.http.post(&url).json(&serde_json::json!({})).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("create session failed: {}", resp.status()));
        }
        let body: serde_json::Value = resp.json().await?;
        let id = body["id"]
            .as_str()
            .ok_or_else(|| anyhow!("no session id in response"))?
            .to_string();
        self.session_id = Some(id.clone());
        Ok(id)
    }

    /// Use an existing session ID.
    pub fn set_session(&mut self, id: String) {
        self.session_id = Some(id);
    }

    /// Send a prompt: POST /session/{id}/prompt_async
    pub async fn send_prompt(&self, text: &str) -> Result<()> {
        let session_id = self
            .session_id
            .as_ref()
            .ok_or_else(|| anyhow!("no session set"))?;
        let url = format!("{}/session/{}/prompt_async", self.base_url, session_id);
        let body = serde_json::json!({
            "parts": [{"type": "text", "text": text}]
        });
        let resp = self.http.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("send prompt failed ({}): {}", status, text));
        }
        Ok(())
    }

    /// Subscribe to SSE events: GET /event
    /// Returns a response whose body can be streamed line by line.
    pub async fn subscribe_events(
        &self,
    ) -> Result<reqwest::Response> {
        let url = format!("{}/event", self.base_url);
        // SSE streams are long-lived — use a client with no timeout.
        let sse_client = reqwest::Client::builder().build()?;
        let resp = sse_client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("SSE connect failed: {}", resp.status()));
        }
        Ok(resp)
    }
}

/// Parse a single SSE `data:` JSON line into a `ServerEvent`, if relevant.
///
/// Returns `None` for events we don't care about (heartbeats return Some for keep-alive tracking).
pub fn parse_sse_event(json_str: &str) -> Option<ServerEvent> {
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let event_type = v["type"].as_str()?;

    match event_type {
        "server.connected" => Some(ServerEvent::Connected),
        "server.heartbeat" => Some(ServerEvent::Heartbeat),
        "session.status" => {
            let props = &v["properties"];
            let session_id = props["sessionID"].as_str()?.to_string();
            let busy = props["status"]["type"].as_str()? == "busy";
            Some(ServerEvent::SessionStatus { session_id, busy })
        }
        "message.part.updated" => {
            let part = &v["properties"]["part"];
            if part["type"].as_str()? != "tool" {
                return None;
            }
            // Tool name is at part.tool (not part.toolName)
            let tool = part["tool"].as_str().unwrap_or("").to_string();
            // State is an object: part.state.status is the status string,
            // part.state.input holds the tool input
            let state_obj = &part["state"];
            let state = state_obj["status"].as_str().unwrap_or("unknown").to_string();
            let input = state_obj
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Some(ServerEvent::Tool(ToolEvent {
                tool,
                input,
                state,
            }))
        }
        _ => None,
    }
}

/// Extract SSE data lines from a chunk of bytes.
/// SSE format: lines starting with "data: " followed by JSON, separated by blank lines.
pub fn extract_sse_data_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")))
        .collect()
}

/// Build a prompt string that prepends focus context as natural language.
pub fn format_prompt_with_context(
    transcript: &str,
    current_focus: Option<&str>,
    recent_files: &[&str],
) -> String {
    if current_focus.is_none() && recent_files.is_empty() {
        return transcript.to_string();
    }

    let mut parts = Vec::new();
    if let Some(focus) = current_focus {
        parts.push(format!("Currently focused on: {}", focus));
    }
    if !recent_files.is_empty() {
        parts.push(format!("Recent files: {}", recent_files.join(", ")));
    }
    format!("[Context: {}]\n{}", parts.join(". "), transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== ToolEvent Parsing Tests =====

    #[test]
    fn test_parse_tool_event_from_sse() {
        // Real OpenCode format: tool name at part.tool, state is object with status/input
        let json = r#"{"type":"message.part.updated","properties":{"part":{"id":"p1","sessionID":"s1","messageID":"m1","type":"tool","tool":"read","state":{"status":"completed","input":{"filePath":"src/main.rs"},"output":"...","title":"","metadata":{},"time":{"start":1,"end":2}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "read");
                assert_eq!(te.input["filePath"], "src/main.rs");
                assert_eq!(te.state, "completed");
            }
            _ => panic!("expected Tool event"),
        }
    }

    #[test]
    fn test_parse_write_tool_event() {
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"tool","tool":"write","state":{"status":"completed","input":{"filePath":"src/utils.ts","content":"hello"},"output":"","title":"","metadata":{},"time":{"start":1,"end":2}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "write");
                assert_eq!(te.input["filePath"], "src/utils.ts");
            }
            _ => panic!("expected Tool event"),
        }
    }

    #[test]
    fn test_parse_bash_tool_event() {
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"tool","tool":"bash","state":{"status":"running","input":{"command":"cd src","description":"change dir"},"time":{"start":1}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "bash");
                assert_eq!(te.input["command"], "cd src");
                assert_eq!(te.state, "running");
            }
            _ => panic!("expected Tool event"),
        }
    }

    #[test]
    fn test_parse_list_tool_event() {
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"tool","tool":"list","state":{"status":"completed","input":{"path":"src/components"},"output":"...","title":"","metadata":{},"time":{"start":1,"end":2}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "list");
                assert_eq!(te.input["path"], "src/components");
            }
            _ => panic!("expected Tool event"),
        }
    }

    #[test]
    fn test_parse_edit_tool_event() {
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"tool","tool":"edit","state":{"status":"completed","input":{"filePath":"README.md","oldString":"a","newString":"b"},"output":"","title":"","metadata":{},"time":{"start":1,"end":2}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "edit");
                assert_eq!(te.input["filePath"], "README.md");
            }
            _ => panic!("expected Tool event"),
        }
    }

    // ===== SSE Line Parsing Tests =====

    #[test]
    fn test_extract_sse_data_lines() {
        let chunk = "data: {\"type\":\"server.connected\",\"properties\":{}}\n\ndata: {\"type\":\"server.heartbeat\",\"properties\":{}}\n\n";
        let lines = extract_sse_data_lines(chunk);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("server.connected"));
        assert!(lines[1].contains("server.heartbeat"));
    }

    #[test]
    fn test_extract_sse_data_lines_no_space() {
        let chunk = "data:{\"type\":\"server.heartbeat\",\"properties\":{}}\n\n";
        let lines = extract_sse_data_lines(chunk);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_extract_sse_ignores_non_data_lines() {
        let chunk = "event: message\ndata: {\"type\":\"server.connected\",\"properties\":{}}\nid: 1\n\n";
        let lines = extract_sse_data_lines(chunk);
        assert_eq!(lines.len(), 1);
    }

    // ===== Server Event Parsing Tests =====

    #[test]
    fn test_parse_server_connected() {
        let json = r#"{"type":"server.connected","properties":{}}"#;
        let event = parse_sse_event(json).unwrap();
        assert!(matches!(event, ServerEvent::Connected));
    }

    #[test]
    fn test_parse_server_heartbeat() {
        let json = r#"{"type":"server.heartbeat","properties":{}}"#;
        let event = parse_sse_event(json).unwrap();
        assert!(matches!(event, ServerEvent::Heartbeat));
    }

    #[test]
    fn test_parse_session_status_busy() {
        let json = r#"{"type":"session.status","properties":{"sessionID":"ses_abc123","status":{"type":"busy"}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::SessionStatus { session_id, busy } => {
                assert_eq!(session_id, "ses_abc123");
                assert!(busy);
            }
            _ => panic!("expected SessionStatus"),
        }
    }

    #[test]
    fn test_parse_session_status_idle() {
        let json = r#"{"type":"session.status","properties":{"sessionID":"ses_abc123","status":{"type":"idle"}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::SessionStatus { session_id, busy } => {
                assert_eq!(session_id, "ses_abc123");
                assert!(!busy);
            }
            _ => panic!("expected SessionStatus"),
        }
    }

    // ===== Filtering Tests =====

    #[test]
    fn test_text_part_event_ignored() {
        // message.part.updated with type "text" should return None (not a tool)
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"text","text":"Hello world"}}}"#;
        assert!(parse_sse_event(json).is_none());
    }

    #[test]
    fn test_session_updated_ignored() {
        let json = r#"{"type":"session.updated","properties":{"info":{"id":"ses_abc"}}}"#;
        assert!(parse_sse_event(json).is_none());
    }

    #[test]
    fn test_message_updated_ignored() {
        let json = r#"{"type":"message.updated","properties":{"info":{"id":"msg_abc"}}}"#;
        assert!(parse_sse_event(json).is_none());
    }

    // ===== Malformed Event Handling =====

    #[test]
    fn test_malformed_json_returns_none() {
        assert!(parse_sse_event("not json at all").is_none());
    }

    #[test]
    fn test_missing_type_field_returns_none() {
        assert!(parse_sse_event(r#"{"properties":{}}"#).is_none());
    }

    #[test]
    fn test_unknown_event_type_returns_none() {
        let json = r#"{"type":"some.unknown.event","properties":{}}"#;
        assert!(parse_sse_event(json).is_none());
    }

    #[test]
    fn test_tool_event_missing_tool_name() {
        // Should still parse, with empty tool name
        let json = r#"{"type":"message.part.updated","properties":{"part":{"type":"tool","state":{"status":"completed","input":{},"output":"","title":"","metadata":{},"time":{"start":1,"end":2}}}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::Tool(te) => {
                assert_eq!(te.tool, "");
            }
            _ => panic!("expected Tool event"),
        }
    }

    // ===== Context Formatting Tests =====

    #[test]
    fn test_format_prompt_no_context() {
        let result = format_prompt_with_context("hello", None, &[]);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_format_prompt_with_current_focus() {
        let result = format_prompt_with_context("do something", Some("src/main.rs"), &[]);
        assert!(result.contains("Currently focused on: src/main.rs"));
        assert!(result.contains("do something"));
    }

    #[test]
    fn test_format_prompt_with_recent_files() {
        let result = format_prompt_with_context("fix it", None, &["src/a.rs", "src/b.rs"]);
        assert!(result.contains("Recent files: src/a.rs, src/b.rs"));
        assert!(result.contains("fix it"));
    }

    #[test]
    fn test_format_prompt_with_full_context() {
        let result = format_prompt_with_context(
            "refactor this",
            Some("src/lib.rs"),
            &["src/main.rs"],
        );
        assert!(result.contains("Currently focused on: src/lib.rs"));
        assert!(result.contains("Recent files: src/main.rs"));
        assert!(result.contains("refactor this"));
    }

    // ===== Prompt JSON Structure Tests =====

    #[test]
    fn test_prompt_json_structure() {
        let body = serde_json::json!({
            "parts": [{"type": "text", "text": "hello world"}]
        });
        assert_eq!(body["parts"][0]["type"], "text");
        assert_eq!(body["parts"][0]["text"], "hello world");
    }

    // ===== OpenCodeClient Construction Tests =====

    #[test]
    fn test_client_creation() {
        let client = OpenCodeClient::new("http://127.0.0.1:4096");
        assert_eq!(client.base_url, "http://127.0.0.1:4096");
        assert!(client.session_id.is_none());
    }

    #[test]
    fn test_client_trailing_slash_stripped() {
        let client = OpenCodeClient::new("http://127.0.0.1:4096/");
        assert_eq!(client.base_url, "http://127.0.0.1:4096");
    }

    #[test]
    fn test_set_session() {
        let mut client = OpenCodeClient::new("http://127.0.0.1:4096");
        client.set_session("ses_abc123".into());
        assert_eq!(client.session_id(), Some("ses_abc123"));
    }

    // ===== Connection Status Tests =====

    #[test]
    fn test_connection_status_variants() {
        let s = ConnectionStatus::Connected;
        assert_eq!(s, ConnectionStatus::Connected);
        assert_ne!(s, ConnectionStatus::Disconnected);
        assert_ne!(s, ConnectionStatus::Reconnecting);
    }

    // ===== ToolEvent Serialization =====

    #[test]
    fn test_tool_event_serialization() {
        let te = ToolEvent {
            tool: "read".into(),
            input: serde_json::json!({"path": "src/main.rs"}),
            state: "completed".into(),
        };
        let json = serde_json::to_string(&te).unwrap();
        let parsed: ToolEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool, "read");
        assert_eq!(parsed.input["path"], "src/main.rs");
    }

    // ===== Real SSE Data Tests (from captured OpenCode output) =====

    #[test]
    fn test_parse_real_sse_session_status() {
        let json = r#"{"type":"session.status","properties":{"sessionID":"ses_3c6990794ffeX4V5KrEdZSit0b","status":{"type":"busy"}}}"#;
        let event = parse_sse_event(json).unwrap();
        match event {
            ServerEvent::SessionStatus { session_id, busy } => {
                assert!(session_id.starts_with("ses_"));
                assert!(busy);
            }
            _ => panic!("expected SessionStatus"),
        }
    }

    #[test]
    fn test_parse_real_sse_text_part() {
        // Real captured text message part - should be ignored (not a tool)
        let json = r#"{"type":"message.part.updated","properties":{"part":{"id":"prt_c3967d681001RKu70R46CIko4s","sessionID":"ses_3c6990794ffeX4V5KrEdZSit0b","messageID":"msg_c3967d67e0010qOCQM41ygxzRM","type":"text","text":"Say hello in exactly 3 words"}}}"#;
        assert!(parse_sse_event(json).is_none());
    }

    // ===== Network-dependent tests (ignored by default) =====

    #[tokio::test]
    #[ignore]
    async fn test_health_check_live() {
        let client = OpenCodeClient::new("http://127.0.0.1:4096");
        let healthy = client.health_check().await.unwrap();
        assert!(healthy);
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_sessions_live() {
        let client = OpenCodeClient::new("http://127.0.0.1:4096");
        let sessions = client.list_sessions().await.unwrap();
        // Should return at least an empty list without error
        assert!(sessions.len() >= 0);
    }

    #[tokio::test]
    #[ignore]
    async fn test_create_session_live() {
        let mut client = OpenCodeClient::new("http://127.0.0.1:4096");
        let id = client.create_session().await.unwrap();
        assert!(!id.is_empty());
        assert!(client.session_id().is_some());
    }

    #[tokio::test]
    async fn test_send_prompt_no_session() {
        let client = OpenCodeClient::new("http://127.0.0.1:4096");
        let result = client.send_prompt("test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no session set"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_health_check_server_down() {
        let client = OpenCodeClient::new("http://127.0.0.1:19999");
        let result = client.health_check().await;
        assert!(result.is_err());
    }
}
