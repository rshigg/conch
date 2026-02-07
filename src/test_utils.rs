// Test utilities and helpers for module tests

#[cfg(test)]
pub mod fixtures {
    //! Test fixtures for common test data

    use std::path::PathBuf;

    /// Generate a sample audio buffer with a sine wave at known frequency
    pub fn generate_sine_wave(frequency: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let num_samples = (sample_rate * duration_secs) as usize;
        (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * frequency * t).sin()
            })
            .collect()
    }

    /// Generate silent audio buffer
    pub fn generate_silence(sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        vec![0.0; (sample_rate * duration_secs) as usize]
    }

    /// Generate white noise audio buffer
    pub fn generate_white_noise(sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let num_samples = (sample_rate * duration_secs) as usize;
        (0..num_samples).map(|_| rng.gen_range(-1.0..1.0)).collect()
    }

    /// Sample OpenCode tool.execute event for read tool
    pub fn sample_tool_event_read(path: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "tool.execute",
            "tool": "read",
            "args": {
                "path": path
            }
        })
    }

    /// Sample OpenCode tool.execute event for write tool
    pub fn sample_tool_event_write(path: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "tool.execute",
            "tool": "write",
            "args": {
                "path": path,
                "content": "sample content"
            }
        })
    }

    /// Sample OpenCode tool.execute event for bash tool with cd
    pub fn sample_tool_event_bash_cd(directory: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "tool.execute",
            "tool": "bash",
            "args": {
                "command": format!("cd {}", directory)
            }
        })
    }

    /// Sample OpenCode tool.execute event for bash tool with git checkout
    pub fn sample_tool_event_bash_git_checkout(branch: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "tool.execute",
            "tool": "bash",
            "args": {
                "command": format!("git checkout {}", branch)
            }
        })
    }

    /// Sample OpenCode tool.execute event for list tool
    pub fn sample_tool_event_list(path: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "tool.execute",
            "tool": "list",
            "args": {
                "path": path
            }
        })
    }

    /// Sample focus context JSON for prompts
    pub fn sample_focus_context_json() -> serde_json::Value {
        serde_json::json!({
            "current_focus": {
                "type": "file",
                "path": "src/App.tsx"
            },
            "recent_focus": {
                "file": "src/App.tsx",
                "directory": "src/",
                "branch": "main"
            }
        })
    }

    /// Sample prompt JSON with context
    pub fn sample_prompt_with_context(prompt_text: &str) -> serde_json::Value {
        serde_json::json!({
            "prompt": prompt_text,
            "context": sample_focus_context_json()
        })
    }
}

#[cfg(test)]
pub mod mocks {
    //! Mock implementations for testing

    /// Mock audio device that provides test audio
    pub struct MockAudioDevice {
        samples: Vec<f32>,
        position: usize,
    }

    impl MockAudioDevice {
        pub fn new(samples: Vec<f32>) -> Self {
            Self {
                samples,
                position: 0,
            }
        }

        /// Read next chunk of samples
        pub fn read(&mut self, chunk_size: usize) -> Vec<f32> {
            let end = std::cmp::min(self.position + chunk_size, self.samples.len());
            let chunk = self.samples[self.position..end].to_vec();
            self.position = end;
            chunk
        }

        pub fn reset(&mut self) {
            self.position = 0;
        }
    }

    /// Mock OpenCode server for testing transport
    pub struct MockOpenCodeServer {
        pub received_prompts: Vec<serde_json::Value>,
        pub events_to_send: Vec<serde_json::Value>,
    }

    impl MockOpenCodeServer {
        pub fn new() -> Self {
            Self {
                received_prompts: Vec::new(),
                events_to_send: Vec::new(),
            }
        }

        pub fn add_event(&mut self, event: serde_json::Value) {
            self.events_to_send.push(event);
        }

        pub fn receive_prompt(&mut self, prompt: serde_json::Value) {
            self.received_prompts.push(prompt);
        }
    }
}

#[cfg(test)]
pub mod assertions {
    //! Custom assertions for testing

    /// Assert two float slices are approximately equal within epsilon
    pub fn assert_float_slice_approx_eq(actual: &[f32], expected: &[f32], epsilon: f32) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "Slices have different lengths"
        );
        for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() < epsilon,
                "Values differ at index {}: {} vs {} (epsilon: {})",
                i,
                a,
                e,
                epsilon
            );
        }
    }

    /// Assert FFT magnitudes are in expected range
    pub fn assert_fft_magnitudes_valid(magnitudes: &[f32]) {
        for (i, mag) in magnitudes.iter().enumerate() {
            assert!(
                mag.is_finite() && *mag >= 0.0,
                "Invalid magnitude at index {}: {}",
                i,
                mag
            );
        }
    }

    /// Assert path is relative (not absolute)
    pub fn assert_path_relative(path: &std::path::Path) {
        assert!(
            path.is_relative(),
            "Expected relative path, got: {:?}",
            path
        );
    }

    /// Assert path is absolute
    pub fn assert_path_absolute(path: &std::path::Path) {
        assert!(
            path.is_absolute(),
            "Expected absolute path, got: {:?}",
            path
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_sine_wave() {
        // Test: Sine wave generator produces expected frequency
        let samples = fixtures::generate_sine_wave(440.0, 16000.0, 0.1);
        assert_eq!(samples.len(), 1600); // 0.1 sec * 16000 Hz
        // Verify it's not silence
        assert!(samples.iter().any(|&s| s.abs() > 0.1));
    }

    #[test]
    fn test_generate_silence() {
        // Test: Silence generator produces zeros
        let samples = fixtures::generate_silence(16000.0, 0.1);
        assert!(samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_mock_audio_device() {
        // Test: Mock audio device reads samples correctly
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let mut device = mocks::MockAudioDevice::new(samples.clone());

        let chunk1 = device.read(2);
        assert_eq!(chunk1, vec![1.0, 2.0]);

        let chunk2 = device.read(2);
        assert_eq!(chunk2, vec![3.0, 4.0]);

        device.reset();
        let chunk3 = device.read(2);
        assert_eq!(chunk3, vec![1.0, 2.0]);
    }

    #[test]
    fn test_sample_tool_events() {
        // Test: Sample tool events have correct structure
        let read_event = fixtures::sample_tool_event_read("src/main.rs");
        assert_eq!(read_event["tool"], "read");
        assert_eq!(read_event["args"]["path"], "src/main.rs");

        let bash_event = fixtures::sample_tool_event_bash_cd("src");
        assert_eq!(bash_event["tool"], "bash");
        assert!(bash_event["args"]["command"].as_str().unwrap().contains("cd"));
    }

    #[test]
    fn test_float_slice_approx_eq() {
        // Test: Float slice comparison assertion works
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0001, 2.0001, 3.0001];
        assertions::assert_float_slice_approx_eq(&a, &b, 0.001);
    }

    #[test]
    #[should_panic(expected = "Values differ")]
    fn test_float_slice_approx_eq_fails() {
        // Test: Float slice comparison fails when difference exceeds epsilon
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.1, 3.0];
        assertions::assert_float_slice_approx_eq(&a, &b, 0.01);
    }
}
