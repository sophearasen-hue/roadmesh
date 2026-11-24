//! Monitoring and audit log (AUDIT S-21). Counters, a JSON line per security event on
//! standard output (shipped to CloudWatch Logs and kept 1 year), a metrics line every
//! minute, and ALERT lines when a minute looks abnormal.
//!
//! Privacy (CLAUDE.md): never coordinates, pseudonyms, keys or message contents. A
//! device appears only as the first 8 bytes of its device id (server-side identity).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counted things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// Sessions opened (TLS and MQTT CONNECT done).
    Sessions,
    /// Sessions refused (banned device, too many sessions, protocol error).
    SessionsRefused,
    /// Certificate requests answered with certificates.
    EnrolOk,
    /// Certificate requests refused.
    EnrolRefused,
    /// Uploaded frames.
    Uploads,
    /// Uploads that failed a check.
    UploadsRejected,
    /// Events pushed to subscribers (one per receiving session).
    EventsPushed,
    /// Messages dropped by a rate limit.
    RateLimited,
    /// Misbehaviour reports accepted.
    Reports,
    /// Misbehaviour reports refused.
    ReportsRefused,
    /// Devices revoked.
    Revocations,
    /// "Delete my data" requests.
    Deletes,
    /// Pushes dropped because a phone was not reading.
    SlowConsumers,
}

const ALL: [Metric; 13] = [
    Metric::Sessions,
    Metric::SessionsRefused,
    Metric::EnrolOk,
    Metric::EnrolRefused,
    Metric::Uploads,
    Metric::UploadsRejected,
    Metric::EventsPushed,
    Metric::RateLimited,
    Metric::Reports,
    Metric::ReportsRefused,
    Metric::Revocations,
    Metric::Deletes,
    Metric::SlowConsumers,
];

/// Per-minute thresholds for ALERT lines.
#[derive(Debug, Clone)]
pub struct Thresholds {
    /// Certificate requests (ok + refused) per minute.
    pub enrol_per_min: u64,
    /// Refused certificate requests per minute.
    pub enrol_refused_per_min: u64,
    /// Misbehaviour reports per minute.
    pub reports_per_min: u64,
    /// Rejected uploads per minute.
    pub rejected_uploads_per_min: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            enrol_per_min: 600,
            enrol_refused_per_min: 60,
            reports_per_min: 120,
            rejected_uploads_per_min: 6000,
        }
    }
}

/// Counters, totals and the last minute.
#[derive(Debug, Default)]
pub struct Monitor {
    total: [AtomicU64; 13],
    minute: [AtomicU64; 13],
    thresholds: Mutex<Thresholds>,
    sink: Mutex<Option<Vec<String>>>,
}

fn idx(m: Metric) -> usize {
    ALL.iter().position(|x| *x == m).unwrap_or(0)
}

impl Monitor {
    /// A monitor with default thresholds writing to standard output.
    pub fn new() -> Self {
        Self::default()
    }

    /// A monitor that keeps its lines in memory (tests).
    pub fn capturing() -> Self {
        let m = Self::default();
        if let Ok(mut s) = m.sink.lock() {
            *s = Some(Vec::new());
        }
        m
    }

    /// Set the alert thresholds.
    pub fn set_thresholds(&self, t: Thresholds) {
        if let Ok(mut g) = self.thresholds.lock() {
            *g = t;
        }
    }

    /// Count one.
    pub fn inc(&self, m: Metric) {
        self.total[idx(m)].fetch_add(1, Ordering::Relaxed);
        self.minute[idx(m)].fetch_add(1, Ordering::Relaxed);
    }

    /// Total so far.
    pub fn get(&self, m: Metric) -> u64 {
        self.total[idx(m)].load(Ordering::Relaxed)
    }

    fn emit(&self, line: String) {
        if let Ok(mut s) = self.sink.lock()
            && let Some(v) = s.as_mut()
        {
            v.push(line);
            return;
        }
        println!("{line}");
    }

    /// Lines captured so far (tests).
    pub fn lines(&self) -> Vec<String> {
        self.sink
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_default()
    }

    /// An audit line: `event` with plain key/value fields (no personal data).
    pub fn audit(&self, now: u64, event: &str, fields: &[(&str, String)]) {
        let mut m = serde_json::Map::new();
        m.insert("t".into(), now.into());
        m.insert("audit".into(), event.into());
        for (k, v) in fields {
            m.insert((*k).into(), v.clone().into());
        }
        self.emit(serde_json::Value::Object(m).to_string());
    }

    /// Close the minute: write the metrics line and any ALERT lines; returns the alerts.
    pub fn minute_tick(&self, now: u64) -> Vec<String> {
        let mut minute = BTreeMap::new();
        for m in ALL {
            minute.insert(m, self.minute[idx(m)].swap(0, Ordering::Relaxed));
        }
        let mut obj = serde_json::Map::new();
        obj.insert("t".into(), now.into());
        obj.insert("metrics".into(), "minute".into());
        for (m, v) in &minute {
            obj.insert(format!("{m:?}"), (*v).into());
        }
        self.emit(serde_json::Value::Object(obj).to_string());
        let t = self
            .thresholds
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        let get = |m| minute.get(&m).copied().unwrap_or(0);
        let mut alerts = Vec::new();
        let enrol = get(Metric::EnrolOk) + get(Metric::EnrolRefused);
        if enrol > t.enrol_per_min {
            alerts.push(format!("certificate requests {enrol}/min"));
        }
        if get(Metric::EnrolRefused) > t.enrol_refused_per_min {
            alerts.push(format!(
                "refused certificate requests {}/min",
                get(Metric::EnrolRefused)
            ));
        }
        let reports = get(Metric::Reports) + get(Metric::ReportsRefused);
        if reports > t.reports_per_min {
            alerts.push(format!("misbehaviour reports {reports}/min"));
        }
        if get(Metric::UploadsRejected) > t.rejected_uploads_per_min {
            alerts.push(format!(
                "rejected uploads {}/min",
                get(Metric::UploadsRejected)
            ));
        }
        for a in &alerts {
            self.audit(now, "ALERT", &[("what", a.clone())]);
        }
        alerts
    }
}

/// How a device appears in logs: the first 8 bytes of its device id, in hex.
pub fn device_ref(id: &[u8; 32]) -> String {
    id[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_alerts_on_report_spikes_and_resets() {
        let m = Monitor::capturing();
        m.set_thresholds(Thresholds {
            reports_per_min: 3,
            ..Thresholds::default()
        });
        for _ in 0..5 {
            m.inc(Metric::Reports);
        }
        let a = m.minute_tick(60);
        assert_eq!(a, vec!["misbehaviour reports 5/min".to_string()]);
        assert!(m.minute_tick(120).is_empty());
        assert_eq!(m.get(Metric::Reports), 5);
        let lines = m.lines();
        assert!(lines.iter().any(|l| l.contains("\"audit\":\"ALERT\"")));
        assert!(lines[0].contains("\"Reports\":5"));
    }

    #[test]
    fn device_ref_is_short() {
        assert_eq!(device_ref(&[0xab; 32]), "abababababababab");
    }
}
