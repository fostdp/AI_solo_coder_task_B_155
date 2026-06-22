CREATE DATABASE IF NOT EXISTS urn_acoustics
    COMMENT '古代瓮听声学共振与声源定位数据库'
    ENGINE = Atomic;

USE urn_acoustics;

CREATE TABLE IF NOT EXISTS urn_devices
(
    device_id UInt32,
    device_name String,
    deployment_x Float64,
    deployment_y Float64,
    deployment_z Float64,
    urn_volume Float64,
    neck_radius Float64,
    neck_length Float64,
    installed_at DateTime DEFAULT now()
)
ENGINE = MergeTree
ORDER BY device_id
COMMENT '瓮听设备元信息表';

CREATE TABLE IF NOT EXISTS sensor_data
(
    timestamp DateTime64(3, 'Asia/Shanghai'),
    device_id UInt32,
    sound_pressure_level Float64,
    resonance_frequency Float64,
    source_direction Float64,
    medium_density Float64,
    temperature Float64,
    humidity Float64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (device_id, timestamp)
TTL timestamp + INTERVAL 90 DAY
COMMENT '传感器实时数据表 - 每分钟上报';

CREATE TABLE IF NOT EXISTS resonance_analysis
(
    timestamp DateTime64(3, 'Asia/Shanghai'),
    device_id UInt32,
    measured_resonance_freq Float64,
    theoretical_resonance_freq Float64,
    gain_db Float64,
    quality_factor Float64,
    frequency_drift Float64,
    drift_percent Float64,
    is_anomaly Bool
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (device_id, timestamp)
TTL timestamp + INTERVAL 90 DAY
COMMENT '声学共振分析结果表 - Helmholtz共振腔理论';

CREATE TABLE IF NOT EXISTS source_localization
(
    timestamp DateTime64(3, 'Asia/Shanghai'),
    source_id UInt64,
    source_x Float64,
    source_y Float64,
    source_z Float64,
    bearing_angle Float64,
    elevation_angle Float64,
    distance_estimate Float64,
    confidence Float64,
    tdoa_matrix String,
    beamformed_power Float64,
    used_devices Array(UInt32)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (timestamp, source_id)
TTL timestamp + INTERVAL 90 DAY
COMMENT '声源定位结果表 - 波束形成算法';

CREATE TABLE IF NOT EXISTS alerts
(
    timestamp DateTime64(3, 'Asia/Shanghai'),
    alert_id UUID DEFAULT generateUUIDv4(),
    alert_type String,
    severity String,
    device_id Nullable(UInt32),
    message String,
    details String,
    is_resolved Bool DEFAULT false
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (timestamp, alert_type)
TTL timestamp + INTERVAL 180 DAY
COMMENT '系统告警表';

CREATE TABLE IF NOT EXISTS medium_properties
(
    medium_type String,
    display_name String,
    density Float64,
    sound_speed Float64,
    attenuation_coeff Float64,
    depth_start Float64,
    thickness Float64
)
ENGINE = ReplacingMergeTree
ORDER BY medium_type
COMMENT '地下介质声学参数表';

INSERT INTO medium_properties (medium_type, display_name, density, sound_speed, attenuation_coeff, depth_start, thickness) VALUES
('dry_sand', '干燥沙', 1600.0, 300.0, 0.5, 0.0, 5.0),
('wet_sand', '湿润沙', 1900.0, 500.0, 0.3, 5.0, 15.0),
('clay', '黏土', 2200.0, 1800.0, 0.15, 20.0, 30.0),
('limestone', '石灰岩', 2500.0, 3500.0, 0.05, 50.0, 50.0),
('granite', '花岗岩', 2700.0, 4500.0, 0.03, 100.0, 100.0),
('water_saturated_soil', '饱和水土', 2200.0, 1800.0, 0.15, 150.0, 50.0);

CREATE MATERIALIZED VIEW IF NOT EXISTS sensor_data_1min_mv
ENGINE = SummingMergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (device_id, toStartOfMinute(timestamp))
AS SELECT
    toStartOfMinute(timestamp) AS timestamp,
    device_id,
    count() AS sample_count,
    avg(sound_pressure_level) AS avg_spl,
    max(sound_pressure_level) AS max_spl,
    min(sound_pressure_level) AS min_spl,
    avg(resonance_frequency) AS avg_res_freq,
    avg(medium_density) AS avg_density
FROM sensor_data
GROUP BY device_id, toStartOfMinute(timestamp);

CREATE TABLE IF NOT EXISTS sensor_data_1h_agg
(
    timestamp DateTime('Asia/Shanghai'),
    device_id UInt32,
    sample_count UInt64,
    avg_spl Float64,
    max_spl Float64,
    min_spl Float64,
    avg_res_freq Float64,
    avg_density Float64,
    stddev_spl Float64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (device_id, timestamp)
TTL timestamp + INTERVAL 1 YEAR
COMMENT '传感器数据1小时聚合 - 用于长期趋势分析';

CREATE MATERIALIZED VIEW IF NOT EXISTS sensor_data_1h_mv
TO sensor_data_1h_agg
AS SELECT
    toStartOfHour(timestamp) AS timestamp,
    device_id,
    count() AS sample_count,
    avg(sound_pressure_level) AS avg_spl,
    max(sound_pressure_level) AS max_spl,
    min(sound_pressure_level) AS min_spl,
    avg(resonance_frequency) AS avg_res_freq,
    avg(medium_density) AS avg_density,
    stddevPop(sound_pressure_level) AS stddev_spl
FROM sensor_data
GROUP BY device_id, toStartOfHour(timestamp);

CREATE TABLE IF NOT EXISTS resonance_analysis_1d_agg
(
    day Date,
    device_id UInt32,
    reading_count UInt64,
    avg_gain_db Float64,
    avg_drift_percent Float64,
    max_drift_percent Float64,
    anomaly_count UInt64,
    avg_q_factor Float64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(day)
ORDER BY (device_id, day)
TTL day + INTERVAL 2 YEAR
COMMENT '共振分析日聚合表';

CREATE MATERIALIZED VIEW IF NOT EXISTS resonance_analysis_1d_mv
TO resonance_analysis_1d_agg
AS SELECT
    toDate(timestamp) AS day,
    device_id,
    count() AS reading_count,
    avg(gain_db) AS avg_gain_db,
    avg(drift_percent) AS avg_drift_percent,
    max(drift_percent) AS max_drift_percent,
    countIf(is_anomaly) AS anomaly_count,
    avg(quality_factor) AS avg_q_factor
FROM resonance_analysis
GROUP BY device_id, toDate(timestamp);

CREATE TABLE IF NOT EXISTS alerts_archive
(
    timestamp DateTime64(3, 'Asia/Shanghai'),
    alert_id UUID,
    alert_type String,
    severity String,
    device_id Nullable(UInt32),
    message String,
    details String,
    is_resolved Bool,
    archived_at DateTime DEFAULT now()
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (severity, timestamp)
TTL timestamp + INTERVAL 2 YEAR
COMMENT '告警归档表 - 历史告警长期保存';
