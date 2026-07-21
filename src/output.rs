/// --silent-aware console output: suppressed on success, always shown on error.
pub struct Output {
    silent: bool,
}

impl Output {
    pub fn new(silent: bool) -> Self {
        Self { silent }
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        if !self.silent {
            println!("[AutoGSE] {}", msg.as_ref());
        }
    }

    /// Non-fatal warnings (e.g. degraded/missing data from a best-effort
    /// step) print even under `--silent`, mirroring how errors already
    /// bypass it — a silent/scripted caller still needs to see these.
    pub fn warn(&self, msg: impl AsRef<str>) {
        eprintln!("[AutoGSE] warning: {}", msg.as_ref());
    }
}
