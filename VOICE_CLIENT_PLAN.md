# OpenCode Voice Client — Project Plan

## Project Summary

Build a **standalone Rust TUI application** that acts as a voice input client for [OpenCode](https://github.com/anomalyco/opencode). The client captures speech, transcribes it locally via Whisper.cpp, visualizes audio input as a spectrogram, maintains a focus stack derived from OpenCode's session log, and sends transcripts + focus context to OpenCode's server via its SDK. Everything runs 100% locally — no cloud STT, no cloud LLM dependency from the client's perspective.

The client is intentionally "dumb" — it is a microphone, a visualizer, and a focus tracker. All natural language understanding and command execution is delegated to OpenCode and whatever model the user has configured there. The client does not parse, interpret, or map natural language to commands.

> "The voice client's job is literally: capture audio, visualize it, turn it into text (Whisper), send text + focus context to OpenCode, listen for events back, update focus stack. That's it."

---

## Architecture Overview

```
┌─────────────────────────────────────┐
│         Voice Client (Rust)         │
│                                     │
│  ┌───────────┐    ┌──────────────┐  │
│  │ Audio In  │───▶│ Ring Buffer  │  │
│  │ (cpal)    │    └──────┬───────┘  │
│  └───────────┘           │          │
│                    ┌─────┴─────┐    │
│                    │           │    │
│              ┌─────▼───┐ ┌────▼──┐  │
│              │ FFT     │ │Whisper│  │
│              │(rustfft)│ │ .cpp  │  │
│              └─────┬───┘ └───┬───┘  │
│                    │         │      │
│              ┌─────▼───┐ ┌───▼────┐ │
│              │Spectro- │ │Tran-   │ │
│              │gram     │ │script  │ │
│              │Widget   │ └───┬────┘ │
│              └─────┬───┘     │      │
│                    │         │      │
│  ┌─────────────────▼─────────▼───┐  │
│  │         TUI (ratatui)         │  │
│  │  ┌─────────────────────────┐  │  │
│  │  │ Focus Stack Display     │  │  │
│  │  │ 📄 App.tsx              │  │  │
│  │  │ 📁 src/                 │  │  │
│  │  │ 🌿 main                │  │  │
│  │  └─────────────────────────┘  │  │
│  │  ┌─────────────────────────┐  │  │
│  │  │ ▁▂▃▅▇█▇▅▃▂▁ Listening  │  │  │
│  │  └─────────────────────────┘  │  │
│  │  ┌─────────────────────────┐  │  │
│  │  │ "open the config file"  │  │  │
│  │  └─────────────────────────┘  │  │
│  └───────────────────────────────┘  │
└──────────────┬──────────────────────┘
               │ SSE/HTTP
         ┌─────▼──────┐
         │  OpenCode  │
         │  Server    │
         └────────────┘
```

---

## Modules

The crate is organized into five modules. Each has a narrow, well-defined responsibility.

### 1. Audio Module

**Responsibility:** Capture mic input, manage a ring buffer, provide PCM data to both the FFT (for visualization) and Whisper (for transcription).

**Crates:**

- `cpal` — cross-platform audio capture
- A ring buffer (custom or `ringbuf` crate)

**Behavior:**

- On push-to-talk key down: start writing audio samples to the ring buffer.
- While recording: the ring buffer is continuously read by the FFT for spectrogram rendering.
- On push-to-talk key up: stop recording, hand the complete PCM buffer to the STT module.
- Consider adding VAD (voice activity detection) as a future enhancement for optional hands-free mode, but start with push-to-talk — it's simpler and better UX for command entry.

### 2. STT Module

**Responsibility:** Take an audio buffer, return a transcript string. Nothing else.

**Crates:**

- `whisper-rs` — Rust bindings for whisper.cpp

**Behavior:**

- Receives a complete PCM audio buffer from the audio module.
- Runs Whisper inference on it.
- Returns the transcript as a `String`.
- The `base` or `small` Whisper model is sufficient — utterances are short imperative sentences (3–10 words), not long-form speech.
- The Whisper model file (`.bin`) lives on disk alongside the binary. The user downloads it once.

**Performance notes:**

- On CPU: acceptable latency for short utterances.
- On GPU (Metal/CUDA): near real-time.
- This is the heaviest computation in the client, but still much lighter than the LLM inference OpenCode is doing.

### 3. Viz Module (Spectrogram Widget)

**Responsibility:** Take FFT frequency data and render a spectrogram or audio visualization in the TUI.

**Crates:**

- `rustfft` — for Short-Time Fourier Transform
- `ratatui` — for rendering

**Rendering approach:**

There are several options for terminal-based audio visualization, ordered by complexity:

1. **Half-block characters** (`▀▁▂▃▄▅▆▇█`) — simplest, good for a waveform or bar-style frequency display. Each character cell gives 8 levels of vertical resolution. This is what tools like `cava` use.

2. **Braille Unicode characters** (`⠁⠂⠃...⣿`) — 2×4 dot grid per character cell, giving 2x–4x resolution of regular characters. Good for higher-fidelity spectrograms.

3. **Color-mapped filled blocks** — use `█` characters with 256-color or true-color (24-bit) mapping. Map amplitude to a color gradient (plasma/magma colormap). Each cell is one time-frequency bin. Color is more important than character resolution for spectrograms.

**Start with option 1** (half-block waveform). It's the simplest, gives clear "mic is hearing you" feedback, and can be upgraded later. A live bouncing bar of `▁▂▃▄▅▆▇█` characters is sufficient MVP.

> "A spectrogram is genuinely useful here beyond just looking cool. It gives the user visual feedback about whether the mic is picking up clean speech vs. background noise, which helps them calibrate their voice and environment."

**Implementation:**

- Read the latest chunk from the ring buffer.
- Run a rolling FFT / STFT on it.
- Bin frequency magnitudes into however many rows the TUI widget has.
- Map magnitudes to characters/colors and render.
- Throttle redraws to 15–30fps, only update the spectrogram region.

**Reference:** `cava` (terminal audio visualizer written in C) has solved many of the gotchas around terminal refresh rates and audio buffering.

### 4. Focus Module

**Responsibility:** Maintain a view of the user's current working context (which file, directory, branch, etc. they're "focused" on) by reading OpenCode's session log/events.

