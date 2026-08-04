/// --silent-aware console output: suppressed on success, always shown on error.
pub struct Output {
    silent: bool,
    /// Set when the caller passed `--json`: a single JSON object/array is the
    /// only thing that should reach stdout, so every `info()` line (which
    /// would otherwise interleave free text with that JSON) is suppressed the
    /// same way `--silent` already suppresses it. Kept as a separate flag
    /// rather than folded into `silent` itself — `--silent` and `--json` are
    /// independent CLI flags with independent reasons to exist, and a future
    /// caller reading `silent` alone (e.g. to decide whether to prompt)
    /// shouldn't have to know `json` implies it too.
    json: bool,
}

impl Output {
    pub fn new(silent: bool) -> Self {
        Self { silent, json: false }
    }

    /// Same as `new`, but also suppresses `info()` when `json` is true —
    /// used by every command that supports `--json` (Phase 4 of
    /// roadmap-cheevos-integration.md), so its human-readable progress lines
    /// never interleave with the single JSON payload printed at the end.
    pub fn new_with_json(silent: bool, json: bool) -> Self {
        Self { silent, json }
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        if !self.silent && !self.json {
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
