use crate::metrics;
use crate::models::{Alert, ResonanceAnalysisResult, SensorReading, SourceLocalizationResult};
use crate::store::ClickHouseStore;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct DbWriter {
    store: ClickHouseStore,
}

impl DbWriter {
    pub fn new(store: ClickHouseStore) -> Arc<Self> {
        Arc::new(Self { store })
    }

    pub async fn run(
        self: Arc<Self>,
        mut rx_sensor: mpsc::Receiver<SensorReading>,
        mut rx_resonance: mpsc::Receiver<ResonanceAnalysisResult>,
        mut rx_localization: mpsc::Receiver<SourceLocalizationResult>,
        mut rx_alert: mpsc::Receiver<Alert>,
    ) {
        info!("[DbWriter] 数据库持久化 worker 启动");
        let mut counts = (0u64, 0u64, 0u64, 0u64);

        loop {
            tokio::select! {
                Some(r) = rx_sensor.recv() => {
                    if let Err(e) = self.store.insert_sensor_reading(&r).await {
                        warn!("[DbWriter] 写入 sensor_data 失败: {}", e);
                    } else {
                        counts.0 += 1;
                        metrics::inc_db_writes("sensor_data");
                    }
                }
                Some(a) = rx_resonance.recv() => {
                    if let Err(e) = self.store.insert_resonance_analysis(&a).await {
                        warn!("[DbWriter] 写入 resonance_analysis 失败: {}", e);
                    } else {
                        counts.1 += 1;
                        metrics::inc_db_writes("resonance_analysis");
                    }
                }
                Some(l) = rx_localization.recv() => {
                    if let Err(e) = self.store.insert_localization(&l).await {
                        warn!("[DbWriter] 写入 source_localization 失败: {}", e);
                    } else {
                        counts.2 += 1;
                        metrics::inc_db_writes("source_localization");
                    }
                }
                Some(al) = rx_alert.recv() => {
                    if let Err(e) = self.store.insert_alert(&al).await {
                        warn!("[DbWriter] 写入 alerts 失败: {}", e);
                    } else {
                        counts.3 += 1;
                        metrics::inc_db_writes("alerts");
                    }
                }
                else => {
                    error!("[DbWriter] 所有输入通道已关闭，持久化 worker 退出");
                    break;
                }
            }

            let total = counts.0 + counts.1 + counts.2 + counts.3;
            if total > 0 && total % 100 == 0 {
                debug!(
                    "[DbWriter] 已写入: sensor={} resonance={} loc={} alert={}",
                    counts.0, counts.1, counts.2, counts.3
                );
            }
        }
    }
}