> "The focus stack doesn't even need to be its own persistent state. It's a _view_ over OpenCode's session history. You're just reading the log, filtering for events that imply focus changes, and projecting the most recent N entries as the stack."

**This is NOT a model. It is a deterministic rules engine.** There is no LLM inference in this module.

#### Core Data Model

The focus module maintains:

- A **history list**: an ordered, immutable timeline of focus-relevant entries derived from OpenCode's log. This is a read-only projection — it cannot get out of sync with OpenCode because it _is_ OpenCode's log, filtered.
- A **pointer** (index): the user's current position in the history. This is the only mutable client-owned state.
- A **follow mode** boolean: when true, the pointer auto-advances to the latest entry when new events arrive (like `less +F` tailing a log). When false, the pointer stays put while the list grows above it.

> "History is immutable and complete. Focus is just 'what am I looking at right now,' which is one pointer."

#### Typed Focus Entries

Each entry in the history has a type, derived from the tool event:

```rust
enum FocusEntry {
    File(PathBuf),
    Directory(PathBuf),
    Branch(String),
    Commit(String),
}
```

> "You'd almost need typed focus slots rather than a single cursor... Then the parser resolves pronouns based on which slot makes sense for the command."

When sending context to OpenCode, include the current focus entry and optionally the most recent entry of each type (so the LLM can resolve "it" based on the verb — "delete it" in a file context vs. branch context).

#### Event-to-Focus Mapping Rules

Pattern match on OpenCode's tool execution events. These events are structured data with known schemas — no NLU required:

```
tool: "read",  args: { path: "src/App.tsx" }           → push File("src/App.tsx")
tool: "write", args: { path: "src/utils.ts" }          → push File("src/utils.ts")
tool: "edit",  args: { filePath: "README.md" }          → push File("README.md")
tool: "bash",  args: { command: "cd src" }              → push Directory("src")
tool: "bash",  args: { command: "git checkout main" }   → push Branch("main")
tool: "bash",  args: { command: "git commit ..." }      → push Commit(hash)
tool: "list",  args: { path: "src/components" }         → push Directory("src/components")
tool: "grep",  args: { ... }                            → no-op (or push if results are file-specific)
```

