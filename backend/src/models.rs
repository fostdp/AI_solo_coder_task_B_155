use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct SensorReading {
    pub timestamp: DateTime<Utc>,
    pub device_id: u32,
    pub sound_pressure_level: f64,
    pub resonance_frequency: f64,
    pub source_direction: f64,
    pub medium_density: f64,
    pub temperature: f64,
    pub humidity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct UrnDevice {
    pub device_id: u32,
    pub device_name: String,
    pub deployment_x: f64,
    pub deployment_y: f64,
    pub deployment_z: f64,
    pub urn_volume: f64,
    pub neck_radius: f64,
    pub neck_length: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct ResonanceAnalysisResult {
    pub timestamp: DateTime<Utc>,
    pub device_id: u32,
    pub measured_resonance_freq: f64,
    pub theoretical_resonance_freq: f64,
    pub gain_db: f64,
    pub quality_factor: f64,
    pub frequency_drift: f64,
    pub drift_percent: f64,
    pub is_anomaly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocalizationResult {
    pub timestamp: DateTime<Utc>,
    pub source_id: u64,
    pub source_x: f64,
    pub source_y: f64,
    pub source_z: f64,
    pub bearing_angle: f64,
    pub elevation_angle: f64,
    pub distance_estimate: f64,
    pub confidence: f64,
    pub tdoa_matrix: Vec<Vec<f64>>,
    pub beamformed_power: f64,
    pub used_devices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct Alert {
    pub timestamp: DateTime<Utc>,
    pub alert_id: Uuid,
    pub alert_type: String,
    pub severity: String,
    pub device_id: Option<u32>,
    pub message: String,
    pub details: String,
    pub is_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketMessage {
    pub message_type: String,
    pub data: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

impl WebSocketMessage {
    pub fn new(message_type: &str, data: impl Serialize) -> Self {
        Self {
            message_type: message_type.to_string(),
            data: serde_json::to_value(data).unwrap_or_default(),
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct MediumProperty {
    pub medium_type: String,
    pub display_name: String,
    pub density: f64,
    pub sound_speed: f64,
    pub attenuation_coeff: f64,
    pub depth_start: f64,
    pub thickness: f64,
}
