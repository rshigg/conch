# Testing Documentation for Conch Voice Client

This document describes the comprehensive test suite created for the OpenCode Voice Client (Conch) based on the plan in `plans/VOICE_CLIENT_PLAN.md`.

## Overview

The test suite follows Rust's standard testing conventions with `#[cfg(test)]` modules and is organized to mirror the five-module architecture described in the plan:

1. **Audio Module** (`src/audio.rs`)
2. **STT Module** (`src/stt.rs`)
3. **Viz Module** (`src/viz.rs`)
4. **Focus Module** (`src/focus.rs`)
5. **Transport Module** (`src/transport.rs`)

Additionally, integration tests validate cross-module behavior and the complete utterance flow.

## Test Organization

### Unit Tests (Module-Level)

Each module contains a `#[cfg(test)] mod tests` section with comprehensive unit tests:

#### Audio Module Tests (`src/audio.rs`)

- **Ring buffer operations**: Creation, write, read, circular behavior
- **Push-to-talk state machine**: Start, stop, state transitions
- **Concurrent access**: FFT reads while audio writes
- **Buffer extraction**: Complete PCM buffer for Whisper
- **Edge cases**: Empty recordings, format validation

**Key test count**: 12 unit tests

#### STT Module Tests (`src/stt.rs`)

- **Whisper model management**: Loading, initialization, error handling
- **Transcription**: Basic, short utterances, edge cases (silence, noise)
- **Audio format validation**: Sample rate, channels, bit depth
- **Model variants**: Base vs small model support
- **Performance**: Latency measurements (release builds only)

**Key test count**: 12 unit tests

#### Viz Module Tests (`src/viz.rs`)

- **FFT computation**: Accuracy, known frequency detection
- **Frequency binning**: Magnitude distribution to display rows
- **Character mapping**: Magnitude to half-block characters (`▁▂▃▄▅▆▇█`)
- **Visualization rendering**: Silent audio, loud audio, normalization
- **Performance**: Real-time updates, throttling to 15-30 FPS

**Key test count**: 16 unit tests

#### Focus Module Tests (`src/focus.rs`)

- **FocusEntry enum**: All variants (File, Directory, Branch, Commit)
- **History list**: Creation, append, immutability, ordering
- **Pointer navigation**: Movement, bounds checking, current entry retrieval
- **Follow mode**: Toggle, auto-advancement, manual movement behavior
- **Event-to-focus mapping**: All tool types (read, write, edit, bash, list, grep)
- **Bash command parsing**: cd, git checkout, git commit
- **Context generation**: Current focus, recent focus by type
- **Session resumption**: Rebuilding history from log
- **Time travel**: Historical focus navigation
- **Display formatting**: Icons for each entry type

**Key test count**: 48 unit tests (largest module due to complex rules engine)

#### Transport Module Tests (`src/transport.rs`)

- **HTTP prompt sending**: Basic, with context, error handling
- **Context serialization**: All focus entry types in JSON
- **SSE event stream**: Connection, event parsing, reconnection
- **Tool event parsing**: All tool types from SSE events
- **Connection status**: Connected, disconnected, reconnecting states
- **Error handling**: Network, server, client errors

**Key test count**: 33 unit tests

### Integration Tests

#### Module Integration Tests (`src/integration_tests.rs`)

Tests that validate the complete system behavior across modules:

