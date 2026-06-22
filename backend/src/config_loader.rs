use crate::localization::BeamformingMethod;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub mqtt: MqttConfig,
    pub clickhouse: ClickHouseConfig,
    pub localization: LocalizationConfig,
    pub alert: AlertConfig,
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub static_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttConfig {
    pub broker: String,
    pub port: u16,
    pub client_id: String,
    pub topic: String,
    pub keep_alive_secs: u64,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClickHouseConfig {
    pub url: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocalizationConfig {
    pub sound_speed_soil: f64,
    pub beamforming_resolution: f64,
    pub max_localization_distance: f64,
    pub localization_confidence_threshold: f64,
    pub beamforming_method: BeamformingMethod,
    pub diagonal_loading: f64,
    pub multipath_suppression: bool,
    pub min_active_devices: usize,
    pub recent_readings_per_device: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlertConfig {
    pub frequency_drift_warning_percent: f64,
    pub localization_bias_warning_percent: f64,
    pub cooldown_seconds: u64,
    pub broadcast_channel_capacity: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PipelineConfig {
    pub mqtt_to_acoustic_buffer: usize,
    pub acoustic_to_locator_buffer: usize,
    pub locator_to_alarm_buffer: usize,
    pub sensor_raw_to_db_buffer: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AcousticsConfig {
    pub speed_of_sound_air: f64,
    pub default_urn_volume: f64,
    pub default_neck_radius: f64,
    pub default_neck_length: f64,
    pub default_wall_thickness: f64,
    pub default_rim_flange_width: f64,
    pub default_shape: String,
    pub drift_warning_threshold_percent: f64,
    pub drift_critical_threshold_percent: f64,
    pub bem_enabled: bool,
    pub bem_boundary_elements: usize,
    pub kinematic_viscosity: f64,
    pub air_density: f64,
    pub quality_factor_min: f64,
    pub quality_factor_max: f64,
    pub reference_spl_db: f64,
    pub reference_spl_distance_m: f64,
    pub spl_attenuation_db_per_octave: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediumPropertyConfig {
    pub medium_type: String,
    pub display_name: String,
    pub density: f64,
    pub sound_speed: f64,
    pub attenuation_coeff: f64,
    pub depth_start: f64,
    pub thickness: f64,
    pub color: String,
}

pub struct ConfigBundle {
    pub app: AppConfig,
    pub acoustics: AcousticsConfig,
    pub media: Vec<MediumPropertyConfig>,
}

impl ConfigBundle {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let base = resolve_config_dir();
        let app: AppConfig = load_json(&base.join("app.json"))?;
        let acoustics: AcousticsConfig = load_json(&base.join("acoustics.json"))?;
        let media: Vec<MediumPropertyConfig> = load_json(&base.join("medium_properties.json"))?;
        Ok(Self { app, acoustics, media })
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let app: AppConfig = load_json(&dir.join("app.json"))?;
        let acoustics: AcousticsConfig = load_json(&dir.join("acoustics.json"))?;
        let media: Vec<MediumPropertyConfig> = load_json(&dir.join("medium_properties.json"))?;
        Ok(Self { app, acoustics, media })
    }
}

fn resolve_config_dir() -> PathBuf {
    let candidates: Vec<PathBuf> = vec![
        std::env::var("URN_CONFIG_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("./config")),
        Some(PathBuf::from("../config")),
        Some(PathBuf::from("/etc/urn-acoustics")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for c in candidates {
        if c.join("app.json").exists() {
            return c;
        }
    }
    PathBuf::from("./config")
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取配置文件 {} 失败: {}", path.display(), e))?;
    let value: T = serde_json::from_str(&content)
        .map_err(|e| format!("解析配置文件 {} 失败: {}", path.display(), e))?;
    Ok(value)
}
