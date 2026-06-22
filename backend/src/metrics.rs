use metrics::{counter, gauge, histogram};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static START_TIME: Lazy<Instant> = Lazy::new(Instant::now);
static TOTAL_MQTT_MESSAGES: AtomicU64 = AtomicU64::new(0);
static TOTAL_INVALID_READINGS: AtomicU64 = AtomicU64::new(0);
static TOTAL_ACOUSTIC_CALCS: AtomicU64 = AtomicU64::new(0);
static TOTAL_LOCALIZATIONS: AtomicU64 = AtomicU64::new(0);
static TOTAL_ALERTS: AtomicU64 = AtomicU64::new(0);
static TOTAL_DB_WRITES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_WS_CLIENTS: AtomicU64 = AtomicU64::new(0);

pub fn init_metrics() {
    Lazy::force(&START_TIME);
    info!("[Metrics] 指标系统初始化完成");
}

pub fn inc_mqtt_messages() {
    TOTAL_MQTT_MESSAGES.fetch_add(1, Ordering::Relaxed);
    counter!("urn_mqtt_messages_total", 1);
}

pub fn inc_invalid_readings() {
    TOTAL_INVALID_READINGS.fetch_add(1, Ordering::Relaxed);
    counter!("urn_invalid_readings_total", 1);
}

pub fn inc_acoustic_calcs() {
    TOTAL_ACOUSTIC_CALCS.fetch_add(1, Ordering::Relaxed);
    counter!("urn_acoustic_calculations_total", 1);
}

pub fn inc_localizations() {
    TOTAL_LOCALIZATIONS.fetch_add(1, Ordering::Relaxed);
    counter!("urn_localizations_total", 1);
}

pub fn inc_alerts(severity: &str) {
    TOTAL_ALERTS.fetch_add(1, Ordering::Relaxed);
    counter!("urn_alerts_total", 1, "severity" => severity.to_string());
}

pub fn inc_db_writes(table: &str) {
    TOTAL_DB_WRITES.fetch_add(1, Ordering::Relaxed);
    counter!("urn_db_writes_total", 1, "table" => table.to_string());
}

pub fn ws_client_connected() {
    ACTIVE_WS_CLIENTS.fetch_add(1, Ordering::Relaxed);
    gauge!("urn_ws_active_clients", ACTIVE_WS_CLIENTS.load(Ordering::Relaxed) as f64);
}

pub fn ws_client_disconnected() {
    ACTIVE_WS_CLIENTS.fetch_sub(1, Ordering::Relaxed);
    gauge!("urn_ws_active_clients", ACTIVE_WS_CLIENTS.load(Ordering::Relaxed) as f64);
}

pub fn observe_spl(spl: f64) {
    histogram!("urn_sound_pressure_level", spl);
}

pub fn observe_resonance_drift(drift_percent: f64) {
    histogram!("urn_resonance_drift_percent", drift_percent);
}

pub fn observe_localization_confidence(confidence: f64) {
    histogram!("urn_localization_confidence", confidence);
}

pub fn observe_pipeline_latency_ms(stage: &str, latency_ms: f64) {
    histogram!("urn_pipeline_latency_ms", latency_ms, "stage" => stage.to_string());
}

pub fn uptime_seconds() -> u64 {
    START_TIME.elapsed().as_secs()
}

pub fn render_prometheus() -> String {
    let mut out = String::new();
    let uptime = uptime_seconds();

    out.push_str("# HELP urn_uptime_seconds 服务运行时长（秒）\n");
    out.push_str("# TYPE urn_uptime_seconds gauge\n");
    out.push_str(&format!("urn_uptime_seconds {}\n\n", uptime));

    out.push_str("# HELP urn_mqtt_messages_total MQTT消息接收总数\n");
    out.push_str("# TYPE urn_mqtt_messages_total counter\n");
    out.push_str(&format!("urn_mqtt_messages_total {}\n\n", TOTAL_MQTT_MESSAGES.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_invalid_readings_total 无效传感器读数总数\n");
    out.push_str("# TYPE urn_invalid_readings_total counter\n");
    out.push_str(&format!("urn_invalid_readings_total {}\n\n", TOTAL_INVALID_READINGS.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_acoustic_calculations_total 声学仿真计算总数\n");
    out.push_str("# TYPE urn_acoustic_calculations_total counter\n");
    out.push_str(&format!("urn_acoustic_calculations_total {}\n\n", TOTAL_ACOUSTIC_CALCS.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_localizations_total 声源定位计算总数\n");
    out.push_str("# TYPE urn_localizations_total counter\n");
    out.push_str(&format!("urn_localizations_total {}\n\n", TOTAL_LOCALIZATIONS.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_alerts_total 告警触发总数\n");
    out.push_str("# TYPE urn_alerts_total counter\n");
    out.push_str(&format!("urn_alerts_total {}\n\n", TOTAL_ALERTS.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_db_writes_total 数据库写入总数\n");
    out.push_str("# TYPE urn_db_writes_total counter\n");
    out.push_str(&format!("urn_db_writes_total {}\n\n", TOTAL_DB_WRITES.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_ws_active_clients 活跃WebSocket连接数\n");
    out.push_str("# TYPE urn_ws_active_clients gauge\n");
    out.push_str(&format!("urn_ws_active_clients {}\n\n", ACTIVE_WS_CLIENTS.load(Ordering::Relaxed)));

    out.push_str("# HELP urn_info 构建信息\n");
    out.push_str("# TYPE urn_info gauge\n");
    out.push_str("urn_info{version=\"0.1.0\",rustc_version=\"\",build_time=\"\"} 1\n");

    out
}

use tracing::info;
