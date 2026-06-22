//! Provenance helpers — best-effort live git SHA + ISO-8601 UTC timestamp
//! capture for snapshot envelopes (`--deprecation-report` /
//! `--deprecation-report --snapshot`) and diff documents (`--compare-to`).
//!
//! Lifted out of `main.rs` in v1.14 PR-D3 round-2 so the lib-level
//! `render_deprecation_report_json` can stamp the same provenance keys
//! the diff path emits — snapshots taken by v1.14+ round-trip
//! `snapshot_a` provenance through `--compare-to`.

use crate::compare_to::SnapshotProvenance;

/// Capture provenance for the current run: live git SHA from CWD + ISO-8601
/// UTC timestamp from the system clock. Both fields are best-effort —
/// `sha` is `None` on any git failure, `timestamp` always succeeds.
pub fn capture_current() -> SnapshotProvenance {
    SnapshotProvenance {
        sha: capture_git_sha(),
        timestamp: Some(current_timestamp_iso8601()),
    }
}

/// Captures HEAD of the CWD's repo, NOT of the analyzed source files'
/// enclosing repo. Provenance reflects the invocation context, not the
/// analyzed code. Returns `None` on any failure (no git on PATH, not in
/// a git repo, command non-zero, output not UTF-8) — callers never block
/// on a missing SHA.
pub fn capture_git_sha() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// ISO-8601 UTC timestamp for the current wall-clock moment. Hand-rolled
/// from `SystemTime` to avoid a `chrono` / `time` dep for one format
/// string. Format: `YYYY-MM-DDTHH:MM:SSZ` (second precision).
pub fn current_timestamp_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_secs_as_iso8601(secs)
}

/// Convert a UNIX epoch second count to `YYYY-MM-DDTHH:MM:SSZ`. Civil
/// date math via Howard Hinnant's `days_from_civil` inverse —
/// `days_from_civil` itself ships with chrono / time / etc., here
/// pykrete pays for the inverse arithmetic directly. Algorithm is
/// well-known and benchmarked across libc implementations; covers
/// 1970-01-01 through year 9999 well past anyone's snapshot needs.
pub fn format_unix_secs_as_iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let secs_in_day = secs % 86_400;
    let hour = secs_in_day / 3600;
    let minute = (secs_in_day % 3600) / 60;
    let second = secs_in_day % 60;

    // Days since 1970-01-01 → (year, month, day). Algorithm from
    // Howard Hinnant's "Date Algorithms" paper, civil_from_days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, minute, second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_epoch_zero() {
        assert_eq!(format_unix_secs_as_iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_known_date() {
        // 2026-06-23T12:34:56Z = 1_782_218_096 seconds since epoch.
        assert_eq!(
            format_unix_secs_as_iso8601(1_782_218_096),
            "2026-06-23T12:34:56Z"
        );
    }

    #[test]
    fn iso8601_y2k_boundary() {
        // 2000-03-01T00:00:00Z = 951868800 (leap-year transition).
        assert_eq!(
            format_unix_secs_as_iso8601(951_868_800),
            "2000-03-01T00:00:00Z"
        );
    }
}
