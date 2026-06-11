//! Operational metrics with Prometheus text exposition.
//!
//! The registry is intentionally small and dependency-free: every label is a
//! `&'static str` drawn from a fixed set (route, HTTP method, status class,
//! endpoint category, error kind), so the output is low-cardinality by
//! construction and can never carry tokens, task text or other user data.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Histogram bucket upper bounds (seconds) for HTTP request durations.
const DURATION_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Pre-formatted `le` label values for [`DURATION_BUCKETS`]. Kept as
/// explicit float strings ("1.0", never "1") because the Prometheus text
/// format expects float representation and strict OpenMetrics parsers may
/// reject integer-looking values.
const DURATION_BUCKET_LABELS: [&str; DURATION_BUCKETS.len()] = [
    "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1.0", "2.5", "5.0", "10.0",
];

/// Route label for an HTTP request path. Unknown paths collapse into
/// `"other"` so arbitrary client-supplied paths cannot grow the label set.
pub fn route_label(path: &str) -> &'static str {
    match path {
        "/healthz" => "/healthz",
        "/readyz" => "/readyz",
        "/metrics" => "/metrics",
        _ if path == "/mcp" || path.starts_with("/mcp/") => "/mcp",
        _ => "other",
    }
}

/// Method label for an HTTP method. Unknown methods collapse into `"OTHER"`.
pub fn method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    }
}

/// Status-class label (`2xx`, `4xx`, ...) for an HTTP status code.
pub fn status_class_label(status: u16) -> &'static str {
    match status / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "other",
    }
}

