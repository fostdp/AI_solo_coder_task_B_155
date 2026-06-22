use crate::config_loader::MqttConfig;
use crate::metrics;
use crate::models::{SensorReading, UrnDevice};
use crate::pipeline::{validate_reading, ValidSensorReading};
use dashmap::DashMap;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct MqttReceiver {
    devices: Arc<DashMap<u32, UrnDevice>>,
    config: MqttConfig,
}

impl MqttReceiver {
    pub fn new(devices: Arc<DashMap<u32, UrnDevice>>, config: MqttConfig) -> Self {
        Self { devices, config }
    }

    pub async fn run(
        self: Arc<Self>,
        tx_acoustic: mpsc::Sender<ValidSensorReading>,
        tx_alarm: mpsc::Sender<ValidSensorReading>,
        tx_raw_db: mpsc::Sender<SensorReading>,
    ) {
        let mut options = MqttOptions::new(&self.config.client_id, &self.config.broker, self.config.port);
        options.set_keep_alive(Duration::from_secs(self.config.keep_alive_secs));
        options.set_pending_throttle(Duration::from_millis(100));

        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            options.set_credentials(username, password);
        }

        let (client, mut eventloop) = AsyncClient::new(options, 100);

        match client.subscribe(&self.config.topic, QoS::AtLeastOnce).await {
            Ok(()) => info!("[MQTT] 已订阅主题: {}", self.config.topic),
            Err(e) => {
                error!("[MQTT] 订阅失败: {}", e);
                return;
            }
        }

        info!(
            "[MQTT] 接收器已启动 broker={}:{}",
            self.config.broker, self.config.port
        );

        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    self.handle_payload(&publish.payload, &tx_acoustic, &tx_alarm, &tx_raw_db).await;
                }
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("[MQTT] 连接已确认");
                }
                Ok(_event) => {
                    debug!("[MQTT] 其他事件: {:?}", _event);
                }
                Err(e) => {
                    error!("[MQTT] 连接错误: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn handle_payload(
        &self,
        payload: &[u8],
        tx_acoustic: &mpsc::Sender<ValidSensorReading>,
        tx_alarm: &mpsc::Sender<ValidSensorReading>,
        tx_raw_db: &mpsc::Sender<SensorReading>,
    ) {
        metrics::inc_mqtt_messages();

        let reading: SensorReading = match serde_json::from_slice(payload) {
            Ok(r) => r,
            Err(e) => {
                metrics::inc_invalid_readings();
                warn!(
                    "[MQTT] JSON 解析失败: {} 原始数据前100字节: {:?}",
                    e,
                    payload.iter().take(100).copied().collect::<Vec<u8>>()
                );
                return;
            }
        };

        metrics::observe_spl(reading.sound_pressure_level);

        if let Err(_e) = tx_raw_db.send(reading.clone()).await {
            warn!("[MQTT] 原始数据入库通道已满，丢弃数据 device_id={}", reading.device_id);
        }

        match validate_reading(&reading) {
            Ok(()) => {}
            Err(errs) => {
                metrics::inc_invalid_readings();
                warn!(
                    "[MQTT] 数据校验失败 device_id={}, 错误: {:?}",
                    reading.device_id, errs
                );
                return;
            }
        }

        let device = match self.devices.get(&reading.device_id) {
            Some(d) => d.value().clone(),
            None => {
                warn!("[MQTT] 收到未知设备 {} 的数据，丢弃", reading.device_id);
                return;
            }
        };

        let valid = ValidSensorReading {
            reading,
            device,
            received_at: chrono::Utc::now(),
        };

        if let Err(_e) = tx_acoustic.send(valid.clone()).await {
            warn!("[MQTT] 向 AcousticSimulator 发送数据失败，通道关闭或已满");
        }
        if let Err(_e) = tx_alarm.send(valid).await {
            warn!("[MQTT] 向 AlarmWsService 发送数据失败，通道关闭或已满");
        }
    }
}
