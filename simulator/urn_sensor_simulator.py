"""
瓮听传感器MQTT模拟器
模拟古代瓮听(地听)设备，每分钟上报声学数据到MQTT Broker

使用方法:
    pip install paho-mqtt numpy
    python urn_sensor_simulator.py --broker localhost --port 1883
"""

import argparse
import json
import math
import random
import time
from datetime import datetime, timezone
from dataclasses import dataclass, asdict
from typing import List

try:
    import paho.mqtt.client as mqtt
except ImportError:
    print("请先安装 paho-mqtt: pip install paho-mqtt")
    exit(1)

try:
    import numpy as np
except ImportError:
    print("警告: numpy未安装，将使用标准随机数替代")
    np = None


@dataclass
class UrnDevice:
    device_id: int
    device_name: str
    x: float
    y: float
    z: float
    volume: float = 0.05
    neck_radius: float = 0.05
    neck_length: float = 0.1


@dataclass
class SensorReading:
    timestamp: str
    device_id: int
    sound_pressure_level: float
    resonance_frequency: float
    source_direction: float
    medium_density: float
    temperature: float
    humidity: float


DEFAULT_DEVICES = [
    UrnDevice(1, "瓮听-东北角", -50.0, -50.0, -2.0),
    UrnDevice(2, "瓮听-东南角", 50.0, -50.0, -2.0),
    UrnDevice(3, "瓮听-西南角", 50.0, 50.0, -2.0),
    UrnDevice(4, "瓮听-西北角", -50.0, 50.0, -2.0),
    UrnDevice(5, "瓮听-正中央", 0.0, 0.0, -2.0, 0.08, 0.06, 0.12),
]

MEDIUM_TYPES = [
    ("dry_sand", 1600.0),
    ("wet_sand", 1900.0),
    ("clay", 2000.0),
    ("water_saturated_soil", 2200.0),
]


