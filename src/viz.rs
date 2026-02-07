// Viz Module - FFT via rustfft, renders spectrogram bar display in ratatui TUI
//
// Phase 2: Takes audio samples from the ring buffer, computes FFT,
// bins frequency magnitudes, and renders a bar-style visualization
// using half-block Unicode characters.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Half-block characters for 8 levels of vertical resolution (low to high).
const BAR_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Map a normalized magnitude (0.0..=1.0) to a half-block character.
pub fn magnitude_to_char(magnitude: f32) -> char {
    let clamped = magnitude.clamp(0.0, 1.0);
    if clamped == 0.0 {
        return ' ';
    }
    let idx = ((clamped * 8.0) as usize).min(7);
    BAR_CHARS[idx]
}

/// Apply a Hann window to a slice of samples in-place.
fn apply_hann_window(samples: &mut [f32]) {
    let n = samples.len() as f32;
    for (i, sample) in samples.iter_mut().enumerate() {
        let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n).cos());
        *sample *= w;
    }
}

/// Compute the magnitude spectrum from a chunk of audio samples.
///
/// Returns a vector of frequency bin magnitudes (only the positive frequencies,
/// i.e. the first half of the FFT output). The magnitudes are in linear scale.
pub fn compute_spectrum(samples: &[f32]) -> Vec<f32> {
    let fft_size = samples.len();
    if fft_size == 0 {
        return Vec::new();
    }

    // Apply Hann window
    let mut windowed: Vec<f32> = samples.to_vec();
    apply_hann_window(&mut windowed);

    // Convert to complex
    let mut input: Vec<Complex<f32>> = windowed
        .iter()
        .map(|&s| Complex { re: s, im: 0.0 })
        .collect();

    // Compute FFT
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    fft.process(&mut input);

    // Take magnitudes of positive frequencies only (first half)
    let half = fft_size / 2;
    input[..half]
        .iter()
        .map(|c| c.norm() / fft_size as f32)
        .collect()
}

/// Bin a spectrum into a fixed number of display bars using logarithmic frequency scaling.
///
/// Groups FFT bins into `num_bars` output bins, averaging the magnitudes within each group.
/// Uses log-frequency scaling so lower frequencies get more bars (matching human perception).
pub fn bin_spectrum(spectrum: &[f32], num_bars: usize) -> Vec<f32> {
    if spectrum.is_empty() || num_bars == 0 {
        return vec![0.0; num_bars];
    }
    if spectrum.len() <= num_bars {
        let mut result: Vec<f32> = spectrum.to_vec();
        result.resize(num_bars, 0.0);
        return result;
    }

    let n = spectrum.len() as f32;
    let mut bars = Vec::with_capacity(num_bars);

    for i in 0..num_bars {
        // Log-scale bin boundaries
        let lo = (n.powf(i as f32 / num_bars as f32)) as usize;
        let hi = (n.powf((i + 1) as f32 / num_bars as f32)) as usize;
        let lo = lo.min(spectrum.len());
        let hi = hi.max(lo + 1).min(spectrum.len());

        let sum: f32 = spectrum[lo..hi].iter().sum();
        let count = (hi - lo) as f32;
        bars.push(sum / count);
    }

    bars
}

/// Normalize magnitudes to 0.0..=1.0 range with a noise floor threshold.
///
/// Uses a fixed minimum reference level (`min_ref`) so that quiet signals
/// (like mic self-noise) don't get amplified to fill the display.
/// `noise_floor`: per-bin threshold below which bins are zeroed.
/// `min_ref`: minimum reference level for normalization — signals whose
///            peak is below this are shown as proportionally quiet rather
///            than being scaled to full range.
pub fn normalize_magnitudes(magnitudes: &[f32], noise_floor: f32, min_ref: f32) -> Vec<f32> {
    if magnitudes.is_empty() {
        return Vec::new();
    }

    let max = magnitudes.iter().cloned().fold(0.0_f32, f32::max);
    if max <= noise_floor {
        return vec![0.0; magnitudes.len()];
    }

    // Use the larger of the actual max and the minimum reference level.
    // This prevents quiet noise from being scaled up to fill the display.
    let ref_level = max.max(min_ref);

    magnitudes
        .iter()
        .map(|&m| {
            if m <= noise_floor {
                0.0
            } else {
                // Log scaling for perceptual loudness
                let db = 20.0 * (m / ref_level).log10();
                // Map -40dB..0dB to 0.0..1.0 (tighter range for more responsive display)
                ((db + 40.0) / 40.0).clamp(0.0, 1.0)
            }
        })
        .collect()
}

/// Holds the current spectrum data ready for rendering.
pub struct SpectrumData {
    /// Normalized bar magnitudes (0.0..=1.0), one per display column.
    pub bars: Vec<f32>,
}

impl SpectrumData {
    /// Create an empty spectrum (silence).
    pub fn empty() -> Self {
        Self { bars: Vec::new() }
    }

