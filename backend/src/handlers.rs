use crate::acoustics::{AcousticAnalyzer, HelmholtzResonator};
use crate::alarm_ws::AlarmWsService;
use crate::config_loader::{AcousticsConfig, AppConfig, MediumPropertyConfig};
use crate::models::{MediumProperty, ResonanceAnalysisResult, SensorReading, UrnDevice, WebSocketMessage};
use crate::store::ClickHouseStore;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub store: ClickHouseStore,
    pub alarm_manager: Arc<AlarmWsService>,
    pub devices: Arc<DashMap<u32, UrnDevice>>,
    pub config: AppConfig,
    pub acoustics_config: AcousticsConfig,
    pub media_config: Vec<MediumPropertyConfig>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<u32>,
    pub device_id: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        }
    }
}

pub async fn health_check() -> Json<ApiResponse<String>> {
    Json(ApiResponse::ok("ok".to_string()))
}

pub async fn get_devices(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<UrnDevice>>> {
    let devices: Vec<UrnDevice> = state.devices.iter().map(|d| d.value().clone()).collect();
    Json(ApiResponse::ok(devices))
}

pub async fn get_device(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<ApiResponse<UrnDevice>>, StatusCode> {
    match state.devices.get(&id) {
        Some(d) => Ok(Json(ApiResponse::ok(d.value().clone()))),
        None => Ok(Json(ApiResponse::err("设备不存在"))),
    }
}

pub async fn create_device(
    State(state): State<AppState>,
    Json(device): Json<UrnDevice>,
) -> Result<Json<ApiResponse<UrnDevice>>, StatusCode> {
    info!("[API] 注册新设备: {} (ID={})", device.device_name, device.device_id);
    state.devices.insert(device.device_id, device.clone());
    Ok(Json(ApiResponse::ok(device)))
}

pub async fn get_sensor_data(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<SensorReading>>> {
    let limit = params.limit.unwrap_or(100);
    match state.store.get_recent_sensor_data(params.device_id, limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询传感器数据失败: {}", e))),
    }
}

pub async fn get_recent_localizations(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<crate::models::SourceLocalizationResult>>> {
    let limit = params.limit.unwrap_or(50);
    match state.store.get_recent_localizations(limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询定位结果失败: {}", e))),
    }
}

pub async fn get_recent_alerts(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<crate::models::Alert>>> {
    let limit = params.limit.unwrap_or(50);
    match state.store.get_recent_alerts(limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询告警失败: {}", e))),
    }
}

pub async fn get_medium_properties(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<MediumProperty>>> {
    let media: Vec<MediumProperty> = state
        .media_config
        .iter()
        .map(|m| MediumProperty {
            medium_type: m.medium_type.clone(),
            display_name: m.display_name.clone(),
            density: m.density,
            sound_speed: m.sound_speed,
            attenuation_coeff: m.attenuation_coeff,
            depth_start: m.depth_start,
            thickness: m.thickness,
        })
        .collect();
    Json(ApiResponse::ok(media))
}

pub async fn get_acoustics_config(
    State(state): State<AppState>,
) -> Json<ApiResponse<AcousticsConfig>> {
    Json(ApiResponse::ok(state.acoustics_config.clone()))
}

#[derive(Debug, Deserialize)]
pub struct ResonanceQuery {
    pub volume: Option<f64>,
    pub neck_radius: Option<f64>,
    pub neck_length: Option<f64>,
    pub frequency: Option<f64>,
    pub medium_density: Option<f64>,
}

pub async fn calculate_resonance(
    State(state): State<AppState>,
    Query(params): Query<ResonanceQuery>,
) -> Json<ApiResponse<ResonanceAnalysisResult>> {
    let volume = params.volume.unwrap_or(state.acoustics_config.default_urn_volume);
    let neck_radius = params.neck_radius.unwrap_or(state.acoustics_config.default_neck_radius);
    let neck_length = params.neck_length.unwrap_or(state.acoustics_config.default_neck_length);
    let frequency = params.frequency.unwrap_or(200.0);
    let medium_density = params.medium_density.unwrap_or(1800.0);

    let mut resonator = HelmholtzResonator::new(
        volume,
        neck_radius,
        neck_length,
        state.acoustics_config.speed_of_sound_air,
    );
    let analyzer = AcousticAnalyzer::new(
        state.acoustics_config.speed_of_sound_air,
        state.acoustics_config.drift_warning_threshold_percent,
        state.acoustics_config.drift_critical_threshold_percent,
    )
    .with_bem(state.acoustics_config.bem_enabled);

    let theoretical_freq = resonator.resonance_frequency();
    let q_factor = resonator.quality_factor();
    let gain = resonator.finite_element_gain_correction(frequency, medium_density);
    let drift = frequency - theoretical_freq;
    let drift_percent = (drift / theoretical_freq).abs() * 100.0;
    let is_anomaly = analyzer.check_resonance_anomaly(frequency, theoretical_freq);

    let result = ResonanceAnalysisResult {
        timestamp: Utc::now(),
        device_id: 0,
        measured_resonance_freq: frequency,
        theoretical_resonance_freq: theoretical_freq,
        gain_db: gain,
        quality_factor: q_factor,
        frequency_drift: drift,
        drift_percent,
        is_anomaly,
    };

    Json(ApiResponse::ok(result))
}

pub async fn simulate_reading(
    State(state): State<AppState>,
    Json(reading): Json<SensorReading>,
) -> Json<ApiResponse<String>> {
    info!(
        "[API] 收到模拟读数: device_id={}, spl={:.1}, freq={:.1}",
        reading.device_id, reading.sound_pressure_level, reading.resonance_frequency
    );
    let msg = WebSocketMessage::new("sensor_data", reading);
    match state.alarm_manager.sender().send(msg) {
        Ok(_) => Json(ApiResponse::ok("模拟读数已广播".to_string())),
        Err(e) => Json(ApiResponse::err(&format!("广播失败: {}", e))),
    }
}

pub async fn broadcast_test_message(
    State(state): State<AppState>,
) -> Json<ApiResponse<String>> {
    let test = serde_json::json!({
        "message": "Hello from WebSocket broadcast",
        "timestamp": Utc::now().to_rfc3339(),
    });
    let msg = WebSocketMessage::new("test", test);
    match state.alarm_manager.sender().send(msg) {
        Ok(_) => Json(ApiResponse::ok("测试消息已广播".to_string())),
        Err(e) => Json(ApiResponse::err(&format!("广播失败: {}", e))),
    }
}
