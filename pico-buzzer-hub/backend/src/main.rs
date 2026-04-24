use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Duration, timeout},
};
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<HubState>>,
}

#[derive(Debug, Default)]
struct HubState {
    next_order: u64,
    round_id: u64,
    live_started_at: Option<Instant>,
    live_at_us: Option<u128>,
    buzzes: Vec<BuzzEntry>,
    devices: Vec<DevicePresence>,
    light_readings: Vec<LightReading>,
}

#[derive(Debug, Clone, Serialize)]
struct BuzzEntry {
    order: u64,
    round_id: u64,
    player_id: String,
    player_name: Option<String>,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    received_at_us: u128,
    received_at_ms: u128,
    reaction_us: Option<u128>,
    client_round_id: Option<u64>,
    client_reaction_us: Option<i128>,
    client_timing_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct LightReading {
    player_id: String,
    player_name: Option<String>,
    device_id: Option<String>,
    mac_address: Option<String>,
    light_pin: Option<u8>,
    raw: u16,
    percent: f32,
    received_at_ms: u128,
}

#[derive(Debug, Deserialize)]
struct DeviceRequest {
    #[serde(default)]
    player_name: Option<String>,
    #[serde(default, rename = "player_id", alias = "student_id")]
    reported_player_id: Option<String>,
    app_kind: Option<String>,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    button_pressed: Option<bool>,
    calibration_port: Option<u16>,
    firmware_version: Option<String>,
    client_checkin_rtt_us: Option<u128>,
    client_go_round_id: Option<u64>,
    client_go_ticks_set: Option<bool>,
    round_id: Option<u64>,
    client_reaction_us: Option<i128>,
    client_timing_status: Option<String>,
    light_pin: Option<u8>,
    light_raw: Option<u16>,
    light_percent: Option<f32>,
    ip_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DevicePresence {
    player_id: String,
    player_name: Option<String>,
    app_kind: String,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    button_pressed: bool,
    calibration_port: Option<u16>,
    firmware_version: Option<String>,
    last_client_checkin_rtt_us: Option<u128>,
    last_client_checkin_jitter_us: Option<u128>,
    last_client_checkin_at_ms: Option<u128>,
    last_client_go_round_id: Option<u64>,
    last_client_go_ticks_set: Option<bool>,
    last_client_go_seen_at_ms: Option<u128>,
    last_calibration_round_id: Option<u64>,
    last_calibration_rtt_us: Option<u128>,
    last_calibration_jitter_us: Option<u128>,
    last_calibration_at_ms: Option<u128>,
    last_calibration_error: Option<String>,
    last_go_attempt_round_id: Option<u64>,
    last_go_attempt_at_ms: Option<u128>,
    last_go_success_round_id: Option<u64>,
    light_pin: Option<u8>,
    last_light_raw: Option<u16>,
    last_light_percent: Option<f32>,
    last_ip: Option<String>,
    last_seen_ms: u128,
}

#[derive(Debug, Serialize)]
struct BuzzResponse {
    accepted: bool,
    message: String,
    state: BuzzerStateResponse,
}

#[derive(Debug, Serialize)]
struct AckResponse {
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct BuzzerStateResponse {
    round_id: u64,
    buzzes: Vec<BuzzEntry>,
    devices: Vec<DevicePresence>,
    online_window_ms: u128,
    live_at_us: Option<u128>,
    live_in_ms: Option<u128>,
    expected_buzzer_firmware_version: &'static str,
}

#[derive(Debug, Serialize)]
struct LightStateResponse {
    devices: Vec<DevicePresence>,
    light_readings: Vec<LightReading>,
    online_window_ms: u128,
    light_history_limit: usize,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

const ONLINE_WINDOW_MS: u128 = 30_000;
const LIGHT_HISTORY_LIMIT: usize = 3600;
const CALIBRATION_TIMEOUT_MS: u64 = 900;
const SCHEDULED_GO_DELAY_US: u128 = 1_000_000;
const EXPECTED_BUZZER_FIRMWARE_VERSION: &str = "2026.04.23.5";

#[derive(Debug, Clone)]
struct CalibrationTarget {
    player_id: String,
    device_id: Option<String>,
    mac_address: Option<String>,
    ip_address: String,
    port: u16,
    estimated_one_way_us: u128,
}

#[derive(Debug)]
struct CalibrationResult {
    target: CalibrationTarget,
    round_id: u64,
    rtt_us: Option<u128>,
    error: Option<String>,
}

#[tokio::main]
async fn main() {
    let state = AppState {
        inner: Arc::new(Mutex::new(HubState {
            next_order: 1,
            round_id: 0,
            live_started_at: None,
            live_at_us: None,
            buzzes: Vec::new(),
            devices: Vec::new(),
            light_readings: Vec::new(),
        })),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(get_buzzer_state))
        .route("/api/light-state", get(get_light_state))
        .route("/api/check-in", post(post_check_in))
        .route("/api/buzz", post(post_buzz))
        .route("/api/light-reading", post(post_light_reading))
        .route("/api/reset", post(reset_round))
        .route("/api/arm-live", post(arm_live_round))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8787));
    println!("Pico hub backend listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind backend listener");

