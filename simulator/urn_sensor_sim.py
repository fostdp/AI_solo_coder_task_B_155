#!/usr/bin/env python3
"""
瓮听传感器模拟器 (Urn Acoustic Sensor Simulator)

功能：
- 模拟多台瓮听设备的传感器数据上报
- 支持配置声源距离、位置、介质条件（影响声速/衰减）
- 支持通过环境变量或命令行参数配置
- 通过 MQTT 向 broker 上报 JSON 格式数据
"""

import argparse
import json
import math
import os
import random
import sys
import time
from dataclasses import dataclass, field, asdict
from typing import Dict, List, Optional

try:
    import paho.mqtt.client as mqtt
except ImportError:
    print("ERROR: 需要 paho-mqtt 库，请执行: pip install paho-mqtt", file=sys.stderr)
    sys.exit(1)


# ---------- 介质参数 ----------
MEDIUM_PRESETS: Dict[str, Dict] = {
    "dry_sand": {
        "name": "干燥沙",
        "density": 1600.0,      # kg/m^3
        "sound_speed": 300.0,   # m/s
        "attenuation": 0.5,     # dB/m
        "temp_base": 25.0,
        "humidity_base": 30.0,
    },
    "wet_sand": {
        "name": "湿润沙",
        "density": 1900.0,
        "sound_speed": 500.0,
        "attenuation": 0.3,
        "temp_base": 20.0,
        "humidity_base": 60.0,
    },
    "clay": {
        "name": "黏土",
        "density": 2200.0,
        "sound_speed": 1800.0,
        "attenuation": 0.15,
        "temp_base": 18.0,
        "humidity_base": 75.0,
    },
    "limestone": {
        "name": "石灰岩",
        "density": 2500.0,
        "sound_speed": 3500.0,
        "attenuation": 0.05,
        "temp_base": 15.0,
        "humidity_base": 40.0,
    },
    "granite": {
        "name": "花岗岩",
        "density": 2700.0,
        "sound_speed": 4500.0,
        "attenuation": 0.03,
        "temp_base": 14.0,
        "humidity_base": 20.0,
    },
}

# ---------- 默认设备（与 Rust 后端 register_default_devices 一致） ----------
DEFAULT_DEVICES: List[Dict] = [
    {"device_id": 1, "device_name": "瓮听-东北角", "x": -50.0, "y": -50.0, "z": -2.0,
     "urn_volume": 0.05, "neck_radius": 0.05, "neck_length": 0.1},
    {"device_id": 2, "device_name": "瓮听-东南角", "x":  50.0, "y": -50.0, "z": -2.0,
     "urn_volume": 0.05, "neck_radius": 0.05, "neck_length": 0.1},
    {"device_id": 3, "device_name": "瓮听-西南角", "x":  50.0, "y":  50.0, "z": -2.0,
     "urn_volume": 0.05, "neck_radius": 0.05, "neck_length": 0.1},
    {"device_id": 4, "device_name": "瓮听-西北角", "x": -50.0, "y":  50.0, "z": -2.0,
     "urn_volume": 0.05, "neck_radius": 0.05, "neck_length": 0.1},
    {"device_id": 5, "device_name": "瓮听-正中央", "x":   0.0, "y":   0.0, "z": -2.0,
     "urn_volume": 0.08, "neck_radius": 0.06, "neck_length": 0.12},
]

# 理论共振频率基础值（基于 Helmholtz）
BASE_THEORETICAL_FREQ = 185.0  # Hz
SPEED_OF_SOUND_AIR = 343.0      # m/s


@dataclass
class SimulatorConfig:
    """模拟器运行时配置"""
    mqtt_broker: str = "localhost"
    mqtt_port: int = 1883
    mqtt_topic_prefix: str = "urn/sensors"
    mqtt_username: Optional[str] = None
    mqtt_password: Optional[str] = None

    medium: str = "wet_sand"          # 介质 preset 名称
    source_x: float = 100.0           # 声源 X (m)
    source_y: float = 0.0             # 声源 Y (m)
    source_z: float = -2.0            # 声源深度 (m)
    source_distance: Optional[float] = None  # 若设置，则覆盖 source_x/source_y，方向随机
    source_strength_db: float = 140.0 # 声源在 1m 处的基准 SPL (dB)
    source_frequency: float = 220.0   # 声源中心频率 (Hz)，影响共振频率读数

    reporting_interval: float = 1.0   # 上报间隔 (秒)
    noise_std: float = 2.0            # SPL 噪声标准差 (dB)
    freq_noise_std: float = 2.0       # 频率噪声标准差 (Hz)
    drift_percent: float = 0.0        # 故意施加的频率漂移 %（用于模拟告警）

    ansi_art: bool = True             # 启动时打印 ANSI art


