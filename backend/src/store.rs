use chrono::{DateTime, Utc};
use clickhouse::Row;
use clickhouse::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::models::{
    Alert, MediumProperty, ResonanceAnalysisResult, SensorReading, SourceLocalizationResult, UrnDevice,
};

#[derive(Row, Serialize, Deserialize, Debug, Clone)]
pub struct LocRow {
    pub timestamp: DateTime<Utc>,
    pub source_id: u64,
    pub source_x: f64,
    pub source_y: f64,
    pub source_z: f64,
    pub bearing_angle: f64,
    pub elevation_angle: f64,
    pub distance_estimate: f64,
    pub confidence: f64,
    pub tdoa_matrix: String,
    pub beamformed_power: f64,
    pub used_devices: Vec<u32>,
}

#[derive(Clone)]
pub struct ClickHouseStore {
    client: Client,
    database: String,
}

impl ClickHouseStore {
    pub fn new(url: &str, database: &str, user: &str, password: &str) -> Self {
        let client = Client::default()
            .with_url(url)
            .with_database(database)
            .with_user(user)
            .with_password(password);

        Self {
            client,
            database: database.to_string(),
        }
    }

    pub async fn insert_sensor_reading(&self, reading: &SensorReading) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client.insert("sensor_data")?;
        inserter.write(reading).await?;
        inserter.end().await?;
        Ok(())
    }

    pub async fn insert_resonance_analysis(
        &self,
        analysis: &ResonanceAnalysisResult,
    ) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client.insert("resonance_analysis")?;
        inserter.write(analysis).await?;
        inserter.end().await?;
        Ok(())
    }

    pub async fn insert_localization(
        &self,
        loc: &SourceLocalizationResult,
    ) -> Result<(), clickhouse::error::Error> {
        let tdoa_str = serde_json::to_string(&loc.tdoa_matrix).unwrap_or_default();

        let row = LocRow {
            timestamp: loc.timestamp,
            source_id: loc.source_id,
            source_x: loc.source_x,
            source_y: loc.source_y,
            source_z: loc.source_z,
            bearing_angle: loc.bearing_angle,
            elevation_angle: loc.elevation_angle,
            distance_estimate: loc.distance_estimate,
            confidence: loc.confidence,
            tdoa_matrix: tdoa_str,
            beamformed_power: loc.beamformed_power,
            used_devices: loc.used_devices.clone(),
        };

        let mut inserter = self.client.insert("source_localization")?;
        inserter.write(&row).await?;
        inserter.end().await?;
        Ok(())
    }

    pub async fn insert_alert(&self, alert: &Alert) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client.insert("alerts")?;
        inserter.write(alert).await?;
        inserter.end().await?;
        Ok(())
    }

    pub async fn get_devices(&self) -> Result<Vec<UrnDevice>, clickhouse::error::Error> {
        let query = "SELECT device_id, device_name, deployment_x, deployment_y, deployment_z,
                     urn_volume, neck_radius, neck_length FROM urn_devices";

        let devices = self.client.query(query).fetch_all::<UrnDevice>().await?;
        Ok(devices)
    }

    pub async fn get_recent_sensor_data(
        &self,
        device_id: Option<u32>,
        limit: u32,
    ) -> Result<Vec<SensorReading>, clickhouse::error::Error> {
        let query = if let Some(id) = device_id {
            format!(
                "SELECT timestamp, device_id, sound_pressure_level, resonance_frequency,
                 source_direction, medium_density, temperature, humidity
                 FROM sensor_data WHERE device_id = {}
                 ORDER BY timestamp DESC LIMIT {}",
                id, limit
            )
        } else {
            format!(
                "SELECT timestamp, device_id, sound_pressure_level, resonance_frequency,
                 source_direction, medium_density, temperature, humidity
                 FROM sensor_data ORDER BY timestamp DESC LIMIT {}",
                limit
            )
        };

        let readings = self.client.query(&query).fetch_all::<SensorReading>().await?;
        Ok(readings)
    }

    pub async fn get_medium_properties(&self) -> Result<Vec<MediumProperty>, clickhouse::error::Error> {
        let query = "SELECT medium_type, display_name, density, sound_speed, attenuation_coeff,
                     depth_start, thickness FROM medium_properties";
        let props = self.client.query(query).fetch_all::<MediumProperty>().await?;
        Ok(props)
    }

    pub async fn get_recent_alerts(&self, limit: u32) -> Result<Vec<Alert>, clickhouse::error::Error> {
        let query = format!(
            "SELECT timestamp, alert_id, alert_type, severity, device_id, message, details, is_resolved
             FROM alerts ORDER BY timestamp DESC LIMIT {}",
            limit
        );
        let alerts = self.client.query(&query).fetch_all::<Alert>().await?;
        Ok(alerts)
    }

    pub async fn get_recent_localizations(
        &self,
        limit: u32,
    ) -> Result<Vec<SourceLocalizationResult>, clickhouse::error::Error> {
        let query = format!(
            "SELECT timestamp, source_id, source_x, source_y, source_z, bearing_angle,
             elevation_angle, distance_estimate, confidence, tdoa_matrix, beamformed_power,
             used_devices FROM source_localization ORDER BY timestamp DESC LIMIT {}",
            limit
        );

        let rows = self.client.query(&query).fetch_all::<LocRow>().await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let tdoa: Vec<Vec<f64>> = serde_json::from_str(&row.tdoa_matrix).unwrap_or_default();
            results.push(SourceLocalizationResult {
                timestamp: row.timestamp,
                source_id: row.source_id,
                source_x: row.source_x,
                source_y: row.source_y,
                source_z: row.source_z,
                bearing_angle: row.bearing_angle,
                elevation_angle: row.elevation_angle,
                distance_estimate: row.distance_estimate,
                confidence: row.confidence,
                tdoa_matrix: tdoa,
                beamformed_power: row.beamformed_power,
                used_devices: row.used_devices,
            });
        }
        Ok(results)
    }
}

pub type SharedStore = Arc<Mutex<ClickHouseStore>>;
