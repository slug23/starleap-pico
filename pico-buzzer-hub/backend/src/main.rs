use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<HubState>>,
}

#[derive(Debug, Default)]
struct HubState {
    next_order: u64,
    buzzes: Vec<BuzzEntry>,
    devices: Vec<DevicePresence>,
    light_readings: Vec<LightReading>,
}

#[derive(Debug, Clone, Serialize)]
struct BuzzEntry {
    order: u64,
    player_id: String,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    received_at_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
struct LightReading {
    player_id: String,
    device_id: Option<String>,
    mac_address: Option<String>,
    light_pin: Option<u8>,
    raw: u16,
    percent: f32,
    received_at_ms: u128,
}

#[derive(Debug, Deserialize)]
struct DeviceRequest {
    #[serde(default, rename = "player_id", alias = "student_id")]
    _reported_player_id: Option<String>,
    app_kind: Option<String>,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    button_pressed: Option<bool>,
    light_pin: Option<u8>,
    light_raw: Option<u16>,
    light_percent: Option<f32>,
    ip_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DevicePresence {
    player_id: String,
    app_kind: String,
    device_id: Option<String>,
    mac_address: Option<String>,
    button_pin: Option<u8>,
    button_pressed: bool,
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
    buzzes: Vec<BuzzEntry>,
    devices: Vec<DevicePresence>,
    online_window_ms: u128,
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

#[tokio::main]
async fn main() {
    let state = AppState {
        inner: Arc::new(Mutex::new(HubState {
            next_order: 1,
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
    let app_kind = resolve_app_kind(&payload, "buzzer");

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    let seen_at_ms = now_ms();
    upsert_device(&mut guard, &payload, &player_id, &app_kind, seen_at_ms);

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
        player_id: player_id.to_string(),
        device_id: payload.device_id,
        mac_address: payload.mac_address,
        button_pin: payload.button_pin,
        received_at_ms: seen_at_ms,
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
    let app_kind = resolve_app_kind(&payload, "light-sensor");
    let seen_at_ms = now_ms();

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    upsert_device(&mut guard, &payload, &player_id, &app_kind, seen_at_ms);
    push_light_reading(
        &mut guard,
        LightReading {
            player_id,
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
    let app_kind = resolve_app_kind(&payload, "buzzer");

    let mut guard = state
        .inner
        .lock()
        .expect("hub state lock should not be poisoned");

    upsert_device(&mut guard, &payload, &player_id, &app_kind, now_ms());

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
    guard.buzzes.clear();

    Json(buzzer_state_response(&guard))
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
        buzzes: state.buzzes.clone(),
        devices: sorted_devices(&state.devices),
        online_window_ms: ONLINE_WINDOW_MS,
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

fn upsert_device(
    state: &mut HubState,
    payload: &DeviceRequest,
    player_id: &str,
    app_kind: &str,
    seen_at_ms: u128,
) {
    if let Some(device) = state
        .devices
        .iter_mut()
        .find(|device| same_device(device, payload, player_id))
    {
        device.player_id = player_id.to_string();
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
        app_kind: app_kind.to_string(),
        device_id: payload.device_id.clone(),
        mac_address: payload.mac_address.clone(),
        button_pin: payload.button_pin,
        button_pressed: payload.button_pressed.unwrap_or(false),
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

fn sanitize_percent(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }

    value.clamp(0.0, 100.0)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis()
}