    /// Compute spectrum data from raw audio samples.
    ///
    /// `fft_size`: number of samples to use for FFT (e.g. 1024).
    /// `num_bars`: number of display bars to produce.
    /// `noise_floor`: magnitude threshold below which bins are zeroed.
    pub fn from_samples(samples: &[f32], fft_size: usize, num_bars: usize, noise_floor: f32) -> Self {
        if samples.len() < fft_size {
            return Self::empty();
        }

        // Use the most recent fft_size samples
        let chunk = &samples[samples.len() - fft_size..];

        // Check RMS level — if the audio is just mic self-noise, skip visualization
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        if rms < 0.005 {
            return Self { bars: vec![0.0; num_bars] };
        }

        let spectrum = compute_spectrum(chunk);
        let binned = bin_spectrum(&spectrum, num_bars);
        // Use a fixed minimum reference level (0.05) so that moderate speech
        // shows as mid-range bars and only loud speech goes red.
        let bars = normalize_magnitudes(&binned, noise_floor, 0.05);

        Self { bars }
    }
}

/// A ratatui widget that renders a bar-style audio visualization.
pub struct SpectrogramWidget<'a> {
    data: &'a SpectrumData,
    active: bool,
}

impl<'a> SpectrogramWidget<'a> {
    pub fn new(data: &'a SpectrumData, active: bool) -> Self {
        Self { data, active }
    }
}

impl Widget for SpectrogramWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 1 {
            return;
        }

        let width = area.width as usize;

        // If no data or not active, show flat line
        if self.data.bars.is_empty() || !self.active {
            let bar_str: String = std::iter::repeat(' ').take(width).collect();
            let style = Style::default().fg(Color::DarkGray);
            buf.set_string(area.x, area.y + area.height / 2, &bar_str, style);
            return;
        }

        // Resample bars to fit the available width
        let bars = if self.data.bars.len() == width {
            self.data.bars.clone()
        } else {
            let ratio = self.data.bars.len() as f32 / width as f32;
            (0..width)
                .map(|i| {
                    let src = (i as f32 * ratio) as usize;
                    self.data.bars[src.min(self.data.bars.len() - 1)]
                })
                .collect()
        };

        // Render bars as a single row of half-block characters
        let y = area.y + area.height / 2;
        for (i, &mag) in bars.iter().enumerate() {
            let ch = magnitude_to_char(mag);
            let color = bar_color(mag);
            let style = Style::default().fg(color);
            buf.set_string(area.x + i as u16, y, &ch.to_string(), style);
        }
    }
}

