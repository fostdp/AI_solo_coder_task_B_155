use crate::models::{Alert, ResonanceAnalysisResult, SourceLocalizationResult, WebSocketMessage};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use uuid::Uuid;

pub struct AlertManager {
    frequency_drift_threshold: f64,
    localization_bias_threshold: f64,
    cooldown: Duration,
    last_alerts: DashMap<String, Instant>,
    tx: broadcast::Sender<WebSocketMessage>,
}

impl AlertManager {
    pub fn new(
        frequency_drift_warning: f64,
        localization_bias_warning: f64,
        cooldown_seconds: u64,
        tx: broadcast::Sender<WebSocketMessage>,
    ) -> Arc<Self> {
        Arc::new(Self {
            frequency_drift_threshold: frequency_drift_warning,
            localization_bias_threshold: localization_bias_warning,
            cooldown: Duration::from_secs(cooldown_seconds),
            last_alerts: DashMap::new(),
            tx,
        })
    }

    pub fn check_resonance(&self, analysis: &ResonanceAnalysisResult) -> Option<Alert> {
        if analysis.drift_percent >= self.frequency_drift_threshold {
            let severity = if analysis.drift_percent >= self.frequency_drift_threshold * 2.0 {
                "critical"
            } else {
                "warning"
            };

            let alert_key = format!("drift_{}", analysis.device_id);
            if !self.should_emit_alert(&alert_key) {
                return None;
            }

            let alert = Alert {
                timestamp: Utc::now(),
                alert_id: Uuid::new_v4(),
                alert_type: "frequency_drift".to_string(),
                severity: severity.to_string(),
                device_id: Some(analysis.device_id),
                message: format!(
                    "设备 {} 共振频率漂移 {:.2}% 超过阈值",
                    analysis.device_id, analysis.drift_percent
                ),
                details: format!(
                    "理论频率: {:.2}Hz, 实测频率: {:.2}Hz, 漂移: {:.2}Hz, 增益: {:.2}dB",
                    analysis.theoretical_resonance_freq,
                    analysis.measured_resonance_freq,
                    analysis.frequency_drift,
                    analysis.gain_db
                ),
                is_resolved: false,
            };

            self.broadcast_alert(&alert);
            self.mark_alert_emitted(&alert_key);
            Some(alert)
        } else {
            None
        }
    }

    pub fn check_localization(&self, loc: &SourceLocalizationResult) -> Option<Alert> {
        let bias_score = (1.0 - loc.confidence) * 100.0;
        if bias_score >= self.localization_bias_threshold && loc.confidence > 0.0 {
            let severity = if bias_score >= self.localization_bias_threshold * 1.5 {
                "critical"
            } else {
                "warning"
            };

            let alert_key = format!("loc_{}", loc.source_id);
            if !self.should_emit_alert(&alert_key) {
                return None;
            }

            let alert = Alert {
                timestamp: Utc::now(),
                alert_id: Uuid::new_v4(),
                alert_type: "localization_bias".to_string(),
                severity: severity.to_string(),
                device_id: None,
                message: format!(
                    "声源 {} 定位置信度 {:.2}% 偏低，可能存在偏差",
                    loc.source_id,
                    loc.confidence * 100.0
                ),
                details: format!(
                    "估计位置: ({:.1}, {:.1}, {:.1}), 方位角: {:.1}°, 距离: {:.1}m, 使用设备: {:?}",
                    loc.source_x, loc.source_y, loc.source_z,
                    loc.bearing_angle, loc.distance_estimate, loc.used_devices
                ),
                is_resolved: false,
            };

            self.broadcast_alert(&alert);
            self.mark_alert_emitted(&alert_key);
            Some(alert)
        } else {
            None
        }
    }

    fn should_emit_alert(&self, key: &str) -> bool {
        match self.last_alerts.get(key) {
            Some(last) => last.elapsed() >= self.cooldown,
            None => true,
        }
    }

    fn mark_alert_emitted(&self, key: &str) {
        self.last_alerts.insert(key.to_string(), Instant::now());
    }

    fn broadcast_alert(&self, alert: &Alert) {
        let msg = WebSocketMessage::new("alert", alert.clone());
        let _ = self.tx.send(msg);
    }

    pub fn broadcast_sensor_data(&self, data: &crate::models::SensorReading) {
        let msg = WebSocketMessage::new("sensor_data", data.clone());
        let _ = self.tx.send(msg);
    }

    pub fn broadcast_localization(&self, loc: &SourceLocalizationResult) {
        let msg = WebSocketMessage::new("localization", loc.clone());
        let _ = self.tx.send(msg);
    }

    pub fn broadcast_resonance(&self, analysis: &ResonanceAnalysisResult) {
        let msg = WebSocketMessage::new("resonance", analysis.clone());
        let _ = self.tx.send(msg);
    }

    pub fn sender(&self) -> broadcast::Sender<WebSocketMessage> {
        self.tx.clone()
    }
}
