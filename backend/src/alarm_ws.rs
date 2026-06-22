use crate::config_loader::AlertConfig;
use crate::metrics;
use crate::models::{Alert, ResonanceAnalysisResult, SensorReading, SourceLocalizationResult, WebSocketMessage};
use crate::pipeline::{AcousticJobResult, LocalizationJobResult, ValidSensorReading};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

pub struct AlarmWsService {
    freq_threshold: f64,
    loc_threshold: f64,
    cooldown: Duration,
    last_alerts: DashMap<String, Instant>,
    tx: broadcast::Sender<WebSocketMessage>,
}

impl AlarmWsService {
    pub fn new(cfg: &AlertConfig) -> Arc<Self> {
        let (tx, _) = broadcast::channel::<WebSocketMessage>(cfg.broadcast_channel_capacity);
        Arc::new(Self {
            freq_threshold: cfg.frequency_drift_warning_percent,
            loc_threshold: cfg.localization_bias_warning_percent,
            cooldown: Duration::from_secs(cfg.cooldown_seconds),
            last_alerts: DashMap::new(),
            tx,
        })
    }

    pub fn sender(&self) -> broadcast::Sender<WebSocketMessage> {
        self.tx.clone()
    }

    pub async fn run(
        self: Arc<Self>,
        mut rx_sensor: mpsc::Receiver<ValidSensorReading>,
        mut rx_acoustic: mpsc::Receiver<AcousticJobResult>,
        mut rx_locator: mpsc::Receiver<LocalizationJobResult>,
        tx_alert_db: mpsc::Sender<Alert>,
    ) {
        info!(
            "[AlarmWs] 告警&WS服务启动 频率阈值={:.1}% 定位阈值={:.1}% 冷却={:?}",
            self.freq_threshold, self.loc_threshold, self.cooldown
        );

        loop {
            tokio::select! {
                Some(valid) = rx_sensor.recv() => {
                    self.broadcast_sensor(&valid.reading);
                }
                Some(job) = rx_acoustic.recv() => {
                    self.broadcast_resonance(&job.analysis);
                    if let Some(alert) = self.check_resonance(&job.analysis) {
                        if let Err(_e) = tx_alert_db.send(alert.clone()).await {
                            warn!("[AlarmWs] 告警入库通道已满");
                        }
                        self.broadcast_alert(&alert);
                    }
                }
                Some(loc) = rx_locator.recv() => {
                    self.broadcast_localization(&loc.result);
                    if let Some(alert) = self.check_localization(&loc.result) {
                        if let Err(_e) = tx_alert_db.send(alert.clone()).await {
                            warn!("[AlarmWs] 告警入库通道已满");
                        }
                        self.broadcast_alert(&alert);
                    }
                }
                else => {
                    error!("[AlarmWs] 所有输入通道已关闭，告警服务退出");
                    break;
                }
            }
        }
    }

    fn check_resonance(&self, analysis: &ResonanceAnalysisResult) -> Option<Alert> {
        if analysis.drift_percent < self.freq_threshold {
            return None;
        }

        let severity = if analysis.drift_percent >= self.freq_threshold * 2.0 {
            "critical"
        } else {
            "warning"
        };

        let key = format!("drift_{}", analysis.device_id);
        if !self.should_emit(&key) {
            return None;
        }

        let alert = Alert {
            timestamp: Utc::now(),
            alert_id: Uuid::new_v4(),
            alert_type: "frequency_drift".to_string(),
            severity: severity.to_string(),
            device_id: Some(analysis.device_id),
            message: format!(
                "设备 {} 共振频率漂移 {:.2}% 超过阈值 {:.1}%",
                analysis.device_id, analysis.drift_percent, self.freq_threshold
            ),
            details: format!(
                "理论频率: {:.2}Hz, 实测频率: {:.2}Hz, 漂移: {:.2}Hz, 增益: {:.2}dB, Q: {:.1}",
                analysis.theoretical_resonance_freq,
                analysis.measured_resonance_freq,
                analysis.frequency_drift,
                analysis.gain_db,
                analysis.quality_factor
            ),
            is_resolved: false,
        };

        metrics::inc_alerts(severity);
        self.mark_emitted(&key);
        Some(alert)
    }

    fn check_localization(&self, loc: &SourceLocalizationResult) -> Option<Alert> {
        let bias_score = (1.0 - loc.confidence) * 100.0;
        if bias_score < self.loc_threshold || loc.confidence <= 0.0 {
            return None;
        }

        let severity = if bias_score >= self.loc_threshold * 1.5 {
            "critical"
        } else {
            "warning"
        };

        let key = format!("loc_{}", loc.source_id);
        if !self.should_emit(&key) {
            return None;
        }

        let alert = Alert {
            timestamp: Utc::now(),
            alert_id: Uuid::new_v4(),
            alert_type: "localization_bias".to_string(),
            severity: severity.to_string(),
            device_id: None,
            message: format!(
                "声源 {} 定位置信度 {:.1}% 偏低（偏差分数 {:.1}）",
                loc.source_id,
                loc.confidence * 100.0,
                bias_score
            ),
            details: format!(
                "估计位置: ({:.1}, {:.1}, {:.1}), 方位角: {:.1}°, 距离: {:.1}m, 设备: {:?}",
                loc.source_x, loc.source_y, loc.source_z,
                loc.bearing_angle, loc.distance_estimate, loc.used_devices
            ),
            is_resolved: false,
        };

        metrics::inc_alerts(severity);
        self.mark_emitted(&key);
        Some(alert)
    }

    fn should_emit(&self, key: &str) -> bool {
        match self.last_alerts.get(key) {
            Some(last) => last.elapsed() >= self.cooldown,
            None => true,
        }
    }

    fn mark_emitted(&self, key: &str) {
        self.last_alerts.insert(key.to_string(), Instant::now());
    }

    fn broadcast_sensor(&self, reading: &SensorReading) {
        let msg = WebSocketMessage::new("sensor_data", reading.clone());
        let _ = self.tx.send(msg);
    }

    fn broadcast_resonance(&self, analysis: &ResonanceAnalysisResult) {
        let msg = WebSocketMessage::new("resonance", analysis.clone());
        let _ = self.tx.send(msg);
    }

    fn broadcast_localization(&self, loc: &SourceLocalizationResult) {
        let msg = WebSocketMessage::new("localization", loc.clone());
        let _ = self.tx.send(msg);
    }

    fn broadcast_alert(&self, alert: &Alert) {
        info!(
            "[AlarmWs] 发送告警 type={} severity={} {}",
            alert.alert_type, alert.severity, alert.message
        );
        let msg = WebSocketMessage::new("alert", alert.clone());
        let _ = self.tx.send(msg);
    }
}