- **Full utterance flow**: Complete steps 1-11 from plan
- **Interaction examples**: All scenarios from plan (go to folder, show what's here, navigate and open)
- **Multi-module state**: Shared ring buffer, focus context in prompts
- **Error recovery**: Whisper failure, OpenCode disconnect, mic failure
- **Multimodal usage**: Voice + keyboard workflows
- **Performance**: Real-time visualization, transcription latency
- **TUI rendering**: All components displayed and updated
- **Configuration**: File loading, settings application

**Key test count**: 38 integration tests

#### Library Integration Tests (`tests/lib.rs`)

Tests for cross-module communication and public API:

- **Pipeline tests**: Audio→STT, Audio→Viz, STT→Transport, Transport→Focus, Focus→Transport
- **Public API validation**: Each module's public interface
- **Type compatibility**: PCM buffers, FocusEntry serialization, event deserialization
- **Error propagation**: Errors across module boundaries
- **Thread safety**: Send + Sync implementations
- **Module initialization**: All modules can be initialized

### Test Utilities (`src/test_utils.rs`)

Shared test helpers and fixtures:

- **Fixtures**: Sample audio generation (sine waves, silence, noise), sample OpenCode events, sample focus contexts
- **Mocks**: MockAudioDevice, MockOpenCodeServer
- **Assertions**: Float comparisons, FFT validation, path validation

## Total Test Coverage

| Category             | File                       | Test Count    |
| -------------------- | -------------------------- | ------------- |
| Audio Unit Tests     | `src/audio.rs`             | 12            |
| STT Unit Tests       | `src/stt.rs`               | 12            |
| Viz Unit Tests       | `src/viz.rs`               | 16            |
| Focus Unit Tests     | `src/focus.rs`             | 48            |
| Transport Unit Tests | `src/transport.rs`         | 33            |
| Integration Tests    | `src/integration_tests.rs` | 38            |
| Library Tests        | `tests/lib.rs`             | 23            |
| Test Utils           | `src/test_utils.rs`        | 7             |
| **Total**            |                            | **189 tests** |

## Running Tests

### Run all tests

```bash
cargo test
```

### Run specific module tests

```bash
cargo test audio::tests
cargo test stt::tests
cargo test focus::tests
# etc.
```

### Run integration tests only

```bash
cargo test --test lib
```

### Run with output

```bash
cargo test -- --nocapture
```

### Run performance tests (release mode)

```bash
cargo test --release
```

Note: Some performance tests are marked with `#[cfg(not(debug_assertions))]` and will only run in release builds.

## Test Implementation Status

Some tests are currently **stubs with `todo!()`** placeholders. Each test includes:

- A descriptive comment linking it to plan requirements
- A clear description of what should be tested
- A `todo!()` macro indicating the implementation work needed

This follows TDD practice: tests are written first to specify behavior, then implementation makes them pass.

## Test Strategy

### Plan Alignment

Every test is explicitly linked to a requirement in `VOICE_CLIENT_PLAN.md`. Comments reference:

- Specific plan sections
- Quoted text from the plan
- Event flow step numbers
- Interaction examples

### Coverage Approach

1. **Core functionality**: Happy path tests for all major features
2. **Edge cases**: Empty inputs, very large inputs, malformed data
3. **Error handling**: All specified error scenarios from plan
4. **Concurrency**: Multi-threaded access where required
5. **Performance**: Latency and throughput where specified
6. **State management**: Follow mode, pointer navigation, history immutability

### Testing Principles

1. **One logical assertion per test**: Tests are focused and easy to debug
2. **Descriptive names**: Test names clearly state what is being verified
3. **Arrange-Act-Assert structure**: Tests follow clear phases
4. **No test interdependencies**: Tests can run in any order
5. **Deterministic**: No flaky tests (mocked external dependencies)

## Event-to-Focus Mapping Test Coverage

The Focus Module includes comprehensive tests for all mapping rules specified in the plan:

| Tool Event           | Focus Entry       | Test Function                                |
| -------------------- | ----------------- | -------------------------------------------- |
| `read`               | `File(path)`      | `test_map_read_tool_to_file_focus`           |
| `write`              | `File(path)`      | `test_map_write_tool_to_file_focus`          |
| `edit`               | `File(filePath)`  | `test_map_edit_tool_to_file_focus`           |
| `bash: cd`           | `Directory(path)` | `test_map_bash_cd_to_directory_focus`        |
| `bash: git checkout` | `Branch(name)`    | `test_map_bash_git_checkout_to_branch_focus` |
| `bash: git commit`   | `Commit(hash)`    | `test_map_bash_git_commit_to_commit_focus`   |
| `list`               | `Directory(path)` | `test_map_list_tool_to_directory_focus`      |
| `grep`               | no-op             | `test_map_grep_tool_no_op`                   |

## FocusEntry Enum Test Coverage

Complete coverage of the four FocusEntry variants:

```rust
enum FocusEntry {
    File(PathBuf),      // ✓ 6 tests
    Directory(PathBuf), // ✓ 6 tests
    Branch(String),     // ✓ 6 tests
    Commit(String),     // ✓ 6 tests
}
```

Each variant is tested for:

- Creation
- Equality comparison
- Type discrimination (pattern matching)
- Display formatting with icon
- Serialization for context JSON
- Integration with history and pointer

## Full Utterance Flow Coverage

The integration tests include a comprehensive test for the complete 11-step utterance flow from the plan:

1. User holds push-to-talk key → audio starts flowing
2. Ring buffer feeds FFT → spectrogram updates
3. Ring buffer accumulates PCM for Whisper
4. User releases key → audio stops
5. Whisper processes buffer → transcript ready
6. Transcript displayed in TUI
7. Client retrieves focus pointer context
8. Sends transcript + context to OpenCode
9. Client receives events via SSE
10. Focus module processes event, appends to history
11. Focus stack display updates

Test: `test_complete_utterance_flow` in `src/integration_tests.rs`

## Interaction Example Coverage

All interaction examples from the plan have corresponding tests:

1. **"go to the src folder"** → `test_interaction_go_to_folder`
2. **"show me what's in here"** → `test_interaction_show_me_whats_here`
3. **[arrow key navigation + "open it"]** → `test_interaction_navigate_history_and_open`

## Next Steps for Implementation

To implement the code and make tests pass:

1. **Start with Focus Module**: It's the most well-defined and has no external dependencies (pure rules engine)
2. **Add Audio Module**: Ring buffer implementation with cpal integration
3. **Add STT Module**: Whisper-rs wrapper with error handling
4. **Add Viz Module**: FFT computation and character mapping
5. **Add Transport Module**: HTTP/SSE client for OpenCode
6. **Wire together in main.rs**: TUI loop with all modules integrated

Each module can be implemented incrementally, running `cargo test` after each function to watch tests turn from `todo!()` to passing.

## Dependencies Needed

Based on the tests, the following dependencies will be needed in `Cargo.toml`:

```toml
[dependencies]
cpal = "0.15"           # Audio capture
rustfft = "6.1"         # FFT for visualization
whisper-rs = "0.10"     # Whisper.cpp bindings
ratatui = "0.27"        # TUI framework
crossterm = "0.27"      # Terminal backend
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
ringbuf = "0.3"         # Ring buffer (or custom implementation)

[dev-dependencies]
rand = "0.8"            # For test fixtures (noise generation)
```

## Test-Driven Development Workflow

1. Read a test to understand the requirement
2. Implement the minimal code to make that test pass
3. Refactor if needed
4. Move to the next test
5. Repeat until all tests pass

The tests serve as:

- **Specification**: Clear expected behavior
- **Verification**: Confirm correctness as you code
- **Documentation**: Executable examples of how to use each module

## Coverage Philosophy

These tests prioritize **behavior verification** over **implementation details**:

- Tests describe **what** the code should do, not **how** it does it
- This allows implementation flexibility while ensuring correctness
- Tests should remain stable even if internal implementation changes

Example: The ring buffer tests verify read/write behavior without specifying whether it uses a Vec, a custom circular buffer, or the `ringbuf` crate.

## Performance Test Notes

Some tests are conditional on release builds:

```rust
#[test]
#[cfg(not(debug_assertions))]
fn test_transcription_latency() {
    // Only runs in cargo test --release
}
```

This prevents false failures due to debug build overhead.

## Mock Strategy

Tests use mocks for external dependencies:

- **MockAudioDevice**: Provides test audio instead of real microphone
- **MockOpenCodeServer**: Simulates OpenCode responses without network
- **Test fixtures**: Pre-generated audio samples, SSE events, focus contexts

This makes tests:

- Fast (no network I/O, no real audio processing)
- Deterministic (no flaky failures)
- Runnable without OpenCode installed

## Conclusion

This test suite provides comprehensive coverage of all functionality described in the Voice Client plan. By implementing the code to pass these tests, you will have a fully functional voice client that:

- Captures and visualizes audio
- Transcribes speech locally via Whisper
- Maintains a focus stack derived from OpenCode events
- Communicates with OpenCode server via HTTP/SSE
- Supports both voice and keyboard input modalities
- Handles errors gracefully with good UX

The tests serve as both specification and verification, ensuring the implementation matches the plan exactly.