Bash command parsing for `cd` and `git` uses simple prefix matching or basic regex. It doesn't need to be perfect — it just needs to catch common cases.

> "When OpenCode emits a tool.execute event, you already know which tool and the arguments. So the focus module is just a rules engine... That's a match statement, not a model. Deterministic, instant, zero dependencies."

#### Benefits of Log-Derived Focus

- **No sync issues.** Cannot diverge from OpenCode's state.
- **Session resumption is free.** Restart the client, read the log, rebuild focus. Pick up exactly where you left off.
- **Time travel works.** Scroll the history to see what was focused on earlier.
- **Stateless by default.** The only client-owned state is the pointer index and follow boolean.

> "The voice client becomes almost stateless. The only state it truly owns is: whether the mic is active, the current audio buffer, and which focus entry the user has manually selected via keyboard. Everything else is derived from OpenCode's log."

#### TUI Rendering

Display the history list with the pointer indicated visually:

```
  📄 opencode.json
  📁 src/components/
  📄 routes.ts
▸ 📄 App.tsx          ← pointer (you are here)
  📁 src/
  📄 package.json
```

Everything above the pointer is newer, everything below is older. When follow mode is on and a new event arrives, it appends to the top and the pointer follows.

#### Keyboard Controls

- **Up/Down arrows:** move the pointer through the history.
- **A hotkey (e.g., `f`):** toggle follow mode on/off (like `less +F`).
- **Enter (optional):** when pointer is on a file, could trigger a read in OpenCode to show it.

> "Voice for intent, keyboard for precise corrections. You're never fully dependent on either one."

### 5. Transport Module

**Responsibility:** Communicate with the OpenCode server. Send prompts (transcripts + focus context), receive events via SSE.

**Crates:**

- `reqwest` — HTTP client for sending prompts
- `tokio-tungstenite` or `reqwest` with SSE support — for receiving events
- `serde` / `serde_json` — for serializing/deserializing

**OpenCode SDK details:**

- OpenCode runs as a server (client/server architecture).
- The SDK (`@opencode-ai/sdk`) provides type-safe TypeScript definitions for HTTP and WebSocket/SSE APIs, defined in an OpenAPI spec. The Rust client will need to implement the relevant endpoints manually or generate a client from the OpenAPI spec.
- Key operations: `client.session.prompt()` to send a message, SSE event stream subscription for tool execution events.

**Outbound (client → OpenCode):**

When a transcript is ready, package it with the current focus context and send as a prompt:

```json
{
  "prompt": "open the config file",
  "context": {
    "current_focus": { "type": "file", "path": "src/App.tsx" },
    "recent_focus": {
      "file": "src/App.tsx",
      "directory": "src/",
      "branch": "main"
    }
  }
}
```

The exact format depends on how OpenCode's prompt API accepts context. The focus information can be prepended to the prompt as natural language, e.g.:

```
[Context: Currently focused on file src/App.tsx, in directory src/, on branch main]

User said: "open the config file"
```

**Inbound (OpenCode → client):**

Subscribe to the SSE event stream. Filter for `tool.execute` events and forward them to the focus module for processing. Other events (responses, status updates) can be displayed in the TUI if desired.

---

## Event Flow for a Single Utterance

1. User holds push-to-talk key — audio starts flowing to ring buffer.
2. Ring buffer feeds FFT → spectrogram widget updates in real time.
3. Ring buffer simultaneously accumulates PCM for Whisper.
4. User releases key — audio stops, Whisper processes the buffer.
5. Transcript appears in the TUI: `"open the config file"`.
6. Client checks the focus pointer for current context.
7. Sends transcript + focus context to OpenCode server via SDK.
8. OpenCode's agent processes it (using whatever model the user configured).
9. Client receives events back via SSE — e.g., a `tool.execute` for `read` on `opencode.json`.
10. Focus module appends `File("opencode.json")` to history, pointer advances (if follow mode is on).
11. Focus stack display updates: `📄 opencode.json` now at top.

