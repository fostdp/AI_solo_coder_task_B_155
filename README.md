# 古代瓮听声学共振与声源定位仿真系统

> 古代兵法"地听/瓮听"的现代声学复刻系统：基于 **Helmholtz 共振腔 + BEM 边界元修正** 做声学分析，基于 **DAS / MVDR / MUSIC** 波束形成做声源定位，配合 ClickHouse 时序存储、MQTT 设备接入、WebSocket 实时告警。

---

## 📐 一、系统架构

### 1.1 架构总览图

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│  仿真/数据输入层                                                                          │
│  ┌─────────────────┐   MQTT (urn/sensors/#)    ┌──────────────────────────────────────┐ │
│  │  传感器模拟器    │ ───────────────────────► │  Mosquitto Broker (1883 / 9001)     │ │
│  │  (Python)       │                           │  匿名 / 可配置用户密码               │ │
│  │  ·声源距离可调   │                           └──────────────┬───────────────────────┘ │
│  │  ·5种介质预设   │                                          │                         │
│  └─────────────────┘                                          ▼                         │
│                                                    ┌──────────────────┐                  │
│  多线程 Pipeline (Rust tokio mpsc 扇出)            │   mqtt_receiver  │ (采集+7项校验)    │
│  ┌──────────────────────────────────────────┐      └────────┬─────────┘                  │
│  │                                            │              │                            │
│  │   Valid ─► acoustic_simulator ─► Locator ─┤      ┌───────▼──────────┐                 │
│  │     │        (Helmholtz+BEM+增益+漂移)    │      │   acoustic_sim    │ (共振+增益)      │
│  │     │               │                     │      └───────┬──────────┘                 │
│  │     ▼               ▼                     │              │                            │
│  │  alarm_ws ◄──── source_locator            │      ┌───────▼───────────┐                │
│  │  (告警判定       (DAS/MVDR/MUSIC           │      │   source_locator  │ (波束形成/定位)  │
│  │   +broadcast)     波束形成)                │      └───────┬───────────┘                │
│  │     │                                           ┌───────▼─────────────┐                │
│  │     ▼                                           │     alarm_ws        │ (告警 + WS)     │
│  │  DbWriter (4路输入 → ClickHouse)               └──────────┬────────────┘                │
│  └───────────────────────────────────────────┘               │                            │
│                                                              ▼                            │
│  数据存储层                                                    ClickHouse                   │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐    │
│  │  Tables: sensor_data / resonance_analysis / source_localization / alerts / MV     │    │
│  │  物化视图(降采样): 1min / 1h / 1d 聚合 + TTL 自动清理 (90天/1年/2年)               │    │
│  └──────────────────────────────────────────────────────────────────────────────────┘    │
│                                                                                          │
│  可观测性 + 对外 API + 前端                                                               │
│  ┌──────────────────────────────────────────┐   HTTP/WS 8080     ┌──────────────────┐    │
│  │ Rust Axum: 12×REST + /ws + /metrics      │◄──────────────────►│  浏览器前端       │    │
│  │ ·/api/devices, /api/sensor-data,...       │   JSON + Gzip     │  Three.js 3D     │    │
│  │ ·/metrics (Prometheus 格式)               │                   │  罗盘/射线追踪   │    │
│  │ ·CompressionLayer Gzip + TraceLayer       │                   │  共振曲线/告警   │    │
│  └──────────────────────────────────────────┘                   └──────────────────┘    │
│         │                                                                                │
│         ▼ (端口 9090)                                                                    │
│  Prometheus (可选 monitoring profile)  ◄─ /metrics 抓取 ──┐                               │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Pipeline 扇出 & 数据流向

```
                ┌────────── mpsc ──────────┐
                │                           ▼
MqttReceiver ─Valid──► AcousticSimulator ─Result─► SourceLocator ─Result─► AlarmWs
      │                  │                          │                    │
      ▼                  ▼                          ▼                    ▼
 DbWriter(sensor)  DbWriter(resonance)      DbWriter(localization)  DbWriter(alert)
```

- **mqtt_receiver**：MQTT 订阅 → JSON 反序列化 → 7 项范围校验 → 扇出到 acoustic_simulator 和 alarm_ws
- **acoustic_simulator**：Helmholtz 共振频率 / BEM 边界元修正 / 多模态增益 / Q 值 / 漂移检测 → 扇出到 source_locator 和 alarm_ws
- **source_locator**：DashMap 缓存设备读数 → ≥3 设备触发 DAS / MVDR / MUSIC 波束形成
- **alarm_ws**：`tokio::select!` 三路 mpsc + 频率漂移/定位偏差判定 + 冷却去重 → broadcast channel WebSocket 推送
- **db_writer**：四路输入 → ClickHouse RowBinary 批量写入

### 1.3 模块边界

| 模块 | 文件 | 对外接口 |
|---|---|---|
| 数据采集 + 校验 | [mqtt_receiver.rs](backend/src/mqtt_receiver.rs) | `MqttReceiver::run(tx_acoustic, tx_alarm, tx_raw_db)` |
| 共振腔 + 增益计算 | [acoustic_simulator.rs](backend/src/acoustic_simulator.rs) | `AcousticSimulator::run(rx_valid, tx_locator, tx_alarm, tx_db)` |
| 波束形成 + 定位 | [source_locator.rs](backend/src/source_locator.rs) | `SourceLocator::run(rx_acoustic, tx_alarm, tx_db)` |
| 告警 + WebSocket 推送 | [alarm_ws.rs](backend/src/alarm_ws.rs) | `AlarmWsService::run(rx_sensor, rx_acoustic, rx_locator, tx_alert_db)` |
| ClickHouse 持久化 | [db_writer.rs](backend/src/db_writer.rs) | `DbWriter::run(rx_sensor, rx_res, rx_loc, rx_alert)` |
| JSON 配置加载 | [config_loader.rs](backend/src/config_loader.rs) | `ConfigBundle::load()` 多目录自动搜索 |
| Prometheus 指标 | [metrics.rs](backend/src/metrics.rs) | `render_prometheus()` + 各 worker 调用点埋点 |

前端拆分：

| 文件 | 职责 |
|---|---|
| [ground_listening_3d.js](frontend/ground_listening_3d.js) | Three.js 三维渲染（场景/相机/灯光/地面/瓮听设备/声源标记/发光脉动） |
| [acoustic_panel.js](frontend/acoustic_panel.js) | 面板数据、WebSocket 订阅、声波波纹、罗盘、共振曲线、射线追踪 |

---

## 🗂️ 二、目录结构

```
AI_solo_coder_task_A_155/
├── backend/                         # Rust 后端
│   ├── Cargo.toml                   # 依赖: axum 0.7 / clickhouse 0.11 / rumqttc / tokio / metrics
│   ├── config/                      # JSON 外置配置
│   │   ├── app.json                 # server/mqtt/clickhouse/localization/alert/pipeline
│   │   ├── acoustics.json           # 18 项声学参数 (声速/瓮体积/BEM/漂移阈值)
│   │   └── medium_properties.json   # 4 层地下介质 (沙/黏土/石灰岩等)
│   └── src/
│       ├── main.rs                  # 9 个 tokio::spawn + /metrics + Gzip + TraceLayer
│       ├── metrics.rs               # Prometheus 指标 (counter/gauge/histogram)
│       ├── mqtt_receiver.rs         # MQTT 采集 + 数据校验
│       ├── acoustic_simulator.rs    # Helmholtz + BEM 共振腔分析
│       ├── source_locator.rs        # 波束形成定位
│       ├── alarm_ws.rs              # 告警判定 + broadcast WebSocket 推送
│       ├── db_writer.rs             # ClickHouse 持久化
│       ├── config_loader.rs         # JSON 配置加载
│       ├── pipeline.rs              # 消息类型 + validate_reading()
│       ├── acoustics.rs             # Helmholtz / BEM / 射线追踪数学模型
│       ├── localization.rs          # DAS / MVDR / MUSIC 算法
│       ├── handlers.rs              # 12 个 HTTP 端点
│       ├── store.rs                 # ClickHouse 0.11 Row derive API
│       ├── models.rs                # 数据模型 (所有 Row derive)
│       └── websocket.rs             # axum ws handler
│
├── frontend/                        # 前端 (HTML + Three.js)
│   ├── index.html                   # 入口 (拆分后加载两个 JS)
│   ├── style.css
│   ├── ground_listening_3d.js       # 拆分 1: Three.js 三维渲染
│   └── acoustic_panel.js            # 拆分 2: 面板/可视化/WebSocket/射线追踪
│
├── simulator/                       # 传感器模拟器 (Python)
│   └── urn_sensor_sim.py            # 声源距离 + 5 种介质预设 + 可配置漂移
│
├── docker/                          # Docker 多阶段构建 + 组件配置
│   ├── backend.Dockerfile           # chef → planner → builder → debian-slim (cargo-chef 加速)
│   ├── simulator.Dockerfile         # python:3.12-slim + paho-mqtt
│   ├── clickhouse/config.xml        # ClickHouse 自定义 (内存/合并/网络)
│   ├── mosquitto/mosquitto.conf     # MQTT Broker (1883 + 9001 ws)
│   └── prometheus/prometheus.yml    # Prometheus 抓取 /metrics
│
├── database/
│   └── init.sql                     # 建表 + 降采样 MV + TTL + 介质种子
│
├── docker-compose.yml               # 5 服务编排 (clickhouse/mosquitto/backend/simulator/prometheus)
├── .env.example                     # 环境变量示例
├── .dockerignore
└── README.md                        # 本文档
```

---

## 🚀 三、快速部署 (Docker Compose)

### 3.1 前置条件

- Docker Engine **24+** 与 Docker Compose Plugin **v2.20+**
- 至少 **4 CPU 核心 / 8 GB 内存**（ClickHouse + Rust 编译较重）
- 本地端口可用：`8080, 8123, 9000, 1883, 9001, 9090`

### 3.2 一键启动核心服务

```bash
# 1. 复制环境变量 (按需修改)
cp .env.example .env

# 2. 构建 + 启动 ClickHouse + MQTT + Rust 后端
docker compose up -d --build clickhouse mosquitto backend

# 3. 查看各服务健康状态
docker compose ps
# NAME              STATUS         PORTS
# urn-clickhouse    healthy (30s)  0.0.0.0:8123,9000
# urn-mosquitto     healthy (10s)  0.0.0.0:1883,9001
# urn-backend       starting (90s) 0.0.0.0:8080

# 4. 启动传感器模拟器 (可选, 自动发数据)
docker compose --profile simulator up -d simulator

# 5. (可选) 启动 Prometheus 抓取 /metrics
docker compose --profile monitoring up -d prometheus
```

### 3.3 访问地址

| 服务 | URL | 说明 |
|---|---|---|
| **前端 3D 控制台** | http://localhost:8080/ | Rust ServeDir 托管，Gzip 压缩 |
| **后端健康检查** | http://localhost:8080/api/health | `{"success":true}` |
| **Prometheus 指标** | http://localhost:8080/metrics | urn_* 前缀指标 |
| **设备列表 API** | http://localhost:8080/api/devices | 5 台瓮听设备元信息 |
| **ClickHouse HTTP** | http://localhost:8123/play | SQL Playground (user:default, pass 空) |
| **Prometheus UI** | http://localhost:9090/ | 指标查询 (monitoring profile) |
| **MQTT Broker** | `mqtt://localhost:1883` | 匿名, 主题 `urn/sensors/{device_id}` |

### 3.4 日志查看

```bash
# 后端 Rust pipeline 日志
docker compose logs -f backend --tail 200

# 模拟器运行参数 + 汇总周期
docker compose logs -f simulator --tail 100

# MQTT 连接/订阅日志
docker compose logs -f mosquitto --tail 50
```

### 3.5 停止与清理

```bash
# 停止所有服务 (保留数据卷)
docker compose down

# 同时清理数据卷 (ClickHouse/MQTT 数据清空!)
docker compose down -v

# 清理所有构建产物
docker image rm urn-acoustics-backend urn-sensor-simulator
docker builder prune -f
```

---

## 🎚️ 四、传感器模拟器用法

**文件**: [urn_sensor_sim.py](simulator/urn_sensor_sim.py)

根据真实声学模型，为每台瓮听设备计算：
1. 到声源的距离（3D 欧氏距离）
2. **球面扩散衰减** (`20·log₁₀(d)`) + **介质吸收衰减** (`α·d`)
3. 声源频率对瓮的**牵引耦合** + 可配置的**故意漂移**（用于触发告警）
4. 从设备指向声源的**方位角**（用于罗盘显示）
5. 受介质 preset 影响的**温湿度/密度读数**

### 4.1 方式 A: 通过 Docker Compose 环境变量

```bash
# 例 1: 距离 200m 的石灰岩地下声源，自动触发异常漂移
SIM_SOURCE_DISTANCE=200 \
SIM_MEDIUM=limestone \
SIM_DRIFT=8.0 \
SIM_INTERVAL=0.5 \
docker compose --profile simulator up -d --build simulator

# 例 2: 黏土介质 + 近距离声源 (50m) + 低噪声
docker compose run --rm -e SIM_MEDIUM=clay -e SIM_SOURCE_DISTANCE=50 -e SIM_NOISE=0.5 simulator
```

### 4.2 方式 B: 本地命令行 (需 `pip install paho-mqtt`)

```bash
cd simulator
pip install paho-mqtt==2.1.0

# 帮助
python urn_sensor_sim.py --help

# 例 1: 湿润沙介质, 声源坐标明确 (x=80,y=60,z=-3)m
python urn_sensor_sim.py \
    --broker localhost --port 1883 \
    --medium wet_sand \
    --source-x 80 --source-y 60 --source-z -3 \
    --strength-db 145 \
    --interval 1.0

# 例 2: 只模拟 ID 1,3,5 设备, 故意 12% 漂移 (强制 critical 告警)
python urn_sensor_sim.py \
    --devices 1,3,5 \
    --medium dry_sand \
    --source-distance 150 \
    --drift 12.0 \
    --frequency 300

# 例 3: 花岗岩, 10Hz 高速上报 (stress test)
python urn_sensor_sim.py --medium granite --source-distance 80 --interval 0.1
```

### 4.3 全部可配置项

| 环境变量 | CLI 参数 | 默认值 | 说明 |
|---|---|---|---|
| `MQTT_BROKER` | `--broker` | `localhost` | MQTT broker 地址 |
| `MQTT_PORT` | `--port` | `1883` | MQTT broker 端口 |
| `MQTT_TOPIC_PREFIX` | `--topic-prefix` | `urn/sensors` | 主题前缀 (`<prefix>/<device_id>`) |
| `SIM_MEDIUM` | `--medium` | `wet_sand` | 介质 preset (见下) |
| `SIM_SOURCE_DISTANCE` | `--source-distance` | - | 声源距离 m，设置后随机方位，覆盖 X/Y |
| `SIM_SOURCE_X/Y/Z` | `--source-x/y/z` | 100/0/-2 m | 声源精确坐标 (未设置 distance 时生效) |
| `SIM_STRENGTH_DB` | `--strength-db` | `140` | 声源 1m 处基准 SPL (dB) |
| `SIM_FREQUENCY` | `--frequency` | `220` | 声源中心频率 (Hz)，影响瓮的共振读数 |
| `SIM_INTERVAL` | `--interval` | `1.0` | 上报周期 (秒) |
| `SIM_DRIFT` | `--drift` | `0.0` | 故意施加的频率漂移 % (5→warning, 10→critical) |
| `SIM_NOISE` | `--noise` | `2.0` | SPL 噪声标准差 (dB) |
| - | `--devices` | - | 只模拟指定设备 ID, 逗号分隔 (例 `1,3,5`) |

**介质预设**（影响密度、声速、衰减系数、温湿度基线）：

| preset | 名称 | 密度 kg/m³ | 声速 m/s | 衰减 dB/m |
|---|---|---|---|---|
| `dry_sand` | 干燥沙 | 1600 | 300 | 0.50 |
| `wet_sand` | 湿润沙 | 1900 | 500 | 0.30 |
| `clay` | 黏土 | 2200 | 1800 | 0.15 |
| `limestone` | 石灰岩 | 2500 | 3500 | 0.05 |
| `granite` | 花岗岩 | 2700 | 4500 | 0.03 |

---

## 📊 五、可观测性 (Tracing + Metrics)

### 5.1 Prometheus 指标清单

`GET http://localhost:8080/metrics` 返回 (urn_ 前缀)：

| 指标 | 类型 | 标签 | 含义 |
|---|---|---|---|
| `urn_uptime_seconds` | Gauge | - | 服务运行秒数 |
| `urn_mqtt_messages_total` | Counter | - | MQTT 接收到的消息总数 |
| `urn_invalid_readings_total` | Counter | - | JSON 解析/校验失败总数 |
| `urn_acoustic_calculations_total` | Counter | - | Helmholtz+BEM 分析次数 |
| `urn_localizations_total` | Counter | - | 波束形成定位次数 |
| `urn_alerts_total` | Counter | `severity` | 告警次数 (warning/critical) |
| `urn_db_writes_total` | Counter | `table` | 各表写入次数 |
| `urn_ws_active_clients` | Gauge | - | 当前活跃 WebSocket 连接数 |
| `urn_info` | Gauge | `version,...` | 构建信息 |

直方图（通过 `render_prometheus()` 渲染为 summary 近似）：
- 声压级 `urn_sound_pressure_level`
- 漂移百分比 `urn_resonance_drift_percent`
- 定位置信度 `urn_localization_confidence`

### 5.2 Tracing (tower-http TraceLayer)

所有 HTTP 请求自动记录：
- 请求方法 / 路径 / 状态码 / 耗时
- 与 `urn_acoustics_backend=info` 的 pipeline 日志混排

```bash
# 示例: 过滤 HTTP 200 + pipeline 周期日志
docker compose logs backend --tail 200 | grep -E "(HTTP|MQTT|AcousticSimulator|SourceLocator)"
```

### 5.3 Prometheus 常用查询

打开 http://localhost:9090/ 后：

```promql
# 每分钟 MQTT 消息吞吐率
rate(urn_mqtt_messages_total[1m])

# 累计告警次数 (按严重度分离)
sum by (severity) (rate(urn_alerts_total[5m]))

# 活跃 WebSocket 连接趋势
urn_ws_active_clients

# 平均漂移百分比分布 (最近 1h)
histogram_quantile(0.95, rate(urn_resonance_drift_percent_bucket[1h]))
```

---

## 💾 六、ClickHouse 降采样 + 保留策略

**SQL 定义**: [init.sql](database/init.sql)

### 6.1 表级 TTL (数据生命周期)

| 表 | TTL | 说明 |
|---|---|---|
| `sensor_data` | `timestamp + 90 DAY` | 原始高频传感器数据 |
| `resonance_analysis` | `timestamp + 90 DAY` | 原始声学分析结果 |
| `source_localization` | `timestamp + 90 DAY` | 原始定位结果 |
| `alerts` | `timestamp + 180 DAY` | 热告警 (6 个月) |
| `alerts_archive` | `timestamp + 2 YEAR` | 归档告警 (2 年) |
| `sensor_data_1h_agg` | `timestamp + 1 YEAR` | 小时聚合 (1 年) |
| `resonance_analysis_1d_agg` | `day + 2 YEAR` | 日聚合 (2 年) |

### 6.2 降采样物化视图 (MV)

| MV 名称 | 粒度 | 聚合内容 |
|---|---|---|
| `sensor_data_1min_mv` | 1 分钟 | sample_count, avg/max/min SPL, avg_res_freq, avg_density |
| `sensor_data_1h_mv` | 1 小时 | 上述 + SPL 标准差 stddev_spl |
| `resonance_analysis_1d_mv` | 1 天 | 平均增益/漂移, 最大漂移, 异常次数, 平均 Q 值 |

### 6.3 验证 TTL / MV

```sql
-- ClickHouse Playground: http://localhost:8123/play 或客户端:
docker exec -it urn-clickhouse clickhouse-client -d urn_acoustics

-- 查看所有 TTL 配置
SELECT name, engine, create_table_query FROM system.tables
WHERE database='urn_acoustics' AND create_table_query LIKE '%TTL%';

-- 查看 MV 定义
SELECT name, target_table, populate FROM system.tables
WHERE database='urn_acoustics' AND engine LIKE '%MaterializedView%';
```

---

## 🧪 七、本地开发 (非 Docker)

### 7.1 Rust 后端

```bash
cd backend

# 一次性: 依赖下载
cargo fetch

# 运行 (需先在本机起 ClickHouse + MQTT，或用 compose 只起这两个)
URN_CONFIG_DIR=./config cargo run

# 编译回归 (强校验)
cargo check --all-targets
cargo test
cargo clippy -- -W clippy::pedantic
```

### 7.2 前端

直接打开 `frontend/index.html` 即可（纯静态，Three.js 走 CDN）。
若需同域部署：把 `frontend/` 设为 `app.json` 中 `server.static_dir`，由 Rust ServeDir 托管 + Gzip 自动压缩。

### 7.3 模拟器

```bash
cd simulator
pip install paho-mqtt==2.1.0

# 最简用法 (假设 MQTT 在本机 1883)
python urn_sensor_sim.py --source-distance 120 --medium wet_sand
```

---

## 🔌 八、REST API 速查

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | 健康检查 |
| GET | `/metrics` | **Prometheus 指标** (urn_*) |
| GET/POST | `/api/devices` | 列出瓮听设备 / 新增设备 |
| GET | `/api/devices/:id` | 单台设备详情 |
| GET | `/api/sensor-data` | 最近传感器读数 (可带 `?device_id=&limit=`) |
| GET | `/api/localizations` | 最近定位结果 |
| GET | `/api/alerts` | 最近告警列表 |
| GET | `/api/medium-properties` | 介质参数 (4 层预设) |
| GET | `/api/acoustics/config` | 当前声学参数 JSON |
| GET | `/api/resonance/calculate` | 计算指定瓮参数的共振频率 (?volume=&nr=&nl=) |
| POST | `/api/simulate/reading` | 手动注入一条传感器读数 |
| GET | `/api/ws/broadcast-test` | 向所有 WS 客户端推送测试消息 |
| GET | `/ws` | **WebSocket 升级** (broadcast: sensor/resonance/localization/alert) |

**WebSocket 消息格式**:
```json
{
  "message_type": "sensor_data | resonance | localization | alert",
  "data": { /* ...各类型 payload */ },
  "timestamp": "2025-06-22T08:00:00.000Z"
}
```

---

## 🛠️ 九、常见问题 FAQ

**Q1: 启动后 backend 一直 `(health: starting)`？**
> Rust 首次构建在 docker 中较慢（~5-15 分钟），start_period 已设为 90s。若 CPU 核心不足 4，建议调大 start_period 或使用预构建镜像。

**Q2: simulator 正常但前端没数据？**
> 检查 browser Console 中 WebSocket 连接（需要后端服务在 `8080`）。手动验证 API：
> ```bash
> curl -s http://localhost:8080/api/devices | jq
> curl -s http://localhost:8080/api/health
> ```

**Q3: 模拟器漂移 > 5% 但没有告警？**
> 告警判定存在冷却期（默认 `cooldown_seconds = 60`），防止刷屏。60 秒内相同设备漂移不会重复告警。修改 `config/app.json -> alert.cooldown_seconds`。

**Q4: ClickHouse 插入失败 "Table urn_acoustics.sensor_data doesn't exist"？**
> init.sql 只在**数据卷全新**时执行。若之前启动过，请：
> ```bash
> docker compose down -v && docker compose up -d clickhouse
> sleep 30 && docker exec -it urn-clickhouse clickhouse-client -q "SHOW TABLES FROM urn_acoustics"
> ```

**Q5: 如何生产化部署？**
- MQTT：启用 mosquitto `password_file` + TLS 8883 端口
- ClickHouse：ReplicatedMergeTree + 3 节点集群 + ZooKeeper
- Rust：多副本 + nginx-ingress / traefik 终止 TLS
- Prometheus：接入 Grafana + Alertmanager（PagerDuty/飞书 webhook）
- 配置：用 Helm / Kustomize 做 K8s 部署模板（本仓库已完成 docker-compose 形态）

---

## 📜 License

项目内置了古代兵法声学工程的算法复现，仅供教学与考古声学研究。

*"凡军行三十里，有敌情，则令地听之，以知远近。"——《武经总要》*
