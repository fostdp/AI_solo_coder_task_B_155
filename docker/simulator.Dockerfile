FROM python:3.12-slim-bookworm

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_NO_CACHE_DIR=1

WORKDIR /app

RUN pip install --no-cache-dir paho-mqtt==2.1.0

COPY simulator/urn_sensor_sim.py /app/urn_sensor_sim.py

RUN chmod +x /app/urn_sensor_sim.py

EXPOSE 1883

ENTRYPOINT ["python", "-u", "/app/urn_sensor_sim.py"]
