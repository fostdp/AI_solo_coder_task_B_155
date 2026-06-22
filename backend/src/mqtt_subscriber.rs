use crate::acoustics::AcousticAnalyzer;
use crate::alerts::AlertManager;
use crate::config::MqttConfig;
use crate::localization::Beamformer;
use crate::models::{SensorReading, UrnDevice, SourceLocalizationResult, ResonanceAnalysisResult};
use crate::store::ClickHouseStore;
use dashmap::DashMap;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

pub struct MqttSubscriber {
    store: ClickHouseStore,
    analyzer: AcousticAnalyzer,
    beamformer: Beamformer,
    alert_manager: Arc<AlertManager>,
    devices: Arc<DashMap<u32, UrnDevice>>,
    recent_readings: Arc<DashMap<u32, Vec<SensorReading>>>,
    source_id_counter: Arc<Mutex<u64>>,
}

impl MqttSubscriber {
    pub fn new(
        store: ClickHouseStore,
        analyzer: AcousticAnalyzer,
        beamformer: Beamformer,
        alert_manager: Arc<AlertManager>,
        devices: Arc<DashMap<u32, UrnDevice>>,
    ) -> Self {
        Self {
            store,
            analyzer,
            beamformer,
            alert_manager,
            devices,
            recent_readings: Arc::new(DashMap::new()),
            source_id_counter: Arc::new(Mutex::new(1)),
        }
    }

    pub async fn run(self: Arc<Self>, config: &MqttConfig) {
        let mut options = MqttOptions::new(&config.client_id, &config.broker, config.port);
        options.set_keep_alive(Duration::from_secs(60));
        options.set_pending_throttle(Duration::from_millis(100));

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            options.set_credentials(username, password);
        }

        let (client, mut eventloop) = AsyncClient::new(options, 100);

        match client.subscribe(&config.topic, QoS::AtLeastOnce).await {
            Ok(()) => info!("已订阅MQTT主题: {}", config.topic),
            Err(e) => {
                error!("MQTT订阅失败: {}", e);
                return;
            }
        }

        info!("MQTT客户端已启动，broker: {}:{}", config.broker, config.port);

        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    let payload = &publish.payload;
                    if let Ok(reading) = serde_json::from_slice::<SensorReading>(payload) {
                        self.process_reading(reading).await;
                    } else {
                        warn!("无法解析MQTT消息: {:?}", String::from_utf8_lossy(payload));
                    }
                }
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT连接已确认");
                }
                Ok(_event) => {
                    debug!("MQTT事件: {:?}", _event);
                }
                Err(e) => {
                    error!("MQTT连接错误: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn process_reading(&self, reading: SensorReading) {
        let device_id = reading.device_id;

        if let Some(device) = self.devices.get(&device_id) {
            if let Err(e) = self.store.insert_sensor_reading(&reading).await {
                error!("存储传感器数据失败: {}", e);
                return;
            }

            self.alert_manager.broadcast_sensor_data(&reading);

            let analysis = self.analyzer.analyze(&reading, &device);
            self.process_analysis(analysis.clone()).await;
            self.alert_manager.broadcast_resonance(&analysis);

            if let Some(alert) = self.alert_manager.check_resonance(&analysis) {
                if let Err(e) = self.store.insert_alert(&alert).await {
                    error!("存储告警失败: {}", e);
                }
            }

            self.add_recent_reading(reading);
            self.try_localize().await;
        } else {
            warn!("收到未知设备 {} 的数据", device_id);
        }
    }

    async fn process_analysis(&self, analysis: ResonanceAnalysisResult) {
        if let Err(e) = self.store.insert_resonance_analysis(&analysis).await {
            error!("存储共振分析结果失败: {}", e);
        }
    }

    fn add_recent_reading(&self, reading: SensorReading) {
        let mut entry = self.recent_readings
            .entry(reading.device_id)
            .or_insert_with(Vec::new);

        entry.push(reading);
        if entry.len() > 10 {
            entry.remove(0);
        }
    }

    async fn try_localize(&self) {
        let active_device_ids: Vec<u32> = self.recent_readings
            .iter()
            .filter(|entry| entry.value().len() >= 1)
            .map(|entry| *entry.key())
            .collect();

        if active_device_ids.len() < 3 {
            return;
        }

        let mut readings_with_devices = Vec::new();

        for device_id in &active_device_ids {
            if let Some(device) = self.devices.get(device_id) {
                if let Some(readings) = self.recent_readings.get(device_id) {
                    if let Some(latest) = readings.value().last() {
                        readings_with_devices.push((latest.clone(), device.clone()));
                    }
                }
            }
        }

        if readings_with_devices.len() >= 3 {
            let mut counter = self.source_id_counter.lock().await;
            let source_id = *counter;
            *counter += 1;
            drop(counter);

            if let Some(localization) = self.beamformer.locate_source(&readings_with_devices, source_id) {
                self.process_localization(localization.clone()).await;
                self.alert_manager.broadcast_localization(&localization);

                if let Some(alert) = self.alert_manager.check_localization(&localization) {
                    if let Err(e) = self.store.insert_alert(&alert).await {
                        error!("存储告警失败: {}", e);
                    }
                }
            }
        }
    }

    async fn process_localization(&self, loc: SourceLocalizationResult) {
        if let Err(e) = self.store.insert_localization(&loc).await {
            error!("存储定位结果失败: {}", e);
        }
    }
}