/// (route, method, status class)
type HttpKey = (&'static str, &'static str, &'static str);
/// (endpoint category, outcome)
type VikunjaKey = (&'static str, &'static str);

#[derive(Debug, Default)]
struct DurationHistogram {
    /// Per-bucket (non-cumulative) observation counts; rendered cumulative.
    buckets: [u64; DURATION_BUCKETS.len()],
    /// Observations above the largest bucket (rendered into `+Inf`).
    overflow: u64,
    sum: f64,
    count: u64,
}

impl DurationHistogram {
    fn observe(&mut self, seconds: f64) {
        match DURATION_BUCKETS.iter().position(|&le| seconds <= le) {
            Some(index) => self.buckets[index] += 1,
            None => self.overflow += 1,
        }
        self.sum += seconds;
        self.count += 1;
    }
}

/// Counter registry for one server process.
///
/// All recorded labels are `&'static str` values chosen by this crate;
/// callers cannot inject request-derived strings.
#[derive(Debug, Default)]
pub struct Metrics {
    http_requests: Mutex<BTreeMap<HttpKey, u64>>,
    http_duration: Mutex<DurationHistogram>,
    vikunja_requests: Mutex<BTreeMap<VikunjaKey, u64>>,
    vikunja_retries: Mutex<BTreeMap<&'static str, u64>>,
}

/// Locks a mutex, recovering from poisoning (counters stay usable even if a
/// panic unwound while a lock was held).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Metrics {
    /// Records one handled HTTP request.
    ///
    /// The counter and the duration histogram are guarded by separate locks
    /// taken sequentially, so a concurrent scrape may observe the counter
    /// incremented before the matching histogram observation lands. The two
    /// families are semantically independent, so this momentary divergence
    /// is harmless; it is a deliberate trade against holding one lock for
    /// both updates.
    pub fn record_http_request(
        &self,
        route: &'static str,
        method: &'static str,
        status: u16,
        duration: Duration,
    ) {
        let key = (route, method, status_class_label(status));
        *lock(&self.http_requests).entry(key).or_insert(0) += 1;
        lock(&self.http_duration).observe(duration.as_secs_f64());
    }

    /// Records the outcome of one Vikunja API request. `endpoint` is a fixed
    /// endpoint category like `tasks.get`; `outcome` is `ok` or an error
    /// kind such as `network`, `timeout`, `auth` or `server`.
    pub fn record_vikunja_request(&self, endpoint: &'static str, outcome: &'static str) {
        *lock(&self.vikunja_requests)
            .entry((endpoint, outcome))
            .or_insert(0) += 1;
    }

    /// Records one retry of an idempotent Vikunja request.
    pub fn record_vikunja_retry(&self, endpoint: &'static str) {
        *lock(&self.vikunja_retries).entry(endpoint).or_insert(0) += 1;
    }

    /// Renders the registry in the Prometheus text exposition format
    /// (`text/plain; version=0.0.4`).
    pub fn render(&self) -> String {
        let mut out = String::new();

        out.push_str(
            "# HELP vikunja_mcp_http_requests_total HTTP requests handled, by route, method and status class.\n\
             # TYPE vikunja_mcp_http_requests_total counter\n",
        );
        for ((route, method, status), count) in lock(&self.http_requests).iter() {
            let _ = writeln!(
                out,
                "vikunja_mcp_http_requests_total{{route=\"{route}\",method=\"{method}\",status=\"{status}\"}} {count}"
            );
        }

        out.push_str(
            "# HELP vikunja_mcp_http_request_duration_seconds HTTP request duration in seconds.\n\
             # TYPE vikunja_mcp_http_request_duration_seconds histogram\n",
        );
        {
            let histogram = lock(&self.http_duration);
            let mut cumulative = 0u64;
            for (le, bucket) in DURATION_BUCKET_LABELS.iter().zip(histogram.buckets.iter()) {
                cumulative += bucket;
                let _ = writeln!(
                    out,
                    "vikunja_mcp_http_request_duration_seconds_bucket{{le=\"{le}\"}} {cumulative}"
                );
            }
            let _ = writeln!(
                out,
                "vikunja_mcp_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {}",
                histogram.count
            );
            // Debug formatting keeps the decimal point ("0.0", not "0"),
            // matching the float representation the text format expects.
            let _ = writeln!(
                out,
                "vikunja_mcp_http_request_duration_seconds_sum {:?}",
                histogram.sum
            );
            let _ = writeln!(
                out,
                "vikunja_mcp_http_request_duration_seconds_count {}",
                histogram.count
            );
        }

        out.push_str(
            "# HELP vikunja_mcp_vikunja_requests_total Requests sent to the Vikunja API, by endpoint category and outcome.\n\
             # TYPE vikunja_mcp_vikunja_requests_total counter\n",
        );
        for ((endpoint, outcome), count) in lock(&self.vikunja_requests).iter() {
            let _ = writeln!(
                out,
                "vikunja_mcp_vikunja_requests_total{{endpoint=\"{endpoint}\",outcome=\"{outcome}\"}} {count}"
            );
        }

        out.push_str(
            "# HELP vikunja_mcp_vikunja_retries_total Idempotent Vikunja API requests retried once, by endpoint category.\n\
             # TYPE vikunja_mcp_vikunja_retries_total counter\n",
        );
        for (endpoint, count) in lock(&self.vikunja_retries).iter() {
            let _ = writeln!(
                out,
                "vikunja_mcp_vikunja_retries_total{{endpoint=\"{endpoint}\"}} {count}"
            );
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_labels_collapse_unknown_paths() {
        assert_eq!(route_label("/healthz"), "/healthz");
        assert_eq!(route_label("/readyz"), "/readyz");
        assert_eq!(route_label("/metrics"), "/metrics");
        assert_eq!(route_label("/mcp"), "/mcp");
        assert_eq!(route_label("/mcp/session"), "/mcp");
        assert_eq!(route_label("/mcpx"), "other");
        assert_eq!(route_label("/tasks/42?token=secret"), "other");
        assert_eq!(route_label("/"), "other");
    }

    #[test]
    fn method_labels_collapse_unknown_methods() {
        assert_eq!(method_label("GET"), "GET");
        assert_eq!(method_label("POST"), "POST");
        assert_eq!(method_label("PUT"), "PUT");
        assert_eq!(method_label("DELETE"), "DELETE");
        assert_eq!(method_label("PATCH"), "PATCH");
        assert_eq!(method_label("HEAD"), "HEAD");
        assert_eq!(method_label("OPTIONS"), "OPTIONS");
        assert_eq!(method_label("BREW"), "OTHER");
    }

    #[test]
    fn status_classes_cover_all_codes() {
        assert_eq!(status_class_label(101), "1xx");
        assert_eq!(status_class_label(200), "2xx");
        assert_eq!(status_class_label(308), "3xx");
        assert_eq!(status_class_label(404), "4xx");
        assert_eq!(status_class_label(503), "5xx");
        assert_eq!(status_class_label(7), "other");
    }

    #[test]
    fn http_requests_are_counted_by_route_method_and_class() {
        let metrics = Metrics::default();
        metrics.record_http_request("/healthz", "GET", 200, Duration::from_millis(1));
        metrics.record_http_request("/healthz", "GET", 200, Duration::from_millis(2));
        metrics.record_http_request("/mcp", "POST", 401, Duration::from_millis(3));

        let body = metrics.render();
        assert!(body.contains(
            "vikunja_mcp_http_requests_total{route=\"/healthz\",method=\"GET\",status=\"2xx\"} 2"
        ));
        assert!(body.contains(
            "vikunja_mcp_http_requests_total{route=\"/mcp\",method=\"POST\",status=\"4xx\"} 1"
        ));
    }

    #[test]
    fn duration_histogram_buckets_are_cumulative() {
        let metrics = Metrics::default();
        metrics.record_http_request("/mcp", "POST", 200, Duration::from_millis(1));
        metrics.record_http_request("/mcp", "POST", 200, Duration::from_millis(60));
        metrics.record_http_request("/mcp", "POST", 200, Duration::from_secs(60));

        let body = metrics.render();
        // 1ms falls into the 0.005 bucket; 60ms into 0.1; 60s overflows.
        assert!(
            body.contains("duration_seconds_bucket{le=\"0.005\"} 1"),
            "{body}"
        );
        assert!(
            body.contains("duration_seconds_bucket{le=\"0.1\"} 2"),
            "{body}"
        );
        assert!(
            body.contains("duration_seconds_bucket{le=\"10.0\"} 2"),
            "{body}"
        );
        assert!(
            body.contains("duration_seconds_bucket{le=\"+Inf\"} 3"),
            "{body}"
        );
        assert!(body.contains("duration_seconds_count 3"), "{body}");
    }

    #[test]
    fn bucket_labels_are_float_strings_matching_the_bounds() {
        for (label, bound) in DURATION_BUCKET_LABELS.iter().zip(DURATION_BUCKETS.iter()) {
            assert!(
                label.contains('.'),
                "le label {label} must look like a float"
            );
            assert_eq!(label.parse::<f64>().unwrap(), *bound, "label {label}");
        }
    }

    #[test]
    fn vikunja_outcomes_and_retries_are_counted() {
        let metrics = Metrics::default();
        metrics.record_vikunja_request("tasks.get", "ok");
        metrics.record_vikunja_request("tasks.get", "ok");
        metrics.record_vikunja_request("tasks.get", "not_found");
        metrics.record_vikunja_retry("tasks.get");

        let body = metrics.render();
        assert!(body.contains(
            "vikunja_mcp_vikunja_requests_total{endpoint=\"tasks.get\",outcome=\"ok\"} 2"
        ));
        assert!(body.contains(
            "vikunja_mcp_vikunja_requests_total{endpoint=\"tasks.get\",outcome=\"not_found\"} 1"
        ));
        assert!(body.contains("vikunja_mcp_vikunja_retries_total{endpoint=\"tasks.get\"} 1"));
    }

    #[test]
    fn empty_registry_still_renders_all_metric_families() {
        let body = Metrics::default().render();
        for needle in [
            "# TYPE vikunja_mcp_http_requests_total counter",
            "# TYPE vikunja_mcp_http_request_duration_seconds histogram",
            "# TYPE vikunja_mcp_vikunja_requests_total counter",
            "# TYPE vikunja_mcp_vikunja_retries_total counter",
            "vikunja_mcp_http_request_duration_seconds_sum 0.0",
            "vikunja_mcp_http_request_duration_seconds_count 0",
        ] {
            assert!(body.contains(needle), "missing {needle} in:\n{body}");
        }
    }
}