def load_config_from_env() -> SimulatorConfig:
    """从环境变量加载配置，命令行参数优先级更高"""
    cfg = SimulatorConfig()
    cfg.mqtt_broker = os.getenv("MQTT_BROKER", cfg.mqtt_broker)
    cfg.mqtt_port = int(os.getenv("MQTT_PORT", str(cfg.mqtt_port)))
    cfg.mqtt_topic_prefix = os.getenv("MQTT_TOPIC_PREFIX", cfg.mqtt_topic_prefix)
    cfg.mqtt_username = os.getenv("MQTT_USERNAME") or None
    cfg.mqtt_password = os.getenv("MQTT_PASSWORD") or None
    cfg.medium = os.getenv("SIM_MEDIUM", cfg.medium)
    cfg.source_x = float(os.getenv("SIM_SOURCE_X", str(cfg.source_x)))
    cfg.source_y = float(os.getenv("SIM_SOURCE_Y", str(cfg.source_y)))
    cfg.source_z = float(os.getenv("SIM_SOURCE_Z", str(cfg.source_z)))
    d = os.getenv("SIM_SOURCE_DISTANCE")
    cfg.source_distance = float(d) if d not in (None, "") else None
    cfg.source_strength_db = float(os.getenv("SIM_STRENGTH_DB", str(cfg.source_strength_db)))
    cfg.source_frequency = float(os.getenv("SIM_FREQUENCY", str(cfg.source_frequency)))
    cfg.reporting_interval = float(os.getenv("SIM_INTERVAL", str(cfg.reporting_interval)))
    cfg.noise_std = float(os.getenv("SIM_NOISE", str(cfg.noise_std)))
    cfg.freq_noise_std = float(os.getenv("SIM_FREQ_NOISE", str(cfg.freq_noise_std)))
    cfg.drift_percent = float(os.getenv("SIM_DRIFT", str(cfg.drift_percent)))
    return cfg


def distance_3d(x1: float, y1: float, z1: float, x2: float, y2: float, z2: float) -> float:
    return math.sqrt((x1 - x2) ** 2 + (y1 - y2) ** 2 + (z1 - z2) ** 2)


def compute_direction_angle(dx: float, dy: float) -> float:
    """从设备到声源的方位角（0=北，顺时针，0~360°）"""
    # 设备到声源的方向向量
    angle_rad = math.atan2(dx, -dy)  # dy 向北为负? 简单调整：0=北(-y方向), 90=东(+x)
    deg = math.degrees(angle_rad) % 360.0
    return deg


def theoretical_resonance_freq(device: Dict, sound_speed_air: float) -> float:
    """简化的 Helmholtz 共振频率公式"""
    V = device["urn_volume"]
    a = device["neck_radius"]
    L = device["neck_length"]
    S = math.pi * a * a
    # 颈部末端修正
    L_eff = L + 1.7 * a
    if L_eff <= 0 or V <= 0 or S <= 0:
        return BASE_THEORETICAL_FREQ
    f0 = (sound_speed_air / (2 * math.pi)) * math.sqrt(S / (V * L_eff))
    return f0


def compute_reading(device: Dict, cfg: SimulatorConfig, mprops: Dict) -> Dict:
    """根据声源位置和介质条件，为一台设备生成一条传感器读数"""
    dist = distance_3d(
        device["x"], device["y"], device["z"],
        cfg.source_x, cfg.source_y, cfg.source_z,
    )
    # 距离至少 1m，避免除零
    dist = max(1.0, dist)

    # 球面扩散 + 介质吸收衰减
    spreading_loss = 20.0 * math.log10(dist)
    medium_loss = mprops["attenuation"] * dist
    spl = cfg.source_strength_db - spreading_loss - medium_loss
    spl += random.gauss(0.0, cfg.noise_std)
    spl = max(20.0, min(200.0, spl))

    # 共振频率：基于瓮的理论值，受声源频率和漂移参数牵引
    f_theory = theoretical_resonance_freq(device, SPEED_OF_SOUND_AIR)
    coupling = 0.6  # 声源频率牵引系数
    freq_target = f_theory * (1 - coupling) + cfg.source_frequency * coupling
    freq_target *= (1.0 + cfg.drift_percent / 100.0)
    measured_freq = freq_target + random.gauss(0.0, cfg.freq_noise_std)

    # 从设备指向声源的方位角
    dx = cfg.source_x - device["x"]
    dy = cfg.source_y - device["y"]
    direction = compute_direction_angle(dx, dy)

    # 介质读数：受 preset 影响 + 小噪声
    density = mprops["density"] + random.gauss(0.0, mprops["density"] * 0.01)
    temp = mprops["temp_base"] + random.gauss(0.0, 1.0)
    humidity = max(0.0, min(100.0, mprops["humidity_base"] + random.gauss(0.0, 2.0)))

    return {
        "device_id": device["device_id"],
        "sound_pressure_level": round(spl, 2),
        "resonance_frequency": round(measured_freq, 2),
        "source_direction": round(direction, 2),
        "medium_density": round(density, 1),
        "temperature": round(temp, 2),
        "humidity": round(humidity, 1),
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S.") + f"{int(time.time()*1000)%1000:03d}Z",
    }


