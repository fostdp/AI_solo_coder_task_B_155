use crate::models::{ResonanceAnalysisResult, SensorReading, SourceLocalizationResult, UrnDevice};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum PipelineMessage {
    ValidReading(ValidSensorReading),
    AcousticResult(AcousticJobResult),
    LocalizationResult(LocalizationJobResult),
    Tick,
}

#[derive(Debug, Clone)]
pub struct ValidSensorReading {
    pub reading: SensorReading,
    pub device: UrnDevice,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct AcousticJobResult {
    pub reading: SensorReading,
    pub device: UrnDevice,
    pub analysis: ResonanceAnalysisResult,
}

#[derive(Debug, Clone)]
pub struct LocalizationJobResult {
    pub result: SourceLocalizationResult,
    pub contributing_devices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

pub fn validate_reading(reading: &SensorReading) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if reading.device_id == 0 {
        errors.push(ValidationError {
            field: "device_id".to_string(),
            message: "device_id 必须大于 0".to_string(),
        });
    }

    if !(0.0..=200.0).contains(&reading.sound_pressure_level) {
        errors.push(ValidationError {
            field: "sound_pressure_level".to_string(),
            message: format!(
                "声压级应在 0~200 dB 范围内，实际为 {:.1}",
                reading.sound_pressure_level
            ),
        });
    }

    if !(10.0..=10000.0).contains(&reading.resonance_frequency) {
        errors.push(ValidationError {
            field: "resonance_frequency".to_string(),
            message: format!(
                "共振频率应在 10~10000 Hz 范围内，实际为 {:.1}",
                reading.resonance_frequency
            ),
        });
    }

    if !(0.0..=360.0).contains(&reading.source_direction) {
        errors.push(ValidationError {
            field: "source_direction".to_string(),
            message: format!(
                "声源方向应在 0~360° 范围内，实际为 {:.1}",
                reading.source_direction
            ),
        });
    }

    if !(500.0..=5000.0).contains(&reading.medium_density) {
        errors.push(ValidationError {
            field: "medium_density".to_string(),
            message: format!(
                "介质密度应在 500~5000 kg/m³ 范围内，实际为 {:.0}",
                reading.medium_density
            ),
        });
    }

    if !(-50.0..=80.0).contains(&reading.temperature) {
        errors.push(ValidationError {
            field: "temperature".to_string(),
            message: format!(
                "温度应在 -50~80°C 范围内，实际为 {:.1}",
                reading.temperature
            ),
        });
    }

    if !(0.0..=100.0).contains(&reading.humidity) {
        errors.push(ValidationError {
            field: "humidity".to_string(),
            message: format!(
                "湿度应在 0~100% 范围内，实际为 {:.0}",
                reading.humidity
            ),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn valid_reading() -> SensorReading {
        SensorReading {
            timestamp: Utc::now(),
            device_id: 1,
            sound_pressure_level: 80.0,
            resonance_frequency: 200.0,
            source_direction: 180.0,
            medium_density: 1800.0,
            temperature: 20.0,
            humidity: 50.0,
        }
    }

    #[test]
    fn test_validate_valid_reading() {
        let r = valid_reading();
        assert!(validate_reading(&r).is_ok());
    }

    #[test]
    fn test_validate_invalid_spl() {
        let mut r = valid_reading();
        r.sound_pressure_level = 300.0;
        let errs = validate_reading(&r).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "sound_pressure_level");
    }

    #[test]
    fn test_validate_multiple_errors() {
        let mut r = valid_reading();
        r.device_id = 0;
        r.humidity = 150.0;
        let errs = validate_reading(&r).unwrap_err();
        assert_eq!(errs.len(), 2);
    }
}
