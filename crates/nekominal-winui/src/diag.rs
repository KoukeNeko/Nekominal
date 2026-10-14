//! Timestamped file log, enabled by `NEKOMINAL_LOG=<file>`. Spike-grade: production should use `tracing`.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    START.get_or_init(Instant::now);
}

pub fn log(message: &str) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("NEKOMINAL_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let elapsed = START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        let _ = writeln!(file, "{elapsed:9.1} ms  {message}");
    }
}
