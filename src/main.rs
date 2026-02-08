// Conch — Voice input client for OpenCode
//
// Phase 1: Audio capture + Whisper STT + Minimal TUI
// Phase 2: Spectrogram visualization
// Phase 3: OpenCode transport (HTTP/SSE)

mod audio;
mod focus;
mod stt;
mod transport;
mod viz;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod test_utils;

use std::fs::OpenOptions;
use std::io::{self, Stdout, Write as _};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use audio::{AudioCapture, RecordingState};
use stt::Transcriber;
use transport::{
    ConnectionStatus, OpenCodeClient, ServerEvent, extract_sse_data_lines, parse_sse_event,
};
use viz::{WaveformData, WaveformWidget};

/// Noise floor threshold for RMS normalization.
const NOISE_FLOOR: f32 = 0.001;
/// OpenCode server base URL.
const OPENCODE_URL: &str = "http://127.0.0.1:4096";

/// Application state for the TUI.
struct App {
    /// Current recording state.
    state: RecordingState,
    /// History of transcripts (newest last).
    transcripts: Vec<String>,
    /// Error message to display, if any.
    error: Option<String>,
    /// Whether we're waiting for a background transcription.
    pending_transcript: bool,
    /// Waveform amplitudes for current frame, one per display column.
    waveform_bars: Vec<f32>,
    /// Transcript pending user confirmation before sending to OpenCode.
    prompt_pending: Option<String>,
    /// OpenCode connection status.
    connection_status: ConnectionStatus,
    /// Current session slug for display.
    session_slug: Option<String>,
    /// Whether OpenCode is currently busy processing.
    opencode_busy: bool,
}

impl App {
    fn new(_sample_rate: u32) -> Self {
        Self {
            state: RecordingState::Idle,
            transcripts: Vec::new(),
            error: None,
            pending_transcript: false,
            waveform_bars: Vec::new(),
            prompt_pending: None,
            connection_status: ConnectionStatus::Disconnected,
            session_slug: None,
            opencode_busy: false,
        }
    }
}

/// Messages sent from background tasks to the main TUI loop.
enum AppMessage {
    TranscriptReady(Result<String>),
    ServerEvent(ServerEvent),
    PromptSent(Result<()>),
    SessionReady { _id: String, slug: Option<String> },
    ConnectionChanged(ConnectionStatus),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let args: Vec<String> = std::env::args().collect();
    let model_path = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("ggml-base.en.bin");

    // Check for --session flag
    let session_flag = args.windows(2).find_map(|w| {
        if w[0] == "--session" {
            Some(w[1].clone())
        } else {
            None
        }
    });

