// GPU load readout for the top bar. See spec/components/gui/viewport.md
// "GPU load readout".
//
// `GpuMonitor` samples the active GPU ~1 Hz and formats a compact
// "GPU 42% · 3.1/24.0 GB" string. The only backend today is NVIDIA NVML
// (`nvml-wrapper`), which loads the NVML library at runtime — so on a
// machine without NVML/NVIDIA the monitor is simply inert and the top
// bar shows nothing.
//
// `GpuBackend` is the seam for future backends — Windows PDH (any vendor
// on Windows), Linux AMD sysfs (`gpu_busy_percent`), macOS IOReport —
// none of which touch the call site: add an `impl GpuBackend` and pick
// it in `GpuMonitor::new`.

use std::time::{Duration, Instant};

use nvml_wrapper::Nvml;

/// One GPU sample: utilisation percent and VRAM bytes (used / total).
#[derive(Debug, Clone, Copy)]
pub struct GpuSample {
    pub util_pct: u32,
    pub mem_used: u64,
    pub mem_total: u64,
}

/// Sampling cadence. NVML calls are sub-millisecond, but once per second
/// is plenty for a status readout and keeps idle repaints cheap.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// A source of GPU samples. Implementors are platform/vendor specific.
trait GpuBackend {
    /// Sample the primary GPU, or `None` if it can't be read this tick.
    fn sample(&mut self) -> Option<GpuSample>;
}

/// NVIDIA backend over NVML.
struct NvmlBackend {
    nvml: Nvml,
}

impl GpuBackend for NvmlBackend {
    fn sample(&mut self) -> Option<GpuSample> {
        let device = self.nvml.device_by_index(0).ok()?;
        let util = device.utilization_rates().ok()?;
        let mem = device.memory_info().ok()?;
        Some(GpuSample {
            util_pct: util.gpu,
            mem_used: mem.used,
            mem_total: mem.total,
        })
    }
}

/// Polls the active GPU backend at most once per [`SAMPLE_INTERVAL`] and
/// caches the latest sample for the UI to read each frame.
pub struct GpuMonitor {
    backend: Option<Box<dyn GpuBackend>>,
    last: Option<GpuSample>,
    last_at: Option<Instant>,
}

impl GpuMonitor {
    /// Try each backend in priority order; fall back to inert. Never
    /// panics — `Nvml::init` returns `Err` when NVML isn't present.
    pub fn new() -> Self {
        let backend: Option<Box<dyn GpuBackend>> = match Nvml::init() {
            Ok(nvml) => Some(Box::new(NvmlBackend { nvml })),
            Err(_) => None,
        };
        Self {
            backend,
            last: None,
            last_at: None,
        }
    }

    /// True when a backend is available (and the readout should show).
    pub fn is_active(&self) -> bool {
        self.backend.is_some()
    }

    /// Refresh the cached sample if the interval has elapsed. Cheap to
    /// call every frame.
    pub fn sample(&mut self) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let due = self
            .last_at
            .map(|t| t.elapsed() >= SAMPLE_INTERVAL)
            .unwrap_or(true);
        if due {
            self.last = backend.sample();
            self.last_at = Some(Instant::now());
        }
    }

    /// The formatted readout, or `None` when there's nothing to show.
    pub fn label(&self) -> Option<String> {
        self.last.as_ref().map(format_label)
    }
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a sample as `"GPU 42% · 3.1/24.0 GB"`. Pure for testability.
fn format_label(s: &GpuSample) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!(
        "GPU {}% · {:.1}/{:.1} GB",
        s.util_pct,
        s.mem_used as f64 / GIB,
        s.mem_total as f64 / GIB,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_label_renders_percent_and_vram() {
        let s = GpuSample {
            util_pct: 42,
            mem_used: 3_328_599_654,
            mem_total: 25_757_220_864,
        };
        assert_eq!(format_label(&s), "GPU 42% · 3.1/24.0 GB");
    }

    #[test]
    fn format_label_handles_zero() {
        let s = GpuSample {
            util_pct: 0,
            mem_used: 0,
            mem_total: 25_757_220_864,
        };
        assert_eq!(format_label(&s), "GPU 0% · 0.0/24.0 GB");
    }
}
