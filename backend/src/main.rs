mod acoustic_simulator;
mod acoustics;
mod alarm_ws;
mod config_loader;
mod db_writer;
mod handlers;
mod localization;
mod metrics;
mod models;
mod mqtt_receiver;
mod pipeline;
mod source_locator;
mod store;
mod websocket;

use acoustic_simulator::AcousticSimulator;
use alarm_ws::AlarmWsService;
use config_loader::ConfigBundle;
use dashmap::DashMap;
use db_writer::DbWriter;
use handlers::AppState;
use mqtt_receiver::MqttReceiver;
use models::UrnDevice;
use source_locator::SourceLocator;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use axum::http::header::{HeaderName, HeaderValue};
use axum::response::PlainText;
use axum::routing::{get, post};
use axum::Router;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "urn_acoustics_backend=info,tower_http=info,axum=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("[Boot] 加载配置文件...");
    let cfg = match ConfigBundle::load() {
        Ok(c) => {
            info!(
                "[Boot] 配置加载成功: MQTT={}:{} HTTP={}:{}",
                c.app.mqtt.broker, c.app.mqtt.port, c.app.server.host, c.app.server.port
            );
            c
        }
        Err(e) => {
            error!("[Boot] 配置加载失败: {}", e);
            std::process::exit(1);
        }
    };

    metrics::init_metrics();

    let devices = Arc::new(DashMap::<u32, UrnDevice>::new());
    register_default_devices(&devices);

    info!("[Boot] 初始化 ClickHouse...");
    let store = store::ClickHouseStore::new(
        &cfg.app.clickhouse.url,
        &cfg.app.clickhouse.database,
        &cfg.app.clickhouse.user,
        &cfg.app.clickhouse.password,
    );

    info!("[Boot] 构建 pipeline 通道...");
    let p = &cfg.app.pipeline;

    let (mqtt_to_acoustic_tx, mqtt_to_acoustic_rx) = mpsc::channel(p.mqtt_to_acoustic_buffer);
    let (mqtt_to_alarm_tx, mqtt_to_alarm_rx) = mpsc::channel(p.mqtt_to_acoustic_buffer);
    let (mqtt_to_db_tx, mqtt_to_db_rx) = mpsc::channel(p.sensor_raw_to_db_buffer);

    let (acoustic_to_locator_tx, acoustic_to_locator_rx) = mpsc::channel(p.acoustic_to_locator_buffer);
    let (acoustic_to_alarm_tx, acoustic_to_alarm_rx) = mpsc::channel(p.acoustic_to_locator_buffer);
    let (acoustic_to_db_tx, acoustic_to_db_rx) = mpsc::channel(p.sensor_raw_to_db_buffer);

    let (locator_to_alarm_tx, locator_to_alarm_rx) = mpsc::channel(p.locator_to_alarm_buffer);
    let (locator_to_db_tx, locator_to_db_rx) = mpsc::channel(p.sensor_raw_to_db_buffer);

    let (alarm_to_db_tx, alarm_to_db_rx) = mpsc::channel(p.sensor_raw_to_db_buffer);

    let alarm_service = AlarmWsService::new(&cfg.app.alert);
    let simulator = AcousticSimulator::new(cfg.acoustics.clone());
    let locator = SourceLocator::new(cfg.app.localization.clone());
    let db_writer = DbWriter::new(store.clone());
    let receiver = Arc::new(MqttReceiver::new(devices.clone(), cfg.app.mqtt.clone()));

    info!("[Boot] 启动 pipeline workers...");

    tokio::spawn(async move {
        receiver
            .run(mqtt_to_acoustic_tx, mqtt_to_alarm_tx, mqtt_to_db_tx)
            .await;
    });

    tokio::spawn(async move {
        simulator
            .run(
                mqtt_to_acoustic_rx,
                acoustic_to_locator_tx,
                acoustic_to_alarm_tx,
                acoustic_to_db_tx,
            )
            .await;
    });

    tokio::spawn(async move {
        locator
            .run(acoustic_to_locator_rx, locator_to_alarm_tx, locator_to_db_tx)
            .await;
    });

    let alarm_for_app = alarm_service.clone();
    tokio::spawn(async move {
        alarm_service
            .run(
                mqtt_to_alarm_rx,
                acoustic_to_alarm_rx,
                locator_to_alarm_rx,
                alarm_to_db_tx,
            )
            .await;
    });

    tokio::spawn(async move {
        db_writer
            .run(mqtt_to_db_rx, acoustic_to_db_rx, locator_to_db_rx, alarm_to_db_rx)
            .await;
    });

    info!("[Boot] 构建 HTTP/WS 服务...");

    let app_state = AppState {
        store: store.clone(),
        alarm_manager: alarm_for_app,
        devices: devices.clone(),
        config: cfg.app.clone(),
        acoustics_config: cfg.acoustics.clone(),
        media_config: cfg.media.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_dir_path = cfg.app.server.static_dir.clone();
    let serve_dir = ServeDir::new(static_dir_path);

    let compression = CompressionLayer::new().gzip(true);

    let app = Router::new()
        .route("/api/health", get(handlers::health_check))
        .route("/metrics", get(metrics_handler))
        .route("/api/devices", get(handlers::get_devices).post(handlers::create_device))
        .route("/api/devices/:id", get(handlers::get_device))
        .route("/api/sensor-data", get(handlers::get_sensor_data))
        .route("/api/localizations", get(handlers::get_recent_localizations))
        .route("/api/alerts", get(handlers::get_recent_alerts))
        .route("/api/medium-properties", get(handlers::get_medium_properties))
        .route("/api/acoustics/config", get(handlers::get_acoustics_config))
        .route("/api/resonance/calculate", get(handlers::calculate_resonance))
        .route("/api/simulate/reading", post(handlers::simulate_reading))
        .route("/api/ws/broadcast-test", get(handlers::broadcast_test_message))
        .route("/ws", get(websocket::websocket_handler))
        .fallback_service(serve_dir)
        .layer(compression)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-powered-by"),
            HeaderValue::from_static("urn-acoustics/0.1.0"),
        ))
        .with_state(app_state);

    let addr = format!("{}:{}", cfg.app.server.host, cfg.app.server.port);
    info!("[Boot] 瓮听声学系统后端服务启动中: http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("[Boot] 无法绑定地址 {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    match axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        Ok(_) => info!("[Boot] 服务器正常关闭"),
        Err(e) => error!("[Boot] 服务器错误: {}", e),
    }
}