ANSI_ART = r"""
   _____                      _    _____           _       _   _             
  / ____|                    | |  / ____|         (_)     | | (_)            
 | |     ___  _ ____   _____ | |_| (___   ___ _ __ _  __ _| |_ _ _ __   __ _ 
 | |    / _ \| '_ \ \ / / _ \| __|\___ \ / __| '__| |/ _` | __| | '_ \ / _` |
 | |___| (_) | | | \ V / (_) | |_ ____) | (__| |  | | (_| | |_| | | | | (_| |
  \_____\___/|_| |_|\_/ \___/ \__|_____/ \___|_|  |_|\__,_|\__|_|_| |_|\__, |
                                                                        __/ |
                                                                       |___/ 
    古代瓮听声学仿真 - 传感器模拟器  |  声源定位 / Helmholtz 共振 / 波束形成
"""


def print_banner(cfg: SimulatorConfig, mprops: Dict):
    if cfg.ansi_art:
        print(ANSI_ART, file=sys.stderr)
    print("=" * 72, file=sys.stderr)
    print(f"  MQTT Broker     : {cfg.mqtt_broker}:{cfg.mqtt_port}", file=sys.stderr)
    print(f"  Topic Prefix    : {cfg.mqtt_topic_prefix}/<device_id>", file=sys.stderr)
    print(f"  上报间隔         : {cfg.reporting_interval}s   设备数量: {len(DEFAULT_DEVICES)}", file=sys.stderr)
    print(f"  介质条件         : {mprops['name']} ({cfg.medium})", file=sys.stderr)
    print(f"                     ρ={mprops['density']} kg/m³  c={mprops['sound_speed']} m/s  α={mprops['attenuation']} dB/m", file=sys.stderr)
    if cfg.source_distance is not None:
        print(f"  声源距离 (随机)  : {cfg.source_distance} m", file=sys.stderr)
    else:
        print(f"  声源坐标         : ({cfg.source_x}, {cfg.source_y}, {cfg.source_z}) m", file=sys.stderr)
    print(f"  声源强度         : {cfg.source_strength_db} dB @1m   中心频率: {cfg.source_frequency} Hz", file=sys.stderr)
    print(f"  频率漂移(模拟异常): {cfg.drift_percent:+.2f} %", file=sys.stderr)
    print("=" * 72, file=sys.stderr)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="瓮听传感器模拟器", formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--broker", help="MQTT broker 地址")
    p.add_argument("--port", type=int, help="MQTT broker 端口")
    p.add_argument("--topic-prefix", help="MQTT 主题前缀")
    p.add_argument("--medium", choices=list(MEDIUM_PRESETS.keys()), help=f"介质预设 ({', '.join(MEDIUM_PRESETS.keys())})")
    p.add_argument("--source-x", type=float, help="声源 X 坐标 (m)")
    p.add_argument("--source-y", type=float, help="声源 Y 坐标 (m)")
    p.add_argument("--source-z", type=float, help="声源 Z 深度 (m, 负值表示地下)")
    p.add_argument("--source-distance", type=float, help="声源距离 (m)，设置后随机方位，覆盖 source-x/y")
    p.add_argument("--strength-db", type=float, help="声源在 1m 处的 SPL (dB)")
    p.add_argument("--frequency", type=float, help="声源中心频率 (Hz)")
    p.add_argument("--interval", type=float, help="上报间隔 (秒)")
    p.add_argument("--drift", type=float, help="故意施加的频率漂移百分比，正值模拟异常")
    p.add_argument("--noise", type=float, help="SPL 噪声标准差 (dB)")
    p.add_argument("--devices", type=str, default=None, help="只模拟指定设备ID，逗号分隔，如 1,3,5")
    return p.parse_args()


