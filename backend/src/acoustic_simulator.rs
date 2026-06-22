use crate::acoustics::{AcousticAnalyzer, BEMCorrection, HelmholtzResonator, UrnShape};
use crate::config_loader::AcousticsConfig;
use crate::metrics;
use crate::models::ResonanceAnalysisResult;
use crate::pipeline::{AcousticJobResult, ValidSensorReading};
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct AcousticSimulator {
    analyzer: AcousticAnalyzer,
    acoustics_cfg: AcousticsConfig,
}

impl AcousticSimulator {
    pub fn new(cfg: AcousticsConfig) -> Arc<Self> {
        let analyzer = AcousticAnalyzer::new(
            cfg.speed_of_sound_air,
            cfg.drift_warning_threshold_percent,
            cfg.drift_critical_threshold_percent,
        )
        .with_bem(cfg.bem_enabled);

        Arc::new(Self {
            analyzer,
            acoustics_cfg: cfg,
        })
    }

    pub async fn run(
        self: Arc<Self>,
        mut rx_valid: mpsc::Receiver<ValidSensorReading>,
        tx_locator: mpsc::Sender<AcousticJobResult>,
        tx_alarm: mpsc::Sender<AcousticJobResult>,
        tx_db: mpsc::Sender<ResonanceAnalysisResult>,
    ) {
        info!("[AcousticSimulator] 声学仿真服务启动");
        let mut processed = 0u64;

        while let Some(valid) = rx_valid.recv().await {
            let result = self.analyze(&valid);
            processed += 1;

            if let Err(_e) = tx_db.send(result.analysis.clone()).await {
                warn!("[AcousticSimulator] 分析结果入库通道已满");
            }

            if let Err(_e) = tx_locator.send(result.clone()).await {
                warn!("[AcousticSimulator] 向 SourceLocator 发送结果失败");
            }
            if let Err(_e) = tx_alarm.send(result).await {
                warn!("[AcousticSimulator] 向 AlarmWsService 发送结果失败");
            }

            if processed % 100 == 0 {
                debug!("[AcousticSimulator] 已处理 {} 条读数", processed);
            }
        }

        error!("[AcousticSimulator] 输入通道关闭，仿真服务退出");
    }

    fn analyze(&self, valid: &ValidSensorReading) -> AcousticJobResult {
        let reading = &valid.reading;
        let device = &valid.device;

        metrics::inc_acoustic_calcs();

        let mut resonator = HelmholtzResonator::from_device(device, self.acoustics_cfg.speed_of_sound_air);

        resonator = resonator.with_shape(self.parse_shape(&self.acoustics_cfg.default_shape));
        resonator = resonator.with_rim_flange(self.acoustics_cfg.default_rim_flange_width);

        let theoretical_freq = if self.analyzer.use_bem() {
            let bem = BEMCorrection::new(&resonator);
            bem.bem_corrected_resonance_freq(&resonator)
        } else {
            resonator.resonance_frequency()
        };

        let measured_freq = reading.resonance_frequency;
        let drift = measured_freq - theoretical_freq;
        let drift_percent = (drift / theoretical_freq).abs() * 100.0;

        let gain_db = if self.analyzer.use_bem() {
            let bem = BEMCorrection::new(&resonator);
            let base_gain = bem.bem_corrected_gain(&resonator, measured_freq);
            base_gain + (reading.medium_density / 1600.0).ln() * 2.5
        } else {
            resonator.finite_element_gain_correction(measured_freq, reading.medium_density)
        };

        let q_factor = resonator.quality_factor();
        let is_anomaly = drift_percent > self.acoustics_cfg.drift_warning_threshold_percent;

        metrics::observe_resonance_drift(drift_percent);

        let analysis = ResonanceAnalysisResult {
            timestamp: Utc::now(),
            device_id: reading.device_id,
            measured_resonance_freq: measured_freq,
            theoretical_resonance_freq: theoretical_freq,
            gain_db,
            quality_factor: q_factor,
            frequency_drift: drift,
            drift_percent,
            is_anomaly,
        };

        AcousticJobResult {
            reading: reading.clone(),
            device: device.clone(),
            analysis,
        }
    }

    fn parse_shape(&self, s: &str) -> UrnShape {
        match s {
            "Spherical" | "spherical" => UrnShape::Spherical,
            "Cylindrical" | "cylindrical" => UrnShape::Cylindrical,
            "Ellipsoidal" | "ellipsoidal" => UrnShape::Ellipsoidal,
            "Irregular" | "irregular" => UrnShape::Irregular,
            _ => {
                warn!("[AcousticSimulator] 未知形状 {}, 回退为 Spherical", s);
                UrnShape::Spherical
            }
        }
    }

    pub fn analyzer(&self) -> &AcousticAnalyzer {
        &self.analyzer
    }
}