    // Load Whisper model
    eprintln!("Loading Whisper model from '{}'...", model_path);
    let transcriber = match Transcriber::new(model_path) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!();
            eprintln!("To use Conch, you need a Whisper model file.");
            eprintln!("Download one with:");
            eprintln!(
                "  curl -L -o ggml-base.en.bin \
                 https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
            );
            eprintln!();
            eprintln!("Then run: conch ggml-base.en.bin");
            return Err(e);
        }
    };

    // Initialize audio capture
    let audio = AudioCapture::new()?;
    eprintln!(
        "Audio device ready ({}Hz). Starting TUI...",
        audio.sample_rate()
    );

    // Set up terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app, ensuring we restore the terminal on exit
    let result = run_app(&mut terminal, &audio, &transcriber, session_flag).await;

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop.
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    audio: &AudioCapture,
    transcriber: &Arc<Transcriber>,
    session_flag: Option<String>,
) -> Result<()> {
    let mut app = App::new(audio.sample_rate());

    // Channel for all messages to the TUI
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppMessage>();

    // Start OpenCode connection in background
    let tx_oc = tx.clone();
    let session_flag_clone = session_flag.clone();
    tokio::spawn(async move {
        connect_opencode(tx_oc, session_flag_clone).await;
    });

    loop {
        // Drain all pending messages (non-blocking)
        while let Ok(msg) = rx.try_recv() {
            match msg {
                AppMessage::TranscriptReady(result) => {
                    app.pending_transcript = false;
                    match result {
                        Ok(text) if !text.is_empty() => {
                            app.transcripts.push(text.clone());
                            app.prompt_pending = Some(text);
                            app.error = None;
                        }
                        Ok(_) => {
                            app.error = Some("No speech detected".into());
                        }
                        Err(e) => {
                            app.error = Some(format!("Transcription error: {}", e));
                        }
                    }
                    app.state = RecordingState::Idle;
                }
                AppMessage::ServerEvent(event) => match event {
                    ServerEvent::Connected => {
                        log("tui: SSE connected event");
                        app.connection_status = ConnectionStatus::Connected;
                    }
                    ServerEvent::SessionStatus { session_id, busy } => {
                        log(&format!(
                            "tui: session {} status: {}",
                            session_id,
                            if busy { "busy" } else { "idle" }
                        ));
                        app.opencode_busy = busy;
                    }
                    ServerEvent::Tool(ref te) => {
                        log(&format!(
                            "tui: tool event: {} (state: {})",
                            te.tool, te.state
                        ));
                        // TODO: Phase 4 — forward to focus module
                    }
                    ServerEvent::Heartbeat => {}
                },
                AppMessage::PromptSent(result) => {
                    match &result {
                        Ok(()) => log("tui: prompt sent successfully"),
                        Err(e) => log(&format!("tui: prompt send failed: {e}")),
                    }
                    if let Err(e) = result {
                        app.error = Some(format!("Send failed: {}", e));
                    }
                }
                AppMessage::SessionReady { slug, .. } => {
                    app.session_slug = slug;
                }
                AppMessage::ConnectionChanged(status) => {
                    app.connection_status = status;
                }
            }
        }

        // Snapshot waveform from ring buffer each frame
        let num_columns = terminal.size()?.width as usize;
        if app.state == RecordingState::Recording {
            // Read ~100ms of recent audio for the snapshot
            let snapshot_samples = audio.sample_rate() as usize / 10;
            let samples = audio.read_last_samples(snapshot_samples);
            if !samples.is_empty() {
                let rms = viz::compute_rms_windows(&samples, num_columns);
                app.waveform_bars = rms
                    .into_iter()
                    .map(|v| {
                        // Boost: divide by a low reference so moderate speech fills the display
                        let boosted = (v / 0.04).clamp(0.0, 1.0);
                        if boosted < NOISE_FLOOR { 0.0 } else { boosted }
                    })
                    .collect();
            }
        } else if !app.waveform_bars.is_empty() {
            app.waveform_bars.clear();
        }

        // Draw UI
        terminal.draw(|f| render(f, &app))?;

        // Poll for keyboard events
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => {
                        if app.prompt_pending.is_none() {
                            handle_space(&mut app, audio, transcriber, &tx)?;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(text) = app.prompt_pending.take() {
                            app.error = None;
                            send_prompt_to_opencode(&text, &tx);
                        }
                    }
                    KeyCode::Backspace | KeyCode::Delete => {
                        if app.prompt_pending.take().is_some() {
                            app.error = Some("Prompt discarded".into());
                        }
                    }
                    KeyCode::Char('c')
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Write a timestamped line to conch.log for debugging.
fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("conch.log")
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let _ = writeln!(f, "[{:.3}] {}", now.as_secs_f64(), msg);
    }
}

/// Shared state for the OpenCode client, accessible from the send path.
static OPENCODE_SESSION_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn send_prompt_to_opencode(text: &str, tx: &tokio::sync::mpsc::UnboundedSender<AppMessage>) {
    let text = text.to_string();
    let tx = tx.clone();
    log(&format!(
        "send_prompt: queuing prompt ({} chars)",
        text.len()
    ));
    tokio::spawn(async move {
        let session_id = OPENCODE_SESSION_ID.lock().unwrap().clone();
        let Some(session_id) = session_id else {
            log("send_prompt: ERROR no session ID set");
            let _ = tx.send(AppMessage::PromptSent(Err(anyhow!("no session"))));
            return;
        };
        log(&format!("send_prompt: sending to session {session_id}"));
        let mut client = OpenCodeClient::new(OPENCODE_URL);
        client.set_session(session_id);
        let result = client.send_prompt(&text).await;
        match &result {
            Ok(()) => log("send_prompt: success"),
            Err(e) => log(&format!("send_prompt: ERROR {e}")),
        }
        let _ = tx.send(AppMessage::PromptSent(result));
    });
}

/// Background task: connect to OpenCode, establish session, listen for SSE events.
async fn connect_opencode(
    tx: tokio::sync::mpsc::UnboundedSender<AppMessage>,
    session_flag: Option<String>,
) {
    let mut client = OpenCodeClient::new(OPENCODE_URL);

    // Health check with retry
    log("connect_opencode: starting health check loop");
    loop {
        match client.health_check().await {
            Ok(true) => {
                log("connect_opencode: health check passed");
                break;
            }
            Ok(false) => {
                log("connect_opencode: health check returned false, retrying...");
                let _ = tx.send(AppMessage::ConnectionChanged(
                    ConnectionStatus::Reconnecting,
                ));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log(&format!(
                    "connect_opencode: health check error: {e}, retrying..."
                ));
                let _ = tx.send(AppMessage::ConnectionChanged(
                    ConnectionStatus::Reconnecting,
                ));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }

    // Session management
    log("connect_opencode: resolving session");
    let session_id = if let Some(id) = session_flag {
        log(&format!("connect_opencode: using --session flag: {id}"));
        client.set_session(id.clone());
        id
    } else {
        // Try to reuse existing session for this directory
        match client.list_sessions().await {
            Ok(sessions) => {
                log(&format!(
                    "connect_opencode: found {} sessions",
                    sessions.len()
                ));
                let cwd = std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                let existing = sessions
                    .iter()
                    .find(|s| s.directory.as_ref() == cwd.as_ref());
                if let Some(s) = existing {
                    log(&format!(
                        "connect_opencode: reusing session {} ({})",
                        s.id,
                        s.slug.as_deref().unwrap_or("?")
                    ));
                    client.set_session(s.id.clone());
                    let _ = tx.send(AppMessage::SessionReady {
                        _id: s.id.clone(),
                        slug: s.slug.clone(),
                    });
                    s.id.clone()
                } else {
                    match client.create_session().await {
                        Ok(id) => {
                            let _ = tx.send(AppMessage::SessionReady {
                                _id: id.clone(),
                                slug: None,
                            });
                            id
                        }
                        Err(e) => {
                            let _ = tx.send(AppMessage::ConnectionChanged(
                                ConnectionStatus::Disconnected,
                            ));
                            eprintln!("Failed to create session: {}", e);
                            return;
                        }
                    }
                }
            }
            Err(_) => match client.create_session().await {
                Ok(id) => {
                    let _ = tx.send(AppMessage::SessionReady {
                        _id: id.clone(),
                        slug: None,
                    });
                    id
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::ConnectionChanged(
                        ConnectionStatus::Disconnected,
                    ));
                    eprintln!("Failed to create session: {}", e);
                    return;
                }
            },
        }
    };

    // Store session ID for the prompt sender
    log(&format!(
        "connect_opencode: session ready, storing ID for prompt sender"
    ));
    *OPENCODE_SESSION_ID.lock().unwrap() = Some(session_id);

    // SSE event loop with reconnection
    log("connect_opencode: entering SSE loop");
    loop {
        match client.subscribe_events().await {
            Ok(resp) => {
                log("connect_opencode: SSE connected");
                let _ = tx.send(AppMessage::ConnectionChanged(ConnectionStatus::Connected));
                if let Err(e) = stream_sse_events(resp, &tx).await {
                    log(&format!("connect_opencode: SSE stream ended: {e}"));
                    let _ = tx.send(AppMessage::ConnectionChanged(
                        ConnectionStatus::Reconnecting,
                    ));
                }
            }
            Err(e) => {
                log(&format!("connect_opencode: SSE connect failed: {e}"));
                let _ = tx.send(AppMessage::ConnectionChanged(
                    ConnectionStatus::Reconnecting,
                ));
            }
        }
        log("connect_opencode: reconnecting in 2s...");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Read SSE events from a streaming response and forward them.
async fn stream_sse_events(
    mut resp: reqwest::Response,
    tx: &tokio::sync::mpsc::UnboundedSender<AppMessage>,
) -> Result<()> {
    let mut buf = String::new();

    // Use reqwest's chunk() method to read the streaming body piece by piece.
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(60), resp.chunk()).await;

        match chunk {
            Ok(Ok(Some(bytes))) => {
                buf.push_str(&String::from_utf8_lossy(&bytes));
                // Process complete SSE events (terminated by \n\n)
                while let Some(pos) = buf.find("\n\n") {
                    let event_text = buf[..pos].to_string();
                    buf = buf[pos + 2..].to_string();

                    for line in extract_sse_data_lines(&event_text) {
                        if let Some(event) = parse_sse_event(line) {
                            let _ = tx.send(AppMessage::ServerEvent(event));
                        }
                    }
                }
            }
            Ok(Ok(None)) => return Err(anyhow!("stream ended")),
            Ok(Err(e)) => return Err(anyhow!("stream error: {}", e)),
            Err(_) => return Err(anyhow!("stream timeout")),
        }
    }
}

/// Handle spacebar press: toggle between recording and stopping.
fn handle_space(
    app: &mut App,
    audio: &AudioCapture,
    transcriber: &Arc<Transcriber>,
    tx: &tokio::sync::mpsc::UnboundedSender<AppMessage>,
) -> Result<()> {
    match app.state {
        RecordingState::Idle => {
            audio.start_recording();
            app.state = RecordingState::Recording;
            app.error = None;
        }
        RecordingState::Recording => {
            let samples = audio.stop_recording();
            let sample_rate = audio.sample_rate();

            if samples.is_empty() {
                app.error = Some("No audio captured".into());
                app.state = RecordingState::Idle;
                return Ok(());
            }

            app.state = RecordingState::Processing;
            app.pending_transcript = true;

            // Run transcription in background thread
            let tx = tx.clone();
            let transcriber = Arc::clone(transcriber);
            std::thread::spawn(move || {
                let result = transcriber.transcribe(&samples, sample_rate);
                let _ = tx.send(AppMessage::TranscriptReady(result));
            });
        }
        RecordingState::Processing => {
            // Ignore space while processing
        }
    }
    Ok(())
}

/// Render the TUI.
fn render(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Length(10), // Waveform (8 content rows = 32 braille dots tall)
            Constraint::Length(3),  // Status
            Constraint::Min(6),     // Transcripts
            Constraint::Length(3),  // Help bar
        ])
        .split(area);

    // Title bar with connection status
    let conn_indicator = match app.connection_status {
        ConnectionStatus::Connected => {
            if app.opencode_busy {
                Span::styled(" [OC: busy] ", Style::default().fg(Color::Yellow))
            } else {
                Span::styled(" [OC: connected] ", Style::default().fg(Color::Green))
            }
        }
        ConnectionStatus::Disconnected => {
            Span::styled(" [OC: disconnected] ", Style::default().fg(Color::Red))
        }
        ConnectionStatus::Reconnecting => {
            Span::styled(" [OC: reconnecting] ", Style::default().fg(Color::Yellow))
        }
    };
    let session_info = app
        .session_slug
        .as_ref()
        .map(|s| format!(" ({}) ", s))
        .unwrap_or_default();

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "Conch Voice Client",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(session_info, Style::default().fg(Color::DarkGray)),
        conn_indicator,
    ]))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    // Waveform

    let waveform_data = WaveformData {
        bars: app.waveform_bars.clone(),
    };
    let wave_block = Block::default();
    let wave_inner = wave_block.inner(chunks[1]);
    f.render_widget(wave_block, chunks[1]);
    let wave_widget = WaveformWidget::new(&waveform_data);
    f.render_widget(wave_widget, wave_inner);

    // Status area
    let (status_text, status_color) = if app.prompt_pending.is_some() {
        (
            "  Press [Enter] to send to OpenCode, [Backspace] to discard".into(),
            Color::Cyan,
        )
    } else {
        match app.state {
            RecordingState::Idle => {
                if let Some(err) = &app.error {
                    (format!("  {}", err), Color::Yellow)
                } else {
                    ("  Ready".into(), Color::Gray)
                }
            }
            RecordingState::Recording => {
                ("  ● Recording... press [Space] to stop".into(), Color::Red)
            }
            RecordingState::Processing => ("  ⏳ Transcribing...".into(), Color::Yellow),
        }
    };
    let status = Paragraph::new(status_text)
        .style(Style::default().fg(status_color))
        .block(Block::default().title(" Status ").borders(Borders::ALL));
    f.render_widget(status, chunks[2]);

    // Transcript area
    let transcript_lines: Vec<Line> = if app.transcripts.is_empty() {
        vec![Line::from(Span::styled(
            "  No transcripts yet",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.transcripts
            .iter()
            .enumerate()
            .rev()
            .take(50)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|(i, t)| {
                let is_pending = app
                    .prompt_pending
                    .as_ref()
                    .map(|p| p == t && i == app.transcripts.len() - 1)
                    .unwrap_or(false);
                let style = if is_pending {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(vec![
                    Span::styled(
                        format!("  {}. ", i + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(t.clone(), style),
                    if is_pending {
                        Span::styled(" [pending]", Style::default().fg(Color::DarkGray))
                    } else {
                        Span::raw("")
                    },
                ])
            })
            .collect()
    };
    let transcripts = Paragraph::new(transcript_lines)
        .block(
            Block::default()
                .title(" Transcripts ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(transcripts, chunks[3]);

    // Help bar
    let mut help_spans = vec![
        Span::styled(" [Space] ", Style::default().fg(Color::Cyan)),
        Span::raw("Record  "),
    ];
    if app.prompt_pending.is_some() {
        help_spans.extend([
            Span::styled("[Enter] ", Style::default().fg(Color::Cyan)),
            Span::raw("Send  "),
            Span::styled("[Bksp] ", Style::default().fg(Color::Cyan)),
            Span::raw("Discard  "),
        ]);
    }
    help_spans.extend([
        Span::styled("[q/Esc] ", Style::default().fg(Color::Cyan)),
        Span::raw("Quit"),
    ]);
    let help = Paragraph::new(Line::from(help_spans)).block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[4]);
}
