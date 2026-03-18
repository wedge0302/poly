use std::collections::VecDeque;

#[derive(Debug, Default)]
pub struct LatencyTracker {
    samples_us: VecDeque<u128>,
    cap: usize,
}

impl LatencyTracker {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            samples_us: VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn observe(&mut self, us: u128) {
        if self.samples_us.len() >= self.cap {
            self.samples_us.pop_front();
        }
        self.samples_us.push_back(us);
    }

    pub fn p95_us(&self) -> Option<u128> {
        percentile(&self.samples_us, 0.95)
    }
}

#[derive(Debug)]
pub struct Metrics {
    pub decisions: u64,
    pub orders_attempted: u64,
    pub orders_success: u64,
    pub veto_count: u64,
    pub latency: LatencyTracker,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            decisions: 0,
            orders_attempted: 0,
            orders_success: 0,
            veto_count: 0,
            latency: LatencyTracker::with_capacity(20_000),
        }
    }
}

impl Metrics {
    pub fn summary_line(&self) -> String {
        let p95 = self
            .latency
            .p95_us()
            .map(|v| format!("{}us", v))
            .unwrap_or_else(|| "n/a".to_string());
        format!(
            "decisions={} orders={}/{} vetoes={} latency_p95={}",
            self.decisions, self.orders_success, self.orders_attempted, self.veto_count, p95
        )
    }
}

fn percentile(data: &VecDeque<u128>, p: f64) -> Option<u128> {
    if data.is_empty() {
        return None;
    }
    let mut v: Vec<u128> = data.iter().copied().collect();
    v.sort_unstable();
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p95_computes() {
        let mut t = LatencyTracker::with_capacity(32);
        for i in 1..=20 {
            t.observe(i);
        }
        let p95 = t.p95_us().unwrap();
        assert!(p95 >= 18 && p95 <= 20);
    }
}
