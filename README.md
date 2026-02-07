# Conch — Voice Input Client for OpenCode

A standalone Rust TUI application that provides voice input for [OpenCode](https://github.com/anomalyco/opencode) using local speech-to-text via Whisper.cpp.

## Phases 1-3 Complete ✓

- ✅ Audio capture via cpal with push-to-talk
- ✅ Local speech-to-text transcription via whisper-rs
- ✅ Minimal TUI displaying transcripts
- ✅ Ring buffer for audio data
- ✅ Automatic resampling to 16kHz for Whisper
- ✅ Live FFT-based spectrogram during recording via rustfft
- ✅ Half-block character bars with green/yellow/red color coding
- ✅ RMS-gated noise floor so silence stays quiet on screen
- ✅ HTTP transport to OpenCode server (prompt sending)
- ✅ SSE event stream subscription with auto-reconnection
- ✅ Session management (reuse or create)
- ✅ Prompt confirmation flow (Enter to send, Backspace to discard)
- ✅ Connection status indicator in TUI

**Goal achieved**: Press Space, speak, confirm with Enter, and the transcript is sent to OpenCode.

## Prerequisites

1. **Rust toolchain** (1.75+)
2. **CMake** (for building whisper-rs/whisper.cpp)
3. **A Whisper model file**

### Download a Whisper Model

Download the `base.en` model (recommended for English voice commands):

```bash
curl -L -o ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

Model options (in order of size/accuracy):

- `ggml-tiny.en.bin` — 75 MB, fastest, least accurate
- `ggml-base.en.bin` — 142 MB, good balance (recommended)
- `ggml-small.en.bin` — 466 MB, slower, more accurate

Place the model file in the project directory or specify its path when running.

## Building

```bash
cargo build --release
```

The binary will be at `target/release/conch`.

## Running

```bash
# Using default model path (ggml-base.en.bin in current directory)
./target/release/conch

# Or specify a model path
./target/release/conch path/to/ggml-base.en.bin
```

## Usage

1. Press **Space** to start recording
2. Speak your command (e.g., "open the config file")
3. Press **Space** again to stop recording
4. Wait for transcription (1-3 seconds)
5. See the transcript appear in the TUI (highlighted as pending)
6. Press **Enter** to send to OpenCode, or **Backspace** to discard

Press **q** or **Esc** to quit.

### OpenCode Connection

Conch connects to OpenCode's server at `http://127.0.0.1:4096`. Start OpenCode in server mode first:

```bash
opencode server
```

Conch will automatically find or create a session for the current directory. You can also specify a session ID:

```bash
./target/release/conch ggml-base.en.bin --session ses_abc123
```

The title bar shows connection status (`[OC: connected]`, `[OC: busy]`, etc.) and the session slug.

Debug logs are written to `conch.log` in the working directory.

## Architecture

Conch is organized into five modules:

1. **audio** — cpal-based microphone capture, ring buffer
2. **stt** — whisper-rs wrapper for local transcription
3. **viz** — FFT-based spectrogram visualization
4. **focus** — Focus stack derived from OpenCode events (Phase 4)
5. **transport** — HTTP/SSE communication with OpenCode (Phase 3)

## Testing

```bash
# Run tests
cargo test

# Run specific module tests
cargo test viz::tests

# Run tests that require a Whisper model (ignored by default)
cargo test -- --ignored
```

## Current Status

### Phase 1: Audio + STT + Minimal TUI ✓

- [x] Audio capture with push-to-talk
- [x] Whisper transcription
- [x] Basic TUI with transcript display

### Phase 2: Spectrogram Visualization ✓

- [x] FFT computation via rustfft with Hann windowing
- [x] Log-frequency binning for perceptual accuracy
- [x] Half-block character bars (`▁▂▃▄▅▆▇█`) with green/yellow/red color coding
- [x] RMS-gated noise floor to suppress mic self-noise
- [x] Real-time update during recording (~20fps)
- [x] 37 passing unit tests (18 audio + 19 viz)
- [x] Async tokio runtime

### Phase 3: OpenCode Transport ✓

- [x] HTTP client for sending prompts via `prompt_async`
- [x] SSE event stream subscription with auto-reconnection
- [x] Tool execution event parsing (`message.part.updated` with `type: "tool"`)
- [x] Session management (list, reuse, create)
- [x] Prompt confirmation flow (Enter/Backspace)
- [x] Connection status display in TUI title bar
- [x] Async tokio runtime
- [x] 32 passing unit tests

### Phase 4: Focus Module

- [ ] Event-to-focus mapping rules
- [ ] Focus history derived from OpenCode log
- [ ] Pointer navigation (up/down arrows)
- [ ] Follow mode (like `less +F`)
- [ ] Context injection into prompts

### Phase 5: Polish

- [ ] Spectrogram rendering refinements
- [ ] Status indicators
- [ ] Configuration file
- [ ] Error handling improvements
- [ ] Installation documentation

## Design Principles

1. **The client is dumb** — All intelligence lives in OpenCode's agent
2. **Everything is local** — No cloud STT, Whisper runs locally
3. **Focus is derived** — Read-only view over OpenCode's session log
4. **Voice + keyboard** — Voice for intent, keyboard for precision
5. **Graceful degradation** — Misparse doesn't derail the flow

## Performance Notes

- **CPU**: Acceptable latency for short utterances (1-3 seconds)
- **GPU (Metal/CUDA)**: Near real-time transcription
- **Memory**: Ring buffer sized for 60 seconds of audio
- **Model size**: Base model (~142 MB) is sufficient for commands

## Dependencies

Core:

- `cpal` — Cross-platform audio capture
- `whisper-rs` — Whisper.cpp Rust bindings
- `ratatui` — TUI framework
- `crossterm` — Terminal backend

- `rustfft` — FFT for spectrogram visualization

- `reqwest` — HTTP/SSE client for OpenCode
- `tokio` — Async runtime

## License

See LICENSE file.

## See Also

- [VOICE_CLIENT_PLAN.md](VOICE_CLIENT_PLAN.md) — Complete project plan
- [TESTING.md](TESTING.md) — Comprehensive test documentation
- [OpenCode](https://github.com/anomalyco/opencode) — The AI coding agent this client interfaces with
