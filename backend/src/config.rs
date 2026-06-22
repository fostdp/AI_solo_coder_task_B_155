use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub mqtt: MqttConfig,
    pub clickhouse: ClickHouseConfig,
    pub acoustics: AcousticsConfig,
    pub localization: LocalizationConfig,
    pub alert: AlertConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub static_dir: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    pub broker: String,
    pub port: u16,
    pub client_id: String,
    pub topic: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClickHouseConfig {
    pub url: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AcousticsConfig {
    pub speed_of_sound: f64,
    pub default_urn_volume: f64,
    pub default_neck_radius: f64,
    pub default_neck_length: f64,
    pub drift_warning_threshold_percent: f64,
    pub drift_critical_threshold_percent: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocalizationConfig {
    pub sound_speed_soil: f64,
    pub beamforming_resolution: f64,
    pub max_localization_distance: f64,
    pub localization_confidence_threshold: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AlertConfig {
    pub frequency_drift_warning: f64,
    pub localization_bias_warning: f64,
    pub cooldown_seconds: u64,
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        let config = config::Config::builder()
            .add_source(config::File::with_name("config"))
            .add_source(config::Environment::with_prefix("URN").separator("__"))
            .build()?;
        config.try_deserialize()
    }
}
