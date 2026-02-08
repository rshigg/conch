# OpenCode SSE Event Spec Reference

Quick reference for the OpenCode server's SSE event format, derived from the [@opencode-ai/sdk](https://www.npmjs.com/package/@opencode-ai/sdk) TypeScript SDK and the OpenAPI 3.1 spec served by the OpenCode server.

Source repos:
- Server: [github.com/sst/opencode](https://github.com/sst/opencode)
- SDK: [github.com/sst/opencode-sdk-js](https://github.com/sst/opencode-sdk-js)

## SSE Endpoint

```
GET /event
Accept: text/event-stream
```

Wire format: `data: {json}\n\n`

## Event Envelope

Every event has the same top-level shape:

```json
{
  "type": "<event-type>",
  "properties": { ... }
}
```

## Event Types

| Type | Description |
|------|-------------|
| `server.connected` | SSE connection established |
| `server.heartbeat` | Keep-alive |
| `session.updated` | Session metadata changed |
| `session.deleted` | Session removed |
| `session.idle` | Session finished processing |
| `session.error` | Session error |
| `session.status` | Session busy/idle status |
| `message.updated` | Full message object |
| `message.removed` | Message deletion |
| `message.part.updated` | **Part created/updated (tool events live here)** |
| `message.part.removed` | Part deletion |
| `installation.updated` | Version info |
| `permission.updated` | Permission change |
| `file.edited` | File edit event |
| `file.watcher.updated` | File change/rename |
| `storage.write` | Storage event |
| `lsp.client.diagnostics` | LSP diagnostics |
| `ide.installed` | IDE installation |

## Session Status

```json
{
  "type": "session.status",
  "properties": {
    "sessionID": "ses_abc123",
    "status": { "type": "busy" }
  }
}
```

`status.type` is `"busy"` or `"idle"`.

## Message Parts (`message.part.updated`)

```json
{
  "type": "message.part.updated",
  "properties": {
    "part": { ... }
  }
}
```

The `part` object is a discriminated union on `part.type`:

| `part.type` | Description |
|-------------|-------------|
| `"text"` | Text content |
| `"tool"` | **Tool execution** |
| `"file"` | File reference |
| `"step-start"` | Step start marker |
| `"step-finish"` | Step finish marker |
| `"snapshot"` | Snapshot |
| `"patch"` | Patch |

## Tool Part Structure

This is the most important type for Conch. **Note the nesting** -- `state` is an object, not a string.

```json
{
  "id": "prt_abc123",
  "callID": "call_def456",
  "messageID": "msg_ghi789",
  "sessionID": "ses_jkl012",
  "type": "tool",
  "tool": "bash",
  "state": {
    "status": "completed",
    "input": { "command": "ls -la", "description": "List files" },
    "output": "total 42\n...",
    "title": "List files",
    "metadata": { "exit": 0, "truncated": false },
    "time": { "start": 1770490531576, "end": 1770490531601 }
  }
}
```

### Key fields

| Path | Type | Notes |
|------|------|-------|
| `part.tool` | `string` | Tool name. **NOT `toolName`** |
| `part.state` | `object` | **NOT a string.** Contains status, input, output |
| `part.state.status` | `string` | `"pending"` / `"running"` / `"completed"` / `"error"` |
| `part.state.input` | `object` | Tool-specific input (only present in running/completed/error) |
| `part.state.output` | `string` | Tool output (only in completed) |
| `part.state.time` | `object` | `{ start, end? }` timestamps in ms |

### State lifecycle

```
pending  -->  running  -->  completed
                       -->  error
```

Events arrive for each state transition, so a single tool call produces multiple `message.part.updated` events.

## Tool Names and Their Input Schemas

| Tool | `state.input` fields | Notes |
|------|---------------------|-------|
| `read` | `filePath`, `offset?`, `limit?` | Read a file |
| `write` | `filePath`, `content` | Write a file |
| `edit` | `filePath`, `oldString`, `newString`, `replaceAll?` | Edit a file |
| `bash` | `command`, `description`, `timeout?`, `workdir?` | Run shell command |
| `list` | `path?`, `ignore?` | List directory contents |
| `glob` | `pattern`, `path?` | Find files by pattern |
| `grep` | `pattern`, `path?`, `include?` | Search file contents |
| `webfetch` | `url`, `format?` | Fetch URL |
| `task` | `description`, `prompt`, `subagent_type` | Spawn sub-agent |

**All file tools use `filePath`** (camelCase), not `path`. Only `list`, `glob`, and `grep` use `path`.

## REST API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/global/health` | Health check |
| `GET` | `/session` | List sessions |
| `POST` | `/session` | Create session |
| `POST` | `/session/{id}/prompt_async` | Send prompt |
| `GET` | `/event` | SSE event stream |

### Send Prompt

```
POST /session/{id}/prompt_async
Content-Type: application/json

{
  "parts": [{ "type": "text", "text": "your prompt here" }]
}
```

## Real Examples

### Bash tool (completed)

```json
{
  "type": "message.part.updated",
  "properties": {
    "part": {
      "id": "prt_c39758ef1001FqOdPG6Lgc0o1D",
      "sessionID": "ses_3c68c0822ffeghLUamkCOjrEIF",
      "messageID": "msg_c39757d9e001uu3H4gSMzNSE2s",
      "type": "tool",
      "callID": "call_4e01a51527834282a2b9696e",
      "tool": "bash",
      "state": {
        "status": "completed",
        "input": {
          "command": "ls -la",
          "description": "List all files"
        },
        "output": "total 42\ndrwxr-xr-x ...",
        "title": "List all files",
        "metadata": {
          "output": "total 42\ndrwxr-xr-x ...",
          "exit": 0,
          "description": "List all files",
          "truncated": false
        },
        "time": { "start": 1770490531576, "end": 1770490531601 }
      }
    }
  }
}
```

### Read tool (completed)

```json
{
  "type": "message.part.updated",
  "properties": {
    "part": {
      "type": "tool",
      "tool": "read",
      "state": {
        "status": "completed",
        "input": { "filePath": "src/main.rs" },
        "output": "// file contents...",
        "title": "",
        "metadata": {},
        "time": { "start": 1, "end": 2 }
      }
    }
  }
}
```

### Glob tool (completed)

```json
{
  "type": "message.part.updated",
  "properties": {
    "part": {
      "type": "tool",
      "tool": "glob",
      "state": {
        "status": "completed",
        "input": { "pattern": "**/*.rs" },
        "output": "src/main.rs\nsrc/audio.rs\n...",
        "title": "",
        "metadata": { "count": 12, "truncated": false },
        "time": { "start": 1770565982145, "end": 1770565982190 }
      }
    }
  }
}
```

### Text part (non-tool, should be ignored)

```json
{
  "type": "message.part.updated",
  "properties": {
    "part": {
      "id": "prt_c3967d681001RKu70R46CIko4s",
      "type": "text",
      "text": "Here are the files in your directory..."
    }
  }
}
```

## Common Mistakes

1. **`toolName` vs `tool`** -- The field is `part.tool`, not `part.toolName`
2. **`state` is an object** -- `part.state.status` for the string, not `part.state` directly
3. **`input` is inside `state`** -- `part.state.input`, not `part.input`
4. **`filePath` not `path`** -- File tools (read/write/edit) use `filePath`; only list/glob/grep use `path`
5. **Multiple events per tool call** -- A single tool call emits pending, running, then completed/error events