class UrnSensorSimulator:
    def __init__(self, broker: str, port: int, topic: str,
                 interval: int = 60, client_id: str = "urn_simulator"):
        self.broker = broker
        self.port = port
        self.topic = topic
        self.interval = interval
        self.devices: List[UrnDevice] = DEFAULT_DEVICES
        self.connected = False
        self.source_x = None
        self.source_y = None
        self.source_active_ticks = 0

        self.client = mqtt.Client(client_id=client_id, protocol=mqtt.MQTTv5)
        self.client.on_connect = self._on_connect
        self.client.on_disconnect = self._on_disconnect
        self.client.on_publish = self._on_publish

    def _on_connect(self, client, userdata, flags, rc, properties=None):
        if rc == 0:
            self.connected = True
            print(f"[OK] 已连接到MQTT Broker {self.broker}:{self.port}")
        else:
            print(f"[ERR] MQTT连接失败，错误码: {rc}")

    def _on_disconnect(self, client, userdata, rc, properties=None):
        self.connected = False
        print(f"[WARN] MQTT连接断开，错误码: {rc}")

    def _on_publish(self, client, userdata, mid, reason_code, properties):
        pass

    def connect(self):
        try:
            self.client.connect(self.broker, self.port, keepalive=120)
            self.client.loop_start()
            timeout = 0
            while not self.connected and timeout < 10:
                time.sleep(0.5)
                timeout += 0.5
            return self.connected
        except Exception as e:
            print(f"[ERR] 连接MQTT失败: {e}")
            return False

    def helmholtz_frequency(self, device: UrnDevice, speed_of_sound: float = 343.0) -> float:
        neck_area = math.pi * device.neck_radius ** 2
        effective_length = device.neck_length + 1.7 * device.neck_radius
        denominator = 2.0 * math.pi * math.sqrt(device.volume * effective_length / neck_area)
        return speed_of_sound / denominator

    def calculate_spl_from_source(self, device: UrnDevice, base_spl: float = 94.0) -> float:
        if self.source_x is None or self.source_y is None:
            return base_spl + random.gauss(0, 5)
        dx = device.x - self.source_x
        dy = device.y - self.source_y
        distance = math.sqrt(dx * dx + dy * dy)
        attenuation = 6.0 * math.log10(max(distance, 1.0))
        medium_attenuation = 0.3 * distance
        noise = random.gauss(0, 3)
        return max(40, base_spl - attenuation - medium_attenuation + noise)

    def calculate_source_direction(self, device: UrnDevice) -> float:
        if self.source_x is None or self.source_y is None:
            return random.uniform(0, 360)
        dx = self.source_x - device.x
        dy = self.source_y - device.y
        angle = math.degrees(math.atan2(dy, dx))
        bearing = (angle + 450) % 360
        noise = random.gauss(0, 5)
        return (bearing + noise) % 360

    def generate_reading(self, device: UrnDevice, tick: int) -> SensorReading:
        base_freq = self.helmholtz_frequency(device)
        drift_noise = random.gauss(0, base_freq * 0.03)
        if tick % 50 == 0 and random.random() < 0.3:
            drift_noise += random.choice([-1, 1]) * base_freq * random.uniform(0.08, 0.2)

        resonance_freq = base_freq + drift_noise
        spl = self.calculate_spl_from_source(device)
        direction = self.calculate_source_direction(device)

        medium_type, medium_density = random.choice(MEDIUM_TYPES)
        density_noise = random.gauss(0, 50)

        return SensorReading(
            timestamp=datetime.now(timezone.utc).isoformat(),
            device_id=device.device_id,
            sound_pressure_level=round(spl, 2),
            resonance_frequency=round(resonance_freq, 2),
            source_direction=round(direction, 1),
            medium_density=round(medium_density + density_noise, 1),
            temperature=round(15 + random.uniform(0, 15), 1),
            humidity=round(30 + random.uniform(0, 50), 1),
        )

    def maybe_trigger_enemy_event(self, tick: int):
        if self.source_active_ticks > 0:
            self.source_active_ticks -= 1
            if self.source_active_ticks == 0:
                self.source_x = None
                self.source_y = None
                print(f"[INFO] ({datetime.now().strftime('%H:%M:%S')}) 敌军声源信号消失")
            return

        if random.random() < 0.15:
            angle = random.uniform(0, math.pi * 2)
            distance = random.uniform(100, 400)
            self.source_x = math.cos(angle) * distance
            self.source_y = math.sin(angle) * distance
            self.source_active_ticks = random.randint(3, 8)
            bearing = (math.degrees(angle) + 450) % 360
            print(f"[ALERT] ({datetime.now().strftime('%H:%M:%S')}) 检测到敌军声源! "
                  f"方位: {bearing:.0f}°, 距离: {distance:.0f}m, 坐标: ({self.source_x:.0f}, {self.source_y:.0f})")

    def publish_reading(self, reading: SensorReading):
        payload = json.dumps(asdict(reading), ensure_ascii=False)
        topic = f"{self.topic}/{reading.device_id}"
        result = self.client.publish(topic, payload, qos=1)
        return result.rc == mqtt.MQTT_ERR_SUCCESS

    def run(self):
        print("=" * 60)
        print("  古代瓮听(地听)传感器模拟器")
        print(f"  Broker: {self.broker}:{self.port}")
        print(f"  Topic:  {self.topic}")
        print(f"  上报间隔: {self.interval}秒")
        print(f"  模拟设备数: {len(self.devices)}")
        print("=" * 60)

        if not self.connect():
            print("[ERR] 无法连接到MQTT Broker，退出")
            return

        tick = 0
        try:
            while True:
                self.maybe_trigger_enemy_event(tick)
                print(f"\n[{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}] "
                      f"开始上报第 {tick + 1} 轮数据...")

                success_count = 0
                for device in self.devices:
                    reading = self.generate_reading(device, tick)
                    if self.publish_reading(reading):
                        success_count += 1
                        print(f"  -> {device.device_name} (ID={device.device_id}): "
                              f"SPL={reading.sound_pressure_level:.1f}dB, "
                              f"F_res={reading.resonance_frequency:.1f}Hz, "
                              f"Dir={reading.source_direction:.0f}°")
                    else:
                        print(f"  -> {device.device_name}: 发布失败")

                print(f"  本轮成功上报: {success_count}/{len(self.devices)}")

                tick += 1
                for _ in range(self.interval):
                    time.sleep(1)
                    if not self.connected:
                        print("[WARN] MQTT连接已断开，尝试重连...")
                        self.connect()

        except KeyboardInterrupt:
            print("\n[INFO] 用户停止模拟器")
        finally:
            self.client.loop_stop()
            self.client.disconnect()
            print("[INFO] 已断开MQTT连接")


def main():
    parser = argparse.ArgumentParser(description="瓮听传感器MQTT模拟器")
    parser.add_argument("--broker", default="localhost", help="MQTT Broker地址")
    parser.add_argument("--port", type=int, default=1883, help="MQTT Broker端口")
    parser.add_argument("--topic", default="urn/sensors", help="MQTT主题前缀")
    parser.add_argument("--interval", type=int, default=60, help="上报间隔(秒)")
    parser.add_argument("--client-id", default="urn_sensor_simulator", help="MQTT客户端ID")
    parser.add_argument("--fast", action="store_true", help="快速模式：5秒间隔（用于测试）")
    args = parser.parse_args()

    interval = 5 if args.fast else args.interval

    sim = UrnSensorSimulator(
        broker=args.broker,
        port=args.port,
        topic=args.topic,
        interval=interval,
        client_id=args.client_id,
    )
    sim.run()


if __name__ == "__main__":
    main()