    axum::serve(listener, app)
        .await
        .expect("backend server failed");
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn get_buzzer_state(State(state): State<AppState>) -> Json<BuzzerStateResponse> {
    Json(buzzer_snapshot(&state))
}

async fn get_light_state(State(state): State<AppState>) -> Json<LightStateResponse> {
    Json(light_snapshot(&state))
}

async fn post_buzz(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRequest>,
) -> impl IntoResponse {
    let player_id = resolve_player_id(&payload);
    let player_name = resolve_player_name(&payload);
    let app_kind = resolve_app_kind(&payload, "buzzer");

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    let seen_at_instant = Instant::now();
    let seen_at_us = now_us();
    let seen_at_ms = us_to_ms(seen_at_us);
    let reaction_us = guard.live_started_at.map(|live_started_at| {
        seen_at_instant
            .saturating_duration_since(live_started_at)
            .as_micros()
    });
    let client_round_id = payload.round_id;
    let client_reaction_us = payload
        .client_reaction_us
        .filter(|_| client_round_id == Some(guard.round_id));
    let client_timing_status =
        if client_round_id.is_some() && client_round_id != Some(guard.round_id) {
            Some("round_mismatch".to_string())
        } else {
            normalize_status(payload.client_timing_status.as_deref())
        };
    upsert_device(
        &mut guard,
        &payload,
        &player_id,
        player_name.as_deref(),
        &app_kind,
        seen_at_ms,
    );

    if guard
        .buzzes
        .iter()
        .any(|entry| entry.player_id == player_id)
    {
        let state = buzzer_state_response(&guard);

        return (
            StatusCode::OK,
            Json(BuzzResponse {
                accepted: false,
                message: format!("{} already buzzed this round", player_id),
                state,
            }),
        );
    }

    let entry = BuzzEntry {
        order: guard.next_order,
        round_id: guard.round_id,
        player_id: player_id.to_string(),
        player_name: player_name.clone(),
        device_id: payload.device_id,
        mac_address: payload.mac_address,
        button_pin: payload.button_pin,
        received_at_us: seen_at_us,
        received_at_ms: seen_at_ms,
        reaction_us,
        client_round_id,
        client_reaction_us,
        client_timing_status,
    };

    guard.next_order += 1;
    guard.buzzes.push(entry);

    let state = buzzer_state_response(&guard);

    (
        StatusCode::OK,
        Json(BuzzResponse {
            accepted: true,
            message: "buzz recorded".to_string(),
            state,
        }),
    )
}

async fn post_light_reading(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRequest>,
) -> impl IntoResponse {
    let Some(raw) = payload.light_raw else {
        return (
            StatusCode::BAD_REQUEST,
            Json(AckResponse {
                ok: false,
                message: "missing light_raw".to_string(),
            }),
        );
    };

    let Some(percent) = payload.light_percent else {
        return (
            StatusCode::BAD_REQUEST,
            Json(AckResponse {
                ok: false,
                message: "missing light_percent".to_string(),
            }),
        );
    };

    let player_id = resolve_player_id(&payload);
    let player_name = resolve_player_name(&payload);
    let app_kind = resolve_app_kind(&payload, "light-sensor");
    let seen_at_ms = now_ms();

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    upsert_device(
        &mut guard,
        &payload,
        &player_id,
        player_name.as_deref(),
        &app_kind,
        seen_at_ms,
    );
    push_light_reading(
        &mut guard,
        LightReading {
            player_id,
            player_name,
            device_id: payload.device_id,
            mac_address: payload.mac_address,
            light_pin: payload.light_pin,
            raw,
            percent: sanitize_percent(percent),
            received_at_ms: seen_at_ms,
        },
    );

    (
        StatusCode::OK,
        Json(AckResponse {
            ok: true,
            message: "light reading recorded".to_string(),
        }),
    )
}

async fn post_check_in(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRequest>,
) -> impl IntoResponse {
    let player_id = resolve_player_id(&payload);
    let player_name = resolve_player_name(&payload);
    let app_kind = resolve_app_kind(&payload, "buzzer");

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    upsert_device(
        &mut guard,
        &payload,
        &player_id,
        player_name.as_deref(),
        &app_kind,
        now_ms(),
    );

    (
        StatusCode::OK,
        Json(AckResponse {
            ok: true,
            message: "check-in recorded".to_string(),
        }),
    )
}

async fn reset_round(State(state): State<AppState>) -> Json<BuzzerStateResponse> {
    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    guard.next_order = 1;
    guard.round_id = guard.round_id.saturating_add(1);
    guard.live_started_at = None;
    guard.live_at_us = None;
    guard.buzzes.clear();

    Json(buzzer_state_response(&guard))
}

async fn arm_live_round(State(state): State<AppState>) -> Json<BuzzerStateResponse> {
    let (response, targets, round_id, live_at_us) = {
        let mut guard = state
            .inner
            .lock()
            .expect("hub state lock should not be poisoned");

        guard.next_order = 1;
        guard.round_id = guard.round_id.saturating_add(1);
        guard.live_started_at =
            Some(Instant::now() + Duration::from_micros(SCHEDULED_GO_DELAY_US as u64));
        guard.live_at_us = Some(now_us().saturating_add(SCHEDULED_GO_DELAY_US));
        guard.buzzes.clear();

        let targets = calibration_targets(&mut guard);
        let response = buzzer_state_response(&guard);
        let live_at_us = guard.live_at_us.expect("live_at_us was just set");

        (response, targets, guard.round_id, live_at_us)
    };

    let mut calibration_tasks = Vec::new();

    for target in targets {
        calibration_tasks.push(tokio::spawn(run_calibration_ping(
            target, round_id, live_at_us,
        )));
    }

    let has_calibration_tasks = !calibration_tasks.is_empty();

    for task in calibration_tasks {
        match task.await {
            Ok(result) => record_calibration_result(&state, result),
            Err(error) => eprintln!("calibration task failed: {}", error),
        }
    }

    if has_calibration_tasks {
        Json(buzzer_snapshot(&state))
    } else {
        Json(response)
    }
}

fn buzzer_snapshot(state: &AppState) -> BuzzerStateResponse {
    let guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    buzzer_state_response(&guard)
}

fn light_snapshot(state: &AppState) -> LightStateResponse {
    let guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    light_state_response(&guard)
}

fn buzzer_state_response(state: &HubState) -> BuzzerStateResponse {
    BuzzerStateResponse {
        round_id: state.round_id,
        buzzes: state.buzzes.clone(),
        devices: sorted_devices(&state.devices),
        online_window_ms: ONLINE_WINDOW_MS,
        live_at_us: state.live_at_us,
        live_in_ms: state
            .live_at_us
            .map(|live_at_us| us_to_ms(live_at_us.saturating_sub(now_us()))),
        expected_buzzer_firmware_version: EXPECTED_BUZZER_FIRMWARE_VERSION,
    }
}

fn light_state_response(state: &HubState) -> LightStateResponse {
    LightStateResponse {
        devices: sorted_devices(&state.devices),
        light_readings: sorted_light_readings(&state.light_readings),
        online_window_ms: ONLINE_WINDOW_MS,
        light_history_limit: LIGHT_HISTORY_LIMIT,
    }
}

fn sorted_devices(devices: &[DevicePresence]) -> Vec<DevicePresence> {
    let mut sorted = devices.to_vec();
    sorted.sort_by(|left, right| {
        left.player_id
            .cmp(&right.player_id)
            .then_with(|| left.app_kind.cmp(&right.app_kind))
            .then_with(|| right.last_seen_ms.cmp(&left.last_seen_ms))
    });
    sorted
}

fn sorted_light_readings(readings: &[LightReading]) -> Vec<LightReading> {
    let mut sorted = readings.to_vec();
    sorted.sort_by(|left, right| {
        left.player_id
            .cmp(&right.player_id)
            .then_with(|| left.received_at_ms.cmp(&right.received_at_ms))
    });
    sorted
}

fn calibration_targets(state: &mut HubState) -> Vec<CalibrationTarget> {
    let now = now_ms();
    let current_round_id = state.round_id;

    state
        .devices
        .iter_mut()
        .filter(|device| normalize_app_kind(&device.app_kind) == "buzzer")
        .filter(|device| now.saturating_sub(device.last_seen_ms) <= ONLINE_WINDOW_MS)
        .filter_map(|device| {
            let ip_address = device.last_ip.clone()?;
            let port = device.calibration_port?;

            device.last_go_attempt_round_id = Some(current_round_id);
            device.last_go_attempt_at_ms = Some(now);

            Some(CalibrationTarget {
                player_id: device.player_id.clone(),
                device_id: device.device_id.clone(),
                mac_address: device.mac_address.clone(),
                ip_address,
                port,
                estimated_one_way_us: device
                    .last_calibration_rtt_us
                    .or(device.last_client_checkin_rtt_us)
                    .unwrap_or(0)
                    / 2,
            })
        })
        .collect()
}

async fn run_calibration_ping(
    target: CalibrationTarget,
    round_id: u64,
    live_at_us: u128,
) -> CalibrationResult {
    let outcome = send_calibration_request(&target, round_id, live_at_us).await;

    match outcome {
        Ok(rtt_us) => CalibrationResult {
            target,
            round_id,
            rtt_us: Some(rtt_us),
            error: None,
        },
        Err(error) => CalibrationResult {
            target,
            round_id,
            rtt_us: None,
            error: Some(error),
        },
    }
}

async fn send_calibration_request(
    target: &CalibrationTarget,
    round_id: u64,
    live_at_us: u128,
) -> Result<u128, String> {
    let address = format!("{}:{}", target.ip_address, target.port);
    let request_started_us = now_us();
    let delay_until_live_us = live_at_us.saturating_sub(request_started_us);
    let adjusted_delay_us = delay_until_live_us.saturating_sub(target.estimated_one_way_us);
    let request = format!(
        "GET /go?round_id={}&go_us={}&delay_us={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        round_id, live_at_us, adjusted_delay_us, address
    );
    let started_at = Instant::now();

    let response = timeout(Duration::from_millis(CALIBRATION_TIMEOUT_MS), async {
        let mut stream = TcpStream::connect(&address)
            .await
            .map_err(|error| format!("connect: {}", error))?;

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|error| format!("write: {}", error))?;

        let mut response = Vec::new();
        let mut buffer = [0_u8; 512];

        loop {
            match stream.read(&mut buffer).await {
                Ok(0) => break,
                Ok(byte_count) => response.extend_from_slice(&buffer[..byte_count]),
                Err(_error) if !response.is_empty() => break,
                Err(error) => return Err(format!("read: {}", error)),
            }
        }

        Ok::<_, String>(response)
    })
    .await
    .map_err(|_| "timeout".to_string())??;

