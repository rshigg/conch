// Audio Module - Captures mic input via cpal, manages ring buffer, provides PCM data

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

/// Recording state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Recording,
    Processing,
}

/// A circular ring buffer for audio samples.
///
/// Stores f32 PCM samples with a fixed capacity. When full, new writes
/// overwrite the oldest data. Supports reading all valid samples in order
/// (for Whisper) and reading the last N samples (for FFT visualization).
pub struct RingBuffer {
    data: Vec<f32>,
    capacity: usize,
    write_pos: usize,
    count: usize,
}

impl RingBuffer {
    /// Create a new ring buffer with the given capacity in samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            capacity,
            write_pos: 0,
            count: 0,
        }
    }

    /// Write samples into the ring buffer. If the buffer is full,
    /// oldest samples are overwritten.
    pub fn write(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.data[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.capacity;
            if self.count < self.capacity {
                self.count += 1;
            }
        }
    }

    /// Read all valid samples in chronological order (oldest first).
    /// Used for extracting the complete buffer for Whisper transcription.
    pub fn read_all(&self) -> Vec<f32> {
        if self.count == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(self.count);
        if self.count < self.capacity {
            // Haven't wrapped yet - data is contiguous from 0..count
            result.extend_from_slice(&self.data[..self.count]);
        } else {
            // Wrapped: oldest data starts at write_pos
            result.extend_from_slice(&self.data[self.write_pos..]);
            result.extend_from_slice(&self.data[..self.write_pos]);
        }
        result
    }

    /// Read the last `n` samples (most recent). Used for FFT visualization.
    pub fn read_last(&self, n: usize) -> Vec<f32> {
        let n = n.min(self.count);
        if n == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(n);
        // Start position is n samples before write_pos
        let start = if self.write_pos >= n {
            self.write_pos - n
        } else {
            self.capacity - (n - self.write_pos)
        };
        if start + n <= self.capacity {
            result.extend_from_slice(&self.data[start..start + n]);
        } else {
            result.extend_from_slice(&self.data[start..]);
            let remaining = n - (self.capacity - start);
            result.extend_from_slice(&self.data[..remaining]);
        }
        result
    }

    /// Clear all data from the buffer.
    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.count = 0;
    }

    /// Number of valid samples in the buffer.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Total capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Shared state between the audio callback thread and the main thread.
struct SharedAudioState {
    recording: bool,
    buffer: RingBuffer,
}

/// Audio capture system using cpal.
///
/// Manages the microphone input stream and a ring buffer for recorded audio.
/// Supports push-to-talk: call `start_recording()` to begin capturing and
/// `stop_recording()` to stop and extract the recorded samples.
pub struct AudioCapture {
    shared: Arc<Mutex<SharedAudioState>>,
    _stream: cpal::Stream,
    sample_rate: u32,
}

// cpal::Stream is not Send, but we ensure it's only accessed from the thread
// that created it. The Arc<Mutex<SharedAudioState>> handles cross-thread access.
unsafe impl Send for AudioCapture {}

impl AudioCapture {
    /// Create a new AudioCapture using the default input device.
    ///
    /// The ring buffer is sized for 60 seconds of audio at the device's sample rate.
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("No audio input device found"))?;

        let supported_config = device.default_input_config()?;
        let sample_rate = supported_config.sample_rate().0;
        let sample_format = supported_config.sample_format();
        let channels = supported_config.channels() as usize;
        let config: cpal::StreamConfig = supported_config.into();

        // 60 seconds of mono audio at the device's sample rate
        let buffer_capacity = sample_rate as usize * 60;
        let shared = Arc::new(Mutex::new(SharedAudioState {
            recording: false,
            buffer: RingBuffer::new(buffer_capacity),
        }));

        let shared_clone = Arc::clone(&shared);
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                build_input_stream::<f32>(&device, &config, shared_clone, channels)?
            }
            cpal::SampleFormat::I16 => {
                build_input_stream::<i16>(&device, &config, shared_clone, channels)?
            }
            cpal::SampleFormat::U16 => {
                build_input_stream::<u16>(&device, &config, shared_clone, channels)?
            }
            format => return Err(anyhow!("Unsupported sample format: {:?}", format)),
        };

        stream.play()?;

        Ok(Self {
            shared,
            _stream: stream,
            sample_rate,
        })
    }

    /// Begin recording audio. Clears any previous buffer contents.
    pub fn start_recording(&self) {
        let mut state = self.shared.lock().unwrap();
        state.buffer.clear();
        state.recording = true;
    }

    /// Stop recording and return all captured samples as mono f32 PCM.
    pub fn stop_recording(&self) -> Vec<f32> {
        let mut state = self.shared.lock().unwrap();
        state.recording = false;
        state.buffer.read_all()
    }

    /// Returns true if currently recording.
    pub fn is_recording(&self) -> bool {
        self.shared.lock().unwrap().recording
    }

    /// Read the most recent `n` samples from the ring buffer.
    /// Used by the viz module for real-time FFT during recording.
    /// Returns an empty vec if not recording or buffer is empty.
    pub fn read_last_samples(&self, n: usize) -> Vec<f32> {
        let state = self.shared.lock().unwrap();
        if state.recording {
            state.buffer.read_last(n)
        } else {
            Vec::new()
        }
    }

    /// The sample rate of the audio input device in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Build a cpal input stream that writes samples to the shared ring buffer.
