use crate::config_loader::LocalizationConfig;
use crate::localization::Beamformer;
use crate::metrics;
use crate::models::{SensorReading, SourceLocalizationResult, UrnDevice};
use crate::pipeline::{AcousticJobResult, LocalizationJobResult};
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

struct LocatorState {
    recent: DashMap<u32, Vec<(SensorReading, UrnDevice)>>,
    source_counter: StdMutex<u64>,
    recent_capacity: usize,
    min_devices: usize,
}

pub struct SourceLocator {
    beamformer: Beamformer,
    config: LocalizationConfig,
}

impl SourceLocator {
    pub fn new(config: LocalizationConfig) -> Arc<Self> {
        let beamformer = Beamformer::new(
            config.sound_speed_soil,
            config.beamforming_resolution,
            config.max_localization_distance,
            config.localization_confidence_threshold,
        )
        .with_method(config.beamforming_method)
        .with_diagonal_loading(config.diagonal_loading)
        .with_multipath_suppression(config.multipath_suppression);

        Arc::new(Self { beamformer, config })
    }

    pub async fn run(
        self: Arc<Self>,
        mut rx_acoustic: mpsc::Receiver<AcousticJobResult>,
        tx_alarm: mpsc::Sender<LocalizationJobResult>,
        tx_db: mpsc::Sender<SourceLocalizationResult>,
    ) {
        info!(
            "[SourceLocator] 声源定位服务启动 方法={:?} 多径抑制={}",
            self.config.beamforming_method, self.config.multipath_suppression
        );

        let state = Arc::new(LocatorState {
            recent: DashMap::new(),
            source_counter: StdMutex::new(1),
            recent_capacity: self.config.recent_readings_per_device,
            min_devices: self.config.min_active_devices,
        });

        let mut located = 0u64;

        while let Some(job) = rx_acoustic.recv().await {
            Self::push_reading(&state, &job);

            if let Some(result) = Self::try_locate(&state, &self.beamformer) {
                located += 1;
                let device_ids = result.used_devices.clone();
                let job_result = LocalizationJobResult {
                    result: result.clone(),
                    contributing_devices: device_ids,
                };

                if let Err(_e) = tx_db.send(result).await {
                    warn!("[SourceLocator] 定位结果入库通道已满");
                }
                if let Err(_e) = tx_alarm.send(job_result).await {
                    warn!("[SourceLocator] 向 AlarmWsService 发送定位结果失败");
                }

                if located % 20 == 0 {
                    debug!("[SourceLocator] 已定位 {} 次", located);
                }
            }
        }

        error!("[SourceLocator] 输入通道关闭，定位服务退出");
    }

    fn push_reading(state: &Arc<LocatorState>, job: &AcousticJobResult) {
        let mut entry = state
            .recent
            .entry(job.reading.device_id)
            .or_insert_with(Vec::new);

        entry.push((job.reading.clone(), job.device.clone()));
        if entry.len() > state.recent_capacity {
            entry.remove(0);
        }
    }

    fn try_locate(
        state: &Arc<LocatorState>,
        beamformer: &Beamformer,
    ) -> Option<SourceLocalizationResult> {
        let active_ids: Vec<u32> = state
            .recent
            .iter()
            .filter(|e| !e.value().is_empty())
            .map(|e| *e.key())
            .collect();

        if active_ids.len() < state.min_devices {
            return None;
        }

        let mut readings_devices: Vec<(SensorReading, UrnDevice)> = Vec::new();
        for id in &active_ids {
            if let Some(entry) = state.recent.get(id) {
                if let Some(latest) = entry.value().last() {
                    readings_devices.push(latest.clone());
                }
            }
        }

        if readings_devices.len() < state.min_devices {
            return None;
        }

        let mut counter = state.source_counter.lock().unwrap();
        let source_id = *counter;
        *counter += 1;
        drop(counter);

        metrics::inc_localizations();

        let result = beamformer.locate_source(&readings_devices, source_id);
        if let Some(ref loc) = result {
            metrics::observe_localization_confidence(loc.confidence);
        }
        result
    }

    pub fn beamformer(&self) -> &Beamformer {
        &self.beamformer
    }
}