    let rtt_us = started_at.elapsed().as_micros();

    if !(response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.0 200")) {
        return Err(response_status(&response));
    }

    let body = response_body(&response);

    if !json_body_contains_u64(&body, "round_id", round_id) {
        return Err(format!("ack missing round {}", round_id));
    }

    if !json_body_contains_bool(&body, "go_ticks_set", true) {
        return Err("ack did not set go_ticks".to_string());
    }

    Ok(rtt_us)
}

fn response_status(response: &[u8]) -> String {
    let status_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let status = String::from_utf8_lossy(status_line).trim().to_string();

    if status.is_empty() {
        "empty response".to_string()
    } else {
        status
    }
}

fn response_body(response: &[u8]) -> String {
    let Some(body_start) = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    else {
        return String::new();
    };

    String::from_utf8_lossy(&response[body_start..]).to_string()
}

fn json_body_contains_u64(body: &str, field: &str, value: u64) -> bool {
    let compact_pattern = format!("\"{}\":{}", field, value);
    let spaced_pattern = format!("\"{}\": {}", field, value);

    body.contains(&compact_pattern) || body.contains(&spaced_pattern)
}

fn json_body_contains_bool(body: &str, field: &str, value: bool) -> bool {
    let expected = if value { "true" } else { "false" };
    let compact_pattern = format!("\"{}\":{}", field, expected);
    let spaced_pattern = format!("\"{}\": {}", field, expected);

    body.contains(&compact_pattern) || body.contains(&spaced_pattern)
}

fn record_calibration_result(state: &AppState, result: CalibrationResult) {
    let measured_at_ms = now_ms();
    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    let Some(device) = guard
        .devices
        .iter_mut()
        .find(|device| same_calibration_target(device, &result.target))
    else {
        return;
    };

    device.last_calibration_round_id = Some(result.round_id);
    device.last_calibration_at_ms = Some(measured_at_ms);

    if let Some(rtt_us) = result.rtt_us {
        device.last_calibration_jitter_us = device
            .last_calibration_rtt_us
            .map(|previous_rtt_us| previous_rtt_us.abs_diff(rtt_us));
        device.last_calibration_rtt_us = Some(rtt_us);
        device.last_calibration_error = None;
        device.last_go_success_round_id = Some(result.round_id);
        device.last_seen_ms = measured_at_ms;
    } else {
        device.last_calibration_error = result.error.map(|value| value.chars().take(80).collect());
    }
}

fn upsert_device(
    state: &mut HubState,
    payload: &DeviceRequest,
    player_id: &str,
    player_name: Option<&str>,
    app_kind: &str,
    seen_at_ms: u128,
) {
    if let Some(device) = state
        .devices
        .iter_mut()
        .find(|device| same_device(device, payload, player_id))
    {
        device.player_id = player_id.to_string();
        device.player_name = player_name.map(str::to_string);
        device.app_kind = app_kind.to_string();
        device.last_seen_ms = seen_at_ms;

        if let Some(device_id) = &payload.device_id {
            device.device_id = Some(device_id.clone());
        }

        if let Some(mac_address) = &payload.mac_address {
            device.mac_address = Some(mac_address.clone());
        }

        if let Some(button_pin) = payload.button_pin {
            device.button_pin = Some(button_pin);
        }

        if let Some(button_pressed) = payload.button_pressed {
            device.button_pressed = button_pressed;
        }

        if let Some(calibration_port) = payload.calibration_port {
            device.calibration_port = Some(calibration_port);
        }

        if let Some(firmware_version) =
            normalize_firmware_version(payload.firmware_version.as_deref())
        {
            device.firmware_version = Some(firmware_version);
        }

        if let Some(client_checkin_rtt_us) = payload.client_checkin_rtt_us {
            device.last_client_checkin_jitter_us = device
                .last_client_checkin_rtt_us
                .map(|previous_rtt_us| previous_rtt_us.abs_diff(client_checkin_rtt_us));
            device.last_client_checkin_rtt_us = Some(client_checkin_rtt_us);
            device.last_client_checkin_at_ms = Some(seen_at_ms);
        }

        if payload.client_go_round_id.is_some() || payload.client_go_ticks_set.is_some() {
            device.last_client_go_round_id = payload.client_go_round_id;
            device.last_client_go_ticks_set = payload.client_go_ticks_set;
            device.last_client_go_seen_at_ms = Some(seen_at_ms);
        }

        if let Some(light_pin) = payload.light_pin {
            device.light_pin = Some(light_pin);
        }

        if let Some(light_raw) = payload.light_raw {
            device.last_light_raw = Some(light_raw);
        }

        if let Some(light_percent) = payload.light_percent {
            device.last_light_percent = Some(sanitize_percent(light_percent));
        }

        if let Some(ip_address) = &payload.ip_address {
            device.last_ip = Some(ip_address.clone());
        }

        return;
    }

    state.devices.push(DevicePresence {
        player_id: player_id.to_string(),
        player_name: player_name.map(str::to_string),
        app_kind: app_kind.to_string(),
        device_id: payload.device_id.clone(),
        mac_address: payload.mac_address.clone(),
        button_pin: payload.button_pin,
        button_pressed: payload.button_pressed.unwrap_or(false),
        calibration_port: payload.calibration_port,
        firmware_version: normalize_firmware_version(payload.firmware_version.as_deref()),
        last_client_checkin_rtt_us: payload.client_checkin_rtt_us,
        last_client_checkin_jitter_us: None,
        last_client_checkin_at_ms: payload.client_checkin_rtt_us.map(|_| seen_at_ms),
        last_client_go_round_id: payload.client_go_round_id,
        last_client_go_ticks_set: payload.client_go_ticks_set,
        last_client_go_seen_at_ms: if payload.client_go_round_id.is_some()
            || payload.client_go_ticks_set.is_some()
        {
            Some(seen_at_ms)
        } else {
            None
        },
        last_calibration_round_id: None,
        last_calibration_rtt_us: None,
        last_calibration_jitter_us: None,
        last_calibration_at_ms: None,
        last_calibration_error: None,
        last_go_attempt_round_id: None,
        last_go_attempt_at_ms: None,
        last_go_success_round_id: None,
        light_pin: payload.light_pin,
        last_light_raw: payload.light_raw,
        last_light_percent: payload.light_percent.map(sanitize_percent),
        last_ip: payload.ip_address.clone(),
        last_seen_ms: seen_at_ms,
    });
}

fn push_light_reading(state: &mut HubState, reading: LightReading) {
    let player_id = reading.player_id.clone();
    state.light_readings.push(reading);

    let player_count = state
        .light_readings
        .iter()
        .filter(|entry| entry.player_id == player_id)
        .count();

    let mut remaining_to_remove = player_count.saturating_sub(LIGHT_HISTORY_LIMIT);

    if remaining_to_remove == 0 {
        return;
    }

    state.light_readings.retain(|entry| {
        if remaining_to_remove > 0 && entry.player_id == player_id {
            remaining_to_remove -= 1;
            return false;
        }

        true
    });
}

fn same_device(device: &DevicePresence, payload: &DeviceRequest, player_id: &str) -> bool {
    if let (Some(existing_id), Some(incoming_id)) = (&device.device_id, &payload.device_id) {
        if existing_id == incoming_id {
            return true;
        }
    }

    if let (Some(existing_mac), Some(incoming_mac)) = (&device.mac_address, &payload.mac_address) {
        if existing_mac == incoming_mac {
            return true;
        }
    }

    if let (Some(existing_ip), Some(incoming_ip)) = (&device.last_ip, &payload.ip_address) {
        if existing_ip == incoming_ip {
            return true;
        }
    }

    device.player_id == player_id
}

fn same_calibration_target(device: &DevicePresence, target: &CalibrationTarget) -> bool {
    if let (Some(existing_id), Some(target_id)) = (&device.device_id, &target.device_id) {
        if existing_id == target_id {
            return true;
        }
    }

    if let (Some(existing_mac), Some(target_mac)) = (&device.mac_address, &target.mac_address) {
        if existing_mac == target_mac {
            return true;
        }
    }

    if device.last_ip.as_deref() == Some(target.ip_address.as_str())
        && device.calibration_port == Some(target.port)
    {
        return true;
    }

    device.player_id == target.player_id
}

fn resolve_player_id(payload: &DeviceRequest) -> String {
    if let Some(ip_address) = payload.ip_address.as_deref() {
        if let Some(last_octet) = ip_address.rsplit('.').next() {
            if !last_octet.is_empty() && last_octet.chars().all(|char| char.is_ascii_digit()) {
                return format!("player-{}", last_octet);
            }
        }
    }

    if let Some(mac_address) = payload.mac_address.as_deref() {
        let compact: String = mac_address
            .chars()
            .filter(|char| char.is_ascii_hexdigit())
            .collect();

        if compact.len() >= 4 {
            return format!("player-{}", &compact[compact.len() - 4..]);
        }
    }

    if let Some(device_id) = payload.device_id.as_deref() {
        let suffix_start = device_id.len().saturating_sub(4);
        return format!("player-{}", &device_id[suffix_start..]);
    }

    "player-unknown".to_string()
}

fn resolve_player_name(payload: &DeviceRequest) -> Option<String> {
    payload
        .player_name
        .as_deref()
        .or(payload.reported_player_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_app_kind(payload: &DeviceRequest, default_kind: &str) -> String {
    if let Some(app_kind) = payload.app_kind.as_deref() {
        return normalize_app_kind(app_kind);
    }

    if payload.light_raw.is_some() || payload.light_pin.is_some() || payload.light_percent.is_some()
    {
        return "light-sensor".to_string();
    }

    if payload.button_pin.is_some() || payload.button_pressed.is_some() {
        return "buzzer".to_string();
    }

    normalize_app_kind(default_kind)
}

fn normalize_app_kind(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "light" | "light-sensor" | "lightsensor" | "sensor" | "photoresistor" => {
            "light-sensor".to_string()
        }
        "buzzer" | "buzzer-game" | "game" => "buzzer".to_string(),
        "" => "generic".to_string(),
        _ => normalized,
    }
}

fn normalize_firmware_version(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(48).collect())
}

fn normalize_status(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(48).collect())
}

fn sanitize_percent(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }

    value.clamp(0.0, 100.0)
}

fn now_ms() -> u128 {
    us_to_ms(now_us())
}

fn now_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_micros()
}

fn us_to_ms(value: u128) -> u128 {
    value / 1000
}
