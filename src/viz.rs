// Viz Module - Waveform amplitude display in ratatui TUI
//
// Phase 2: Takes audio samples from the ring buffer, computes RMS energy
// over windows, and renders a scrolling braille waveform visualization.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

/// Convert a 4-row x 2-column dot grid to a braille Unicode character.
///
/// Braille dot positions map to bits as:
/// ```text
/// ┌───┬───┐    ┌────┬────┐
/// │ 0 │ 3 │    │ b0 │ b3 │
/// │ 1 │ 4 │    │ b1 │ b4 │
/// │ 2 │ 5 │    │ b2 │ b5 │
/// │ 6 │ 7 │    │ b6 │ b7 │
/// └───┴───┘    └────┴────┘
/// ```
fn dots_to_braille(dots: [[bool; 2]; 4]) -> char {
    let mut bits: u8 = 0;
    // Left column: bits 0, 1, 2, 6
    if dots[0][0] {
        bits |= 1 << 0;
    }
    if dots[1][0] {
        bits |= 1 << 1;
    }
    if dots[2][0] {
        bits |= 1 << 2;
    }
    if dots[3][0] {
        bits |= 1 << 6;
    }
    // Right column: bits 3, 4, 5, 7
    if dots[0][1] {
        bits |= 1 << 3;
    }
    if dots[1][1] {
        bits |= 1 << 4;
    }
    if dots[2][1] {
        bits |= 1 << 5;
    }
    if dots[3][1] {
        bits |= 1 << 7;
    }
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

/// A canvas of dot-pixels that maps to braille characters.
///
/// Each terminal cell is 2 dots wide and 4 dots tall.
struct BrailleCanvas {
    width: usize,  // in dot pixels (terminal_cols * 2)
    height: usize, // in dot pixels (terminal_rows * 4)
    dots: Vec<bool>,
}

impl BrailleCanvas {
    fn new(terminal_cols: usize, terminal_rows: usize) -> Self {
        let width = terminal_cols * 2;
        let height = terminal_rows * 4;
        Self {
            width,
            height,
            dots: vec![false; width * height],
        }
    }

    fn set_dot(&mut self, x: usize, y: usize) {
        if x < self.width && y < self.height {
            self.dots[y * self.width + x] = true;
        }
    }

    fn get_dot(&self, x: usize, y: usize) -> bool {
        if x < self.width && y < self.height {
            self.dots[y * self.width + x]
        } else {
            false
        }
    }

    /// Fill a vertical line of dots from y_start to y_end (inclusive).
    fn fill_vertical_line(&mut self, x: usize, y_start: usize, y_end: usize) {
        if x >= self.width {
            return;
        }
        let start = y_start.min(y_end);
        let end = y_start.max(y_end).min(self.height - 1);
        for y in start..=end {
            self.dots[y * self.width + x] = true;
        }
    }

    /// Convert the dot canvas to a grid of braille characters.
    fn to_braille_grid(&self) -> Vec<Vec<char>> {
        let cols = self.width / 2;
        let rows = self.height / 4;
        let mut grid = vec![vec![' '; cols]; rows];
        for row in 0..rows {
            for col in 0..cols {
                let px = col * 2;
                let py = row * 4;
                let dots = [
                    [self.get_dot(px, py), self.get_dot(px + 1, py)],
                    [self.get_dot(px, py + 1), self.get_dot(px + 1, py + 1)],
                    [self.get_dot(px, py + 2), self.get_dot(px + 1, py + 2)],
                    [self.get_dot(px, py + 3), self.get_dot(px + 1, py + 3)],
                ];
                grid[row][col] = dots_to_braille(dots);
            }
        }
        grid
    }
}

/// Render waveform amplitudes onto a braille canvas as a symmetric mirrored display.
fn render_waveform_to_canvas(bars: &[f32], canvas: &mut BrailleCanvas) {
    let center = canvas.height / 2;
    for (i, &amp) in bars.iter().enumerate() {
        let amp = amp.clamp(0.0, 1.0);
        let extent = (amp * center as f32).round() as usize;
        if extent == 0 {
            continue;
        }
        // Fill both left and right sub-pixels for each bar
        let px_left = i * 2;
        let px_right = i * 2 + 1;
        let y_top = center.saturating_sub(extent);
        let y_bot = (center + extent - 1).min(canvas.height - 1);
        canvas.fill_vertical_line(px_left, y_top, y_bot);
        canvas.fill_vertical_line(px_right, y_top, y_bot);
    }
}

/// Pick a color based on amplitude: green (low) -> yellow (mid) -> red (high).
fn waveform_color(amplitude: f32) -> Color {
    let a = amplitude.clamp(0.0, 1.0);
    if a < 0.30 {
        Color::Green
    } else if a < 0.60 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// Compute RMS energy for each of `num_windows` equal-sized chunks of samples.
///
/// Returns a Vec of RMS values, one per window. If samples is empty or
/// num_windows is 0, returns an empty/zero-filled vec.
pub fn compute_rms_windows(samples: &[f32], num_windows: usize) -> Vec<f32> {
    if samples.is_empty() || num_windows == 0 {
        return vec![0.0; num_windows];
    }

    let chunk_size = samples.len() / num_windows;
    if chunk_size == 0 {
        return vec![0.0; num_windows];
    }

    (0..num_windows)
        .map(|i| {
            let start = i * chunk_size;
            let end = if i == num_windows - 1 {
                samples.len()
            } else {
                start + chunk_size
            };
            let chunk = &samples[start..end];
            let sum_sq: f32 = chunk.iter().map(|&s| s * s).sum();
            (sum_sq / chunk.len() as f32).sqrt()
        })
        .collect()
}

/// Normalize magnitudes to 0.0..=1.0 range with a noise floor threshold.
///
/// Values below `noise_floor` are zeroed. The peak is scaled against
/// whichever is larger: the actual max or `min_ref`.
pub fn normalize_magnitudes(magnitudes: &[f32], noise_floor: f32, min_ref: f32) -> Vec<f32> {
    if magnitudes.is_empty() {
        return Vec::new();
    }

    let max = magnitudes.iter().cloned().fold(0.0_f32, f32::max);
    if max <= noise_floor {
        return vec![0.0; magnitudes.len()];
    }

    let ref_level = max.max(min_ref);

    magnitudes
        .iter()
        .map(|&m| {
            if m <= noise_floor {
                0.0
            } else {
                (m / ref_level).clamp(0.0, 1.0)
            }
        })
        .collect()
}

/// Holds the current waveform data ready for rendering.
pub struct WaveformData {
    /// Normalized RMS amplitudes (0.0..=1.0), one per display column.
    pub bars: Vec<f32>,
}

impl WaveformData {
    /// Create an empty waveform (silence).
    pub fn empty() -> Self {
        Self { bars: Vec::new() }
    }

    /// Compute waveform data from raw audio samples.
    ///
    /// `num_columns`: number of display columns to produce.
    /// `noise_floor`: RMS threshold below which windows are zeroed.
    pub fn from_samples(samples: &[f32], num_columns: usize, noise_floor: f32) -> Self {
        if samples.is_empty() || num_columns == 0 {
            return Self::empty();
        }

        let rms_windows = compute_rms_windows(samples, num_columns);
        let bars = normalize_magnitudes(&rms_windows, noise_floor, 0.05);

        Self { bars }
    }
}

/// A ratatui widget that renders a waveform amplitude display.
pub struct WaveformWidget<'a> {
    data: &'a WaveformData,
}

impl<'a> WaveformWidget<'a> {
    pub fn new(data: &'a WaveformData) -> Self {
        Self { data }
    }
}

impl Widget for WaveformWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 1 || area.height < 1 {
            return;
        }

        let waveform_cols = area.width as usize;
        let waveform_rows = area.height as usize;

        // Silence / idle: render thin center line using braille dots
        if self.data.bars.is_empty() {
            let mut canvas = BrailleCanvas::new(waveform_cols, waveform_rows);
            let center_y = canvas.height / 2;
            for x in 0..canvas.width {
                canvas.set_dot(x, center_y);
            }
            let grid = canvas.to_braille_grid();
            for (row_idx, row) in grid.iter().enumerate() {
                let s: String = row.iter().collect();
                buf.set_string(
                    area.x,
                    area.y + row_idx as u16,
                    &s,
                    Style::default().fg(Color::DarkGray),
                );
            }
            return;
        }

        // Resample bars to fit the available width
        let bars = if self.data.bars.len() == waveform_cols {
            self.data.bars.clone()
        } else {
            let ratio = self.data.bars.len() as f32 / waveform_cols as f32;
            (0..waveform_cols)
                .map(|i| {
                    let src = (i as f32 * ratio) as usize;
                    self.data.bars[src.min(self.data.bars.len() - 1)]
                })
                .collect()
        };

        // Render braille waveform
        let mut canvas = BrailleCanvas::new(waveform_cols, waveform_rows);
        render_waveform_to_canvas(&bars, &mut canvas);
        let grid = canvas.to_braille_grid();

        for (row_idx, row) in grid.iter().enumerate() {
            for (col_idx, &ch) in row.iter().enumerate() {
                let amp = bars[col_idx.min(bars.len() - 1)];
                let color = waveform_color(amp);
                buf.set_string(
                    area.x + 2 + col_idx as u16,
                    area.y + row_idx as u16,
                    &ch.to_string(),
                    Style::default().fg(color),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Braille encoding tests ---

    #[test]
    fn test_dots_to_braille_empty() {
        let dots = [[false; 2]; 4];
        assert_eq!(dots_to_braille(dots), '\u{2800}'); // empty braille
    }

    #[test]
    fn test_dots_to_braille_full() {
        let dots = [[true; 2]; 4];
        assert_eq!(dots_to_braille(dots), '\u{28FF}'); // all dots
    }

    #[test]
    fn test_dots_to_braille_left_only() {
        let dots = [[true, false], [true, false], [true, false], [true, false]];
        // Left column = bits 0,1,2,6 = 1+2+4+64 = 71 = 0x47
        assert_eq!(dots_to_braille(dots), '\u{2847}');
    }

    #[test]
    fn test_dots_to_braille_right_only() {
        let dots = [[false, true], [false, true], [false, true], [false, true]];
        // Right column = bits 3,4,5,7 = 8+16+32+128 = 184 = 0xB8
        assert_eq!(dots_to_braille(dots), '\u{28B8}');
    }

    // --- BrailleCanvas tests ---

    #[test]
    fn test_canvas_set_get_dot() {
        let mut canvas = BrailleCanvas::new(5, 3); // 10 x 12 dots
        assert!(!canvas.get_dot(3, 5));
        canvas.set_dot(3, 5);
        assert!(canvas.get_dot(3, 5));
        // Out of bounds should not panic
        assert!(!canvas.get_dot(100, 100));
    }

    #[test]
    fn test_canvas_fill_vertical() {
        let mut canvas = BrailleCanvas::new(2, 2); // 4 x 8 dots
        canvas.fill_vertical_line(1, 2, 5);
        assert!(!canvas.get_dot(1, 1));
        assert!(canvas.get_dot(1, 2));
        assert!(canvas.get_dot(1, 3));
        assert!(canvas.get_dot(1, 4));
        assert!(canvas.get_dot(1, 5));
        assert!(!canvas.get_dot(1, 6));
    }

    #[test]
    fn test_canvas_to_braille() {
        let mut canvas = BrailleCanvas::new(1, 1); // 2 x 4 dots = 1 braille char
        // Set top-left dot only
        canvas.set_dot(0, 0);
        let grid = canvas.to_braille_grid();
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].len(), 1);
        assert_eq!(grid[0][0], '\u{2801}'); // bit 0 only
    }

    // --- Waveform rendering tests ---

    #[test]
    fn test_render_silence() {
        let bars = vec![0.0; 5];
        let mut canvas = BrailleCanvas::new(5, 3);
        render_waveform_to_canvas(&bars, &mut canvas);
        // All dots should be false (no extent for 0 amplitude)
        assert!(canvas.dots.iter().all(|&d| !d));
    }

    #[test]
    fn test_render_full_amplitude() {
        let bars = vec![1.0; 4];
        let mut canvas = BrailleCanvas::new(4, 2); // 8x8 dots
        render_waveform_to_canvas(&bars, &mut canvas);
        // Full amplitude should fill most of the vertical range
        let center = canvas.height / 2;
        // Center column should have dots above and below center
        assert!(canvas.get_dot(0, 0)); // top
        assert!(canvas.get_dot(0, center)); // center
        assert!(canvas.get_dot(0, canvas.height - 1)); // bottom
    }

    #[test]
    fn test_render_symmetry() {
        let bars = vec![0.5; 3];
        let mut canvas = BrailleCanvas::new(3, 4); // 6x16 dots
        render_waveform_to_canvas(&bars, &mut canvas);
        let center = canvas.height / 2;
        // Check symmetry around center for column 0
        for offset in 1..center {
            let above = canvas.get_dot(0, center - offset);
            let below = canvas.get_dot(0, center + offset - 1);
            assert_eq!(above, below, "asymmetric at offset {offset}");
        }
    }

    // --- Existing tests (kept unchanged) ---

    #[test]
    fn test_compute_rms_windows_basic() {
        // Constant signal of 1.0 should have RMS of 1.0
        let samples = vec![1.0; 100];
        let rms = compute_rms_windows(&samples, 5);
        assert_eq!(rms.len(), 5);
        for &v in &rms {
            assert!((v - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_compute_rms_windows_silence() {
        let samples = vec![0.0; 100];
        let rms = compute_rms_windows(&samples, 10);
        assert_eq!(rms.len(), 10);
        assert!(rms.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_compute_rms_windows_empty() {
        let rms = compute_rms_windows(&[], 5);
        assert_eq!(rms, vec![0.0; 5]);
    }

    #[test]
    fn test_compute_rms_windows_zero_windows() {
        let rms = compute_rms_windows(&[1.0, 2.0], 0);
        assert!(rms.is_empty());
    }

    #[test]
    fn test_compute_rms_windows_sine() {
        // RMS of a sine wave with amplitude A is A/sqrt(2)
        let n = 4000;
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        let rms = compute_rms_windows(&samples, 4);
        let expected_rms = 1.0 / (2.0_f32).sqrt();
        for &v in &rms {
            assert!(
                (v - expected_rms).abs() < 0.05,
                "expected ~{}, got {}",
                expected_rms,
                v
            );
        }
    }

    #[test]
    fn test_normalize_magnitudes() {
        let mags = vec![0.0, 0.5, 1.0, 2.0];
        let normed = normalize_magnitudes(&mags, 0.0, 2.0);
        assert_eq!(normed.len(), 4);
        assert!(normed.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert_eq!(normed[3], 1.0);
    }

    #[test]
    fn test_normalize_magnitudes_empty() {
        let normed = normalize_magnitudes(&[], 0.0, 1.0);
        assert!(normed.is_empty());
    }

    #[test]
    fn test_normalize_magnitudes_below_noise_floor() {
        let mags = vec![0.001, 0.002, 0.003];
        let normed = normalize_magnitudes(&mags, 0.01, 0.05);
        assert!(normed.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_waveform_data_empty() {
        let data = WaveformData::empty();
        assert!(data.bars.is_empty());
    }

    #[test]
    fn test_waveform_data_from_silence() {
        let samples = vec![0.0; 1024];
        let data = WaveformData::from_samples(&samples, 20, 0.001);
        assert_eq!(data.bars.len(), 20);
        assert!(data.bars.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_waveform_data_from_loud_signal() {
        let samples: Vec<f32> = (0..4096)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        let data = WaveformData::from_samples(&samples, 20, 0.0);
        assert_eq!(data.bars.len(), 20);
        assert!(data.bars.iter().any(|&v| v > 0.0));
    }

    #[test]
    fn test_waveform_data_from_empty() {
        let data = WaveformData::from_samples(&[], 20, 0.0);
        assert!(data.bars.is_empty());
    }

    #[test]
    fn test_waveform_color() {
        assert_eq!(waveform_color(0.1), Color::Green);
        assert_eq!(waveform_color(0.4), Color::Yellow);
        assert_eq!(waveform_color(0.8), Color::Red);
    }

    #[test]
    fn test_rolling_window() {
        // Simulating continuous updates
        let mut all_samples = Vec::new();
        for i in 0..5 {
            let chunk: Vec<f32> = (0..512)
                .map(|j| ((i * 512 + j) as f32 * 0.01).sin())
                .collect();
            all_samples.extend_from_slice(&chunk);
            let data = WaveformData::from_samples(&all_samples, 20, 0.0);
            assert_eq!(data.bars.len(), 20);
        }
    }
}