fn register_default_devices(devices: &DashMap<u32, UrnDevice>) {
    let defaults = vec![
        UrnDevice { device_id: 1, device_name: "瓮听-东北角".into(), deployment_x: -50.0, deployment_y: -50.0, deployment_z: -2.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        UrnDevice { device_id: 2, device_name: "瓮听-东南角".into(), deployment_x: 50.0, deployment_y: -50.0, deployment_z: -2.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        UrnDevice { device_id: 3, device_name: "瓮听-西南角".into(), deployment_x: 50.0, deployment_y: 50.0, deployment_z: -2.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        UrnDevice { device_id: 4, device_name: "瓮听-西北角".into(), deployment_x: -50.0, deployment_y: 50.0, deployment_z: -2.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        UrnDevice { device_id: 5, device_name: "瓮听-正中央".into(), deployment_x: 0.0, deployment_y: 0.0, deployment_z: -2.0, urn_volume: 0.08, neck_radius: 0.06, neck_length: 0.12 },
    ];
    for d in defaults {
        info!("[Boot] 注册默认设备: {} (ID={})", d.device_name, d.device_id);
        devices.insert(d.device_id, d);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("[Boot] 收到关闭信号，正在优雅停止服务...");
}

async fn metrics_handler() -> PlainText<String> {
    PlainText(metrics::render_prometheus())
}