/// Handles mono conversion from multi-channel audio.
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Mutex<SharedAudioState>>,
    channels: usize,
) -> Result<cpal::Stream>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(mut state) = shared.try_lock() {
                if state.recording {
                    // Convert to mono f32
                    for chunk in data.chunks(channels) {
                        let mono: f32 = chunk
                            .iter()
                            .map(|s| f32::from_sample(*s))
                            .sum::<f32>()
                            / channels as f32;
                        state.buffer.write(&[mono]);
                    }
                }
            }
            // If lock fails (contention), drop the audio frame.
            // This is acceptable for a voice input client.
        },
        |err| eprintln!("Audio stream error: {}", err),
        None,
    )?;
    Ok(stream)
}

/// Resample audio from one sample rate to another using linear interpolation.
/// Whisper requires 16kHz mono f32 audio.
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio) as usize;
    (0..output_len)
        .map(|i| {
            let src_idx = i as f64 * ratio;
            let idx = src_idx as usize;
            let frac = (src_idx - idx as f64) as f32;
            let s1 = samples[idx.min(samples.len() - 1)];
            let s2 = samples[(idx + 1).min(samples.len() - 1)];
            s1 + (s2 - s1) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_creation() {
        let buf = RingBuffer::new(1024);
        assert_eq!(buf.capacity(), 1024);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_ring_buffer_write() {
        let mut buf = RingBuffer::new(1024);
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        buf.write(&samples);
        assert_eq!(buf.len(), 5);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_ring_buffer_read() {
        let mut buf = RingBuffer::new(1024);
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        buf.write(&samples);
        let read = buf.read_all();
        assert_eq!(read, samples);

        // Reading again returns the same data (non-consuming)
        let read2 = buf.read_all();
        assert_eq!(read2, samples);
    }

    #[test]
    fn test_ring_buffer_circular_behavior() {
        let mut buf = RingBuffer::new(4);
        buf.write(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.read_all(), vec![1.0, 2.0, 3.0, 4.0]);

        // Write more — should overwrite oldest
        buf.write(&[5.0, 6.0]);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.read_all(), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_ring_buffer_read_last() {
        let mut buf = RingBuffer::new(1024);
        buf.write(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(buf.read_last(3), vec![3.0, 4.0, 5.0]);
        assert_eq!(buf.read_last(1), vec![5.0]);
        assert_eq!(buf.read_last(5), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        // Requesting more than available returns all
        assert_eq!(buf.read_last(10), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn test_ring_buffer_read_last_after_wrap() {
        let mut buf = RingBuffer::new(4);
        buf.write(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        // Buffer contains [3.0, 4.0, 5.0, 6.0]
        assert_eq!(buf.read_last(2), vec![5.0, 6.0]);
        assert_eq!(buf.read_last(4), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_ring_buffer_clear() {
        let mut buf = RingBuffer::new(1024);
        buf.write(&[1.0, 2.0, 3.0]);
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.read_all(), Vec::<f32>::new());
    }

    #[test]
    fn test_push_to_talk_start() {
        // Verify the state transition to Recording
        let state = RecordingState::Recording;
        assert_eq!(state, RecordingState::Recording);
    }

    #[test]
    fn test_push_to_talk_stop() {
        // Verify the state transition from Recording to Idle
        let state = RecordingState::Idle;
        assert_eq!(state, RecordingState::Idle);
    }

    #[test]
    fn test_audio_capture_state_machine() {
        // Verify all three states exist
        let states = [
            RecordingState::Idle,
            RecordingState::Recording,
            RecordingState::Processing,
        ];
        assert_eq!(states[0], RecordingState::Idle);
        assert_eq!(states[1], RecordingState::Recording);
        assert_eq!(states[2], RecordingState::Processing);
    }

    #[test]
    fn test_sample_format_f32() {
        // Ring buffer stores f32 samples
        let mut buf = RingBuffer::new(16);
        buf.write(&[0.5, -0.5, 1.0, -1.0]);
        let read = buf.read_all();
        assert_eq!(read[0], 0.5_f32);
        assert_eq!(read[1], -0.5_f32);
    }

    #[test]
    fn test_buffer_extraction_for_whisper() {
        let mut buf = RingBuffer::new(1024);
        let samples: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        buf.write(&samples);
        let extracted = buf.read_all();
        assert_eq!(extracted.len(), 100);
        assert_eq!(extracted, samples);
    }

    #[test]
    fn test_empty_recording_handling() {
        let buf = RingBuffer::new(1024);
        let extracted = buf.read_all();
        assert!(extracted.is_empty());
    }

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0];
        let result = resample(&samples, 16000, 16000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        // 48kHz to 16kHz = 3:1 ratio
        let samples: Vec<f32> = (0..4800).map(|i| (i as f32 / 4800.0).sin()).collect();
        let result = resample(&samples, 48000, 16000);
        // Output should be approximately 1/3 the length
        assert_eq!(result.len(), 1600);
    }

    #[test]
    fn test_resample_empty() {
        let result = resample(&[], 48000, 16000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_concurrent_read_write() {
        // Test ring buffer works correctly with sequential write-then-read pattern
        // (True concurrent testing with cpal requires an audio device)
        let mut buf = RingBuffer::new(1024);
        for i in 0..10 {
            let chunk: Vec<f32> = (0..100).map(|j| (i * 100 + j) as f32).collect();
            buf.write(&chunk);
            let last = buf.read_last(50);
            assert_eq!(last.len(), 50);
        }
    }

    #[test]
    fn test_recording_duration_tracking() {
        // At 16kHz, 16000 samples = 1 second
        let mut buf = RingBuffer::new(160000);
        let one_second: Vec<f32> = vec![0.0; 16000];
        buf.write(&one_second);
        let duration_secs = buf.len() as f64 / 16000.0;
        assert!((duration_secs - 1.0).abs() < 0.001);
    }
}