/// Pick a color based on magnitude: green (low) -> yellow (mid) -> red (high).
fn bar_color(magnitude: f32) -> Color {
    let m = magnitude.clamp(0.0, 1.0);
    if m < 0.33 {
        Color::Green
    } else if m < 0.66 {
        Color::Yellow
    } else {
        Color::Red
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fft_computation() {
        // FFT can be computed on an audio buffer chunk
        let samples: Vec<f32> = (0..1024)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        let spectrum = compute_spectrum(&samples);
        assert!(!spectrum.is_empty());
        assert_eq!(spectrum.len(), 512); // half of FFT size
    }

    #[test]
    fn test_fft_known_frequency() {
        // Generate 440Hz sine wave at 16kHz sample rate, 1024 samples
        let sample_rate = 16000.0;
        let freq = 440.0;
        let n = 1024;
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
            .collect();
        let spectrum = compute_spectrum(&samples);

        // The expected bin for 440Hz: bin = freq * n / sample_rate = 440 * 1024 / 16000 = 28.16
        let expected_bin = (freq * n as f32 / sample_rate) as usize;
        // Find the peak bin
        let peak_bin = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        // Peak should be within 1 bin of expected
        assert!(
            (peak_bin as i32 - expected_bin as i32).unsigned_abs() <= 1,
            "Expected peak near bin {}, got {}",
            expected_bin,
            peak_bin
        );
    }

    #[test]
    fn test_frequency_binning() {
        let spectrum: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        let binned = bin_spectrum(&spectrum, 20);
        assert_eq!(binned.len(), 20);
        // All bins should be non-negative
        assert!(binned.iter().all(|&v| v >= 0.0));
    }

    #[test]
    fn test_magnitude_to_character_mapping() {
        assert_eq!(magnitude_to_char(0.0), ' ');
        assert_eq!(magnitude_to_char(1.0), '█');
        // Mid-range should be a middle character
        let mid = magnitude_to_char(0.5);
        assert!(BAR_CHARS.contains(&mid));
    }

    #[test]
    fn test_magnitude_normalization() {
        let mags = vec![0.0, 0.5, 1.0, 2.0];
        let normed = normalize_magnitudes(&mags, 0.0, 2.0);
        assert_eq!(normed.len(), 4);
        assert!(normed.iter().all(|&v| (0.0..=1.0).contains(&v)));
        // Maximum input should map to 1.0
        assert_eq!(normed[3], 1.0);
    }

    #[test]
    fn test_zero_audio_visualization() {
        let samples = vec![0.0; 1024];
        let spectrum = compute_spectrum(&samples);
        // All magnitudes should be zero or near-zero
        assert!(spectrum.iter().all(|&v| v < 1e-10));
    }

    #[test]
    fn test_loud_audio_visualization() {
        // Full-scale sine wave
        let samples: Vec<f32> = (0..1024)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        let data = SpectrumData::from_samples(&samples, 1024, 20, 0.0);
        // At least one bar should be non-zero
        assert!(data.bars.iter().any(|&v| v > 0.0));
    }

    #[test]
    fn test_rolling_window() {
        // Simulating a rolling window: new samples added, FFT on latest chunk
        let mut all_samples = Vec::new();
        for i in 0..3 {
            let chunk: Vec<f32> = (0..512)
                .map(|j| ((i * 512 + j) as f32 * 0.01).sin())
                .collect();
            all_samples.extend_from_slice(&chunk);
            if all_samples.len() >= 1024 {
                let data = SpectrumData::from_samples(&all_samples, 1024, 10, 0.0);
                assert_eq!(data.bars.len(), 10);
            }
        }
    }

    #[test]
    fn test_window_function_application() {
        let mut samples = vec![1.0; 64];
        apply_hann_window(&mut samples);
        // Hann window: endpoints should be near zero, middle should be near 1.0
        assert!(samples[0].abs() < 0.01);
        assert!(samples[63].abs() < 0.1);
        assert!((samples[32] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_spectrogram_data_structure() {
        let samples: Vec<f32> = (0..1024)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        let data = SpectrumData::from_samples(&samples, 1024, 32, 0.0);
        assert_eq!(data.bars.len(), 32);
    }

    #[test]
    fn test_character_set_mapping() {
        // Verify progressive mapping from low to high
        let chars: Vec<char> = (1..=8)
            .map(|i| magnitude_to_char(i as f32 / 8.0))
            .collect();
        // Each successive character should be >= the previous (monotonic)
        for pair in chars.windows(2) {
            assert!(pair[1] >= pair[0], "{} should be >= {}", pair[1], pair[0]);
        }
    }

    #[test]
    fn test_frequency_range_focus() {
        // Speech frequencies should be represented in binned output
        let sample_rate = 16000.0;
        let n = 1024;
        // Generate 200Hz sine (within speech range)
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 200.0 * i as f32 / sample_rate).sin())
            .collect();
        let spectrum = compute_spectrum(&samples);
        let binned = bin_spectrum(&spectrum, 20);
        // Lower bins should have more energy (200Hz is low)
        let lower_energy: f32 = binned[..5].iter().sum();
        let upper_energy: f32 = binned[15..].iter().sum();
        assert!(lower_energy > upper_energy);
    }

    #[test]
    fn test_real_time_update() {
        // Simulating continuous reads from a ring buffer
        let mut samples: Vec<f32> = vec![0.0; 2048];
        for frame in 0..5 {
            // Add new audio chunk
            let chunk: Vec<f32> = (0..256)
                .map(|i| (2.0 * std::f32::consts::PI * 440.0 * (frame * 256 + i) as f32 / 16000.0).sin())
                .collect();
            let start = (frame * 256) % samples.len();
            let len = samples.len();
            for (j, &s) in chunk.iter().enumerate() {
                samples[(start + j) % len] = s;
            }
            let data = SpectrumData::from_samples(&samples, 1024, 20, 0.0);
            assert_eq!(data.bars.len(), 20);
        }
    }

    #[test]
    fn test_fft_size_selection() {
        // Various FFT sizes should all work
        for &size in &[256, 512, 1024, 2048] {
            let samples: Vec<f32> = (0..size)
                .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
                .collect();
            let spectrum = compute_spectrum(&samples);
            assert_eq!(spectrum.len(), size / 2);
        }
    }

    #[test]
    fn test_noise_floor_handling() {
        // Very quiet signal should be suppressed by noise floor
        let samples: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.001).sin() * 0.001).collect();
        let spectrum = compute_spectrum(&samples);
        let normed = normalize_magnitudes(&spectrum, 0.01, 0.05);
        // Most bins should be zero after noise floor
        let nonzero = normed.iter().filter(|&&v| v > 0.0).count();
        assert!(nonzero < spectrum.len() / 2, "Too many bins above noise floor");
    }

    #[test]
    fn test_empty_spectrum_data() {
        let data = SpectrumData::empty();
        assert!(data.bars.is_empty());
    }

    #[test]
    fn test_too_few_samples() {
        // Fewer samples than FFT size should return empty
        let samples = vec![0.5; 100];
        let data = SpectrumData::from_samples(&samples, 1024, 20, 0.0);
        assert!(data.bars.is_empty());
    }

    #[test]
    fn test_bin_spectrum_empty() {
        assert_eq!(bin_spectrum(&[], 10), vec![0.0; 10]);
        assert_eq!(bin_spectrum(&[1.0], 0), Vec::<f32>::new());
    }

    #[test]
    fn test_bar_color() {
        assert_eq!(bar_color(0.1), Color::Green);
        assert_eq!(bar_color(0.5), Color::Yellow);
        assert_eq!(bar_color(0.9), Color::Red);
    }
}