def main():
    env_cfg = load_config_from_env()
    args = parse_args()

    # 命令行参数覆盖环境变量
    cfg = env_cfg
    if args.broker: cfg.mqtt_broker = args.broker
    if args.port: cfg.mqtt_port = args.port
    if args.topic_prefix: cfg.mqtt_topic_prefix = args.topic_prefix
    if args.medium: cfg.medium = args.medium
    if args.source_x is not None: cfg.source_x = args.source_x
    if args.source_y is not None: cfg.source_y = args.source_y
    if args.source_z is not None: cfg.source_z = args.source_z
    if args.source_distance is not None: cfg.source_distance = args.source_distance
    if args.strength_db is not None: cfg.source_strength_db = args.strength_db
    if args.frequency is not None: cfg.source_frequency = args.frequency
    if args.interval is not None: cfg.reporting_interval = args.interval
    if args.drift is not None: cfg.drift_percent = args.drift
    if args.noise is not None: cfg.noise_std = args.noise

    # 选择设备子集
    devices = DEFAULT_DEVICES
    if args.devices:
        ids = set(int(x.strip()) for x in args.devices.split(",") if x.strip().isdigit())
        devices = [d for d in DEFAULT_DEVICES if d["device_id"] in ids]
        if not devices:
            print("错误: 过滤后没有匹配的设备", file=sys.stderr)
            sys.exit(1)

    # 若指定 source_distance，则生成一个随机方位的坐标
    if cfg.source_distance is not None and cfg.source_distance > 0:
        theta = random.uniform(0, 2 * math.pi)
        cfg.source_x = round(cfg.source_distance * math.cos(theta), 2)
        cfg.source_y = round(cfg.source_distance * math.sin(theta), 2)

    # 选择介质
    if cfg.medium not in MEDIUM_PRESETS:
        print(f"警告: 未知介质 '{cfg.medium}'，回退到 wet_sand", file=sys.stderr)
        cfg.medium = "wet_sand"
    mprops = MEDIUM_PRESETS[cfg.medium]

    print_banner(cfg, mprops)

    # ---------- MQTT 连接 ----------
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=f"urn-sim-{int(time.time())}")
    if cfg.mqtt_username:
        client.username_pw_set(cfg.mqtt_username, cfg.mqtt_password or None)

    connected = {"ok": False}

    def on_connect(cli, userdata, flags, rc, properties=None):
        if rc == 0:
            connected["ok"] = True
            print(f"[MQTT] 已连接到 {cfg.mqtt_broker}:{cfg.mqtt_port}", file=sys.stderr)
        else:
            print(f"[MQTT] 连接失败 rc={rc}", file=sys.stderr)

    def on_disconnect(cli, userdata, dc, *a):
        connected["ok"] = False
        print(f"[MQTT] 已断开 原因={dc}", file=sys.stderr)

    client.on_connect = on_connect
    client.on_disconnect = on_disconnect

    try:
        client.connect(cfg.mqtt_broker, cfg.mqtt_port, keepalive=30)
        client.loop_start()
    except Exception as e:
        print(f"[MQTT] 连接异常: {e}", file=sys.stderr)
        sys.exit(1)

    # 等待连接
    for _ in range(20):
        if connected["ok"]:
            break
        time.sleep(0.25)
    if not connected["ok"]:
        print("[MQTT] 无法在 5 秒内建立连接，退出", file=sys.stderr)
        sys.exit(1)

    cycle = 0
    try:
        while True:
            cycle += 1
            published = 0
            for d in devices:
                reading = compute_reading(d, cfg, mprops)
                topic = f"{cfg.mqtt_topic_prefix}/{reading['device_id']}"
                payload = json.dumps(reading, ensure_ascii=False)
                try:
                    info = client.publish(topic, payload, qos=1)
                    if info.rc == mqtt.MQTT_ERR_SUCCESS:
                        published += 1
                except Exception as e:
                    print(f"[MQTT] 发布失败 device_id={d['device_id']} err={e}", file=sys.stderr)
            # 每 10 轮打一条汇总日志
            if cycle == 1 or cycle % 10 == 0:
                total_dists = [
                    round(distance_3d(d["x"], d["y"], d["z"], cfg.source_x, cfg.source_y, cfg.source_z), 1)
                    for d in devices
                ]
                print(
                    f"[Simulator] cycle={cycle}  发布设备数={published}/{len(devices)}  "
                    f"距离范围=[{min(total_dists)}~{max(total_dists)}]m  "
                    f"声源=({cfg.source_x:.0f},{cfg.source_y:.0f})  介质={cfg.medium}",
                    flush=True,
                )
            time.sleep(cfg.reporting_interval)
    except KeyboardInterrupt:
        print("\n[Simulator] 收到 Ctrl+C，退出中...", file=sys.stderr)
    finally:
        client.loop_stop()
        client.disconnect()
        print("[Simulator] 已停止", file=sys.stderr)


if __name__ == "__main__":
    main()
