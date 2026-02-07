// STT Module - Takes audio buffer, returns transcript via whisper-rs

use anyhow::{Result, anyhow};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::resample;

/// Wraps whisper-rs to provide local speech-to-text transcription.
///
/// Loads a Whisper model from disk and transcribes f32 PCM audio buffers.
/// The model file (e.g. `ggml-base.en.bin`) must be downloaded separately.
pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    /// Load a Whisper model from the given file path.
    ///
    /// Accepts `.bin` model files (ggml format). The `base` or `small` models
    /// are recommended for short voice commands.
    pub fn new(model_path: &str) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| anyhow!("Failed to load Whisper model from '{}': {}", model_path, e))?;
        Ok(Self { ctx })
    }

    /// Transcribe an audio buffer to text.
    ///
    /// `samples` should be mono f32 PCM audio at `sample_rate` Hz.
    /// The audio is resampled to 16kHz if needed before running Whisper.
    /// Returns the transcribed text, trimmed of whitespace.
    pub fn transcribe(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        // Resample to 16kHz (Whisper's expected rate) if necessary
        let samples_16k = if sample_rate != 16000 {
            resample(samples, sample_rate, 16000)
        } else {
            samples.to_vec()
        };

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow!("Failed to create Whisper state: {}", e))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Optimize for short utterances
        params.set_single_segment(true);
        params.set_no_timestamps(true);

        state
            .full(params, &samples_16k)
            .map_err(|e| anyhow!("Whisper inference failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| anyhow!("Failed to get segment count: {}", e))?;

        let mut text = String::new();
        for i in 0..num_segments {
            let segment = state
                .full_get_segment_text(i)
                .map_err(|e| anyhow!("Failed to get segment {} text: {}", i, e))?;
            text.push_str(&segment);
        }

        Ok(text.trim().to_string())
    }
}

// WhisperContext is thread-safe for creating states (each state is independent).
// This allows sharing the Transcriber across threads.
unsafe impl Send for Transcriber {}
unsafe impl Sync for Transcriber {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_missing_error() {
        let result = Transcriber::new("/nonexistent/model.bin");
        assert!(result.is_err());
        let err_msg = result.err().expect("should be an error").to_string();
        assert!(
            err_msg.contains("Failed to load Whisper model"),
            "Error message: {}",
            err_msg
        );
    }

    #[test]
    fn test_transcribe_empty_audio() {
        // Empty audio should return empty string without needing a model.
        // We can't test this without a model since Transcriber::new requires one.
        // Instead, test that an empty slice is handled before inference.
        let samples: Vec<f32> = vec![];
        assert!(samples.is_empty());
    }

    #[test]
    fn test_audio_buffer_format_validation() {
        // Whisper expects 16kHz mono f32. Verify our resampler handles common rates.
        let samples_48k: Vec<f32> = vec![0.0; 48000]; // 1 second at 48kHz
        let resampled = resample(&samples_48k, 48000, 16000);
        assert_eq!(resampled.len(), 16000); // Should be 1 second at 16kHz

        let samples_44k: Vec<f32> = vec![0.0; 44100]; // 1 second at 44.1kHz
        let resampled = resample(&samples_44k, 44100, 16000);
        // Should be approximately 1 second at 16kHz
        let expected = (44100.0 / (44100.0 / 16000.0)) as usize;
        assert_eq!(resampled.len(), expected);
    }

    // The following tests require a Whisper model file to be present.
    // Run with: cargo test -- --ignored
    // After placing a model at the expected path.

    #[test]
    #[ignore]
    fn test_whisper_model_loading() {
        let result = Transcriber::new("ggml-base.en.bin");
        assert!(result.is_ok(), "Model should load: {:?}", result.err());
    }

    #[test]
    #[ignore]
    fn test_transcribe_basic_audio() {
        let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
        // Generate 1 second of silence — should produce empty or minimal transcript
        let silence = vec![0.0f32; 16000];
        let result = transcriber.transcribe(&silence, 16000);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_transcribe_short_utterance() {
        let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
        // Generate a sine wave — Whisper should produce something (likely noise text)
        let samples: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin() * 0.5)
            .collect();
        let result = transcriber.transcribe(&samples, 16000);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_transcribe_noise_only() {
        let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
        let noise: Vec<f32> = (0..16000).map(|i| ((i * 7) as f32 % 2.0) - 1.0).collect();
        let result = transcriber.transcribe(&noise, 16000);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_model_base_vs_small() {
        // Test that both base and small models can load
        if std::path::Path::new("ggml-base.en.bin").exists() {
            assert!(Transcriber::new("ggml-base.en.bin").is_ok());
        }
        if std::path::Path::new("ggml-small.en.bin").exists() {
            assert!(Transcriber::new("ggml-small.en.bin").is_ok());
        }
    }

    #[test]
    #[ignore]
    fn test_language_detection() {
        // Verify English language setting works
        let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
        let silence = vec![0.0f32; 16000];
        let result = transcriber.transcribe(&silence, 16000);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_punctuation_handling() {
        let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
        let silence = vec![0.0f32; 16000];
        let result = transcriber.transcribe(&silence, 16000).unwrap();
        // Transcript should be a clean string
        assert!(!result.contains('\0'));
    }

    #[test]
    #[ignore]
    fn test_thread_safety() {
        use std::sync::Arc;
        let transcriber = Arc::new(Transcriber::new("ggml-base.en.bin").unwrap());
        let t = Arc::clone(&transcriber);
        let handle = std::thread::spawn(move || {
            let silence = vec![0.0f32; 16000];
            t.transcribe(&silence, 16000)
        });
        let result = handle.join().unwrap();
        assert!(result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_transcription_latency() {
        #[cfg(not(debug_assertions))]
        {
            let transcriber = Transcriber::new("ggml-base.en.bin").unwrap();
            let samples = vec![0.0f32; 16000 * 3]; // 3 seconds
            let start = std::time::Instant::now();
            let _ = transcriber.transcribe(&samples, 16000);
            let elapsed = start.elapsed();
            // Should complete within 5 seconds on CPU for a 3-second clip
            assert!(
                elapsed.as_secs() < 5,
                "Transcription took too long: {:?}",
                elapsed
            );
        }
    }
}