---

## Interaction Example

```
voice: "go to the src folder"
  → OpenCode runs: list src/
  → focus pushes: 📁 src/

voice: "show me what's in here"
  → OpenCode runs: list src/  (context tells it "here" = src/)
  → output displayed in OpenCode TUI
  → arrow keys on voice client to browse focus history

[down arrow to highlight Button.tsx from earlier]
  → pointer moves to: 📄 Button.tsx

voice: "open it"
  → context sends: current focus = Button.tsx
  → OpenCode runs: read src/Button.tsx
```

> "The focus stack becomes the shared state between the voice parser, the keyboard input, and the TUI display. Everyone reads from and writes to the same stack."

---

## Crate Dependencies Summary

| Crate                  | Purpose                            |
| ---------------------- | ---------------------------------- |
| `cpal`                 | Cross-platform audio capture       |
| `rustfft`              | FFT for spectrogram                |
| `whisper-rs`           | Whisper.cpp bindings for local STT |
| `ratatui`              | TUI framework                      |
| `crossterm`            | Terminal backend for ratatui       |
| `tokio`                | Async runtime                      |
| `reqwest`              | HTTP client for OpenCode SDK       |
| `serde` / `serde_json` | Serialization                      |
| `ringbuf`              | Ring buffer (or custom)            |

## Distribution

Compiles to a **single binary** with no runtime dependencies. The user needs:

- The binary itself
- A Whisper model file (e.g., `ggml-base.bin`, ~150MB download once)
- OpenCode installed and running as a server

No Python, no node_modules, no Docker.

---

## Build Order / Implementation Phases

### Phase 1: Audio + STT + Minimal TUI

- Set up `cpal` audio capture with push-to-talk.
- Integrate `whisper-rs`, transcribe on key release.
- Display transcript in a basic ratatui layout.
- Goal: press a key, speak, see text on screen.

### Phase 2: Spectrogram Visualization

- Add FFT via `rustfft`.
- Build a ratatui widget using half-block characters.
- Show live audio levels during push-to-talk.
- Goal: visual feedback that the mic is active and hearing you.

### Phase 3: OpenCode Transport

- Connect to OpenCode's server via HTTP/SSE.
- Send transcripts as prompts.
- Subscribe to SSE event stream and log incoming events.
- Goal: speak a command, see OpenCode execute it.

### Phase 4: Focus Module

- Parse OpenCode's session log / SSE events for tool executions.
- Build the focus history list with typed entries.
- Implement pointer navigation (up/down arrows, follow mode toggle).
- Render the focus stack widget in the TUI.
- Inject focus context into prompts sent to OpenCode.
- Goal: the TUI shows what you're focused on, and pronouns resolve correctly.

### Phase 5: Polish

- Refine spectrogram rendering (upgrade to braille or color-mapped if desired).
- Add status indicators (recording state, OpenCode connection status, model info).
- Error handling for mic failures, OpenCode disconnects, Whisper errors.
- Configuration file for keybindings, Whisper model path, OpenCode server URL.
- README and installation instructions.

---

## Key Design Principles

1. **The client is dumb.** All intelligence lives in OpenCode's agent. The client is a microphone, a visualizer, and a focus tracker. Improvements to the underlying LLM automatically make the voice experience better.

2. **Everything is local.** No cloud dependencies from the voice client. Whisper runs locally. The LLM is OpenCode's concern — it can be local (Ollama) or cloud, the client doesn't care.

3. **Focus is derived, not invented.** The focus stack is a read-only view over OpenCode's session log. The only client-owned mutable state is the pointer position and follow mode.

4. **Voice + keyboard, not voice only.** Voice for intent, keyboard for precise corrections. The user is never fully dependent on either modality.

5. **Graceful degradation.** If speech recognition botches something, the focus stack is still there, keyboard still works. A misparse doesn't derail the flow.

> "This reframes the whole project: it's less 'build a voice-controlled shell' and more 'add voice as an input modality to an existing AI coding agent, with a spatial context model to make the voice input more precise.'"
