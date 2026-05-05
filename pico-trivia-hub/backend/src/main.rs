use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    net::SocketAddr,
    path::PathBuf,
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
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use serde::{Deserialize, Serialize};
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
};

const ONLINE_WINDOW_MS: u128 = 30_000;
const SAVE_FILE_NAME: &str = "save.json";
const QUESTIONS_PATH: &str = "../questions/questions.json";
const IMAGES_DIR: &str = "../questions/images";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Question {
    id: String,
    category: String,
    prompt: String,
    answer: String,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    image_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuestionPool {
    version: String,
    title: String,
    questions: Vec<Question>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GamePhase {
    CheckIn,
    QuestionOpen,
    Answering,
    BetweenQuestions,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuzzEntry {
    player_id: String,
    player_name: Option<String>,
    device_id: Option<String>,
    received_at_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevicePresence {
    player_id: String,
    player_name: Option<String>,
    device_id: Option<String>,
    button_pin: Option<u8>,
    button_pressed: bool,
    firmware_version: Option<String>,
    last_ip: Option<String>,
    last_seen_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TriviaGame {
    game_id: String,
    shuffled: bool,
    question_order: Vec<String>,
    current_index: usize,
    phase: GamePhase,
    locked_out: HashSet<String>,
    buzz_queue: VecDeque<BuzzEntry>,
    current_answerer: Option<BuzzEntry>,
    scores: HashMap<String, i64>,
    player_names: HashMap<String, String>,
    show_leaderboard: bool,
    answer_revealed: bool,
    started_at_ms: u128,
    last_saved_at_ms: u128,
    questions_skipped: HashSet<String>,
    questions_correct: HashSet<String>,
    #[serde(default)]
    checked_in: HashSet<String>,
    #[serde(default)]
    show_check_in: bool,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<HubState>>,
}

struct HubState {
    pool: QuestionPool,
    game: Option<TriviaGame>,
    devices: Vec<DevicePresence>,
    save_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct DeviceRequest {
    #[serde(default)]
    player_name: Option<String>,
    #[serde(default, rename = "player_id", alias = "student_id")]
    reported_player_id: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    mac_address: Option<String>,
    #[serde(default)]
    button_pin: Option<u8>,
    #[serde(default)]
    button_pressed: Option<bool>,
    #[serde(default)]
    firmware_version: Option<String>,
    #[serde(default)]
    ip_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartGameRequest {
    #[serde(default)]
    shuffled: bool,
    #[serde(default)]
    include_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct JudgeRequest {
    correct: bool,
}

#[derive(Debug, Serialize)]
struct StateResponse {
    pool_version: String,
    pool_title: String,
    pool_size: usize,
    online_window_ms: u128,
    devices: Vec<DevicePresence>,
    game: Option<GameSnapshot>,
    save_info: SaveInfo,
}

#[derive(Debug, Serialize)]
struct GameSnapshot {
    game_id: String,
    shuffled: bool,
    phase: GamePhase,
    current_index: usize,
    total_questions: usize,
    current_question: Option<Question>,
    locked_out: Vec<String>,
    buzz_queue: Vec<BuzzEntry>,
    current_answerer: Option<BuzzEntry>,
    checked_in: Vec<String>,
    show_check_in: bool,
    scores: Vec<ScoreEntry>,
    show_leaderboard: bool,
    answer_revealed: bool,
    started_at_ms: u128,
    last_saved_at_ms: u128,
    finished: bool,
}

#[derive(Debug, Serialize)]
struct ScoreEntry {
    player_id: String,
    player_name: Option<String>,
    score: i64,
}

#[derive(Debug, Serialize)]
struct SaveInfo {
    exists: bool,
    summary: Option<SaveSummary>,
}

#[derive(Debug, Serialize)]
struct SaveSummary {
    started_at_ms: u128,
    last_saved_at_ms: u128,
    current_index: usize,
    total_questions: usize,
    player_count: usize,
    finished: bool,
}

#[derive(Debug, Serialize)]
struct BuzzResponse {
    accepted: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct AckResponse {
    ok: bool,
    message: String,
}

#[tokio::main]
async fn main() {
    let backend_root = std::env::current_dir().expect("could not get current dir");
    let pool = load_pool();
    let save_path = backend_root.join("data").join(SAVE_FILE_NAME);

    let game = match load_save(&save_path) {
        Ok(loaded) => {
            println!("Loaded save: index {} / {}", loaded.current_index, loaded.question_order.len());
            Some(loaded)
        }
        Err(error) => {
            println!("No usable save file ({error}); starting fresh.");
            None
        }
    };

    let state = AppState {
        inner: Arc::new(Mutex::new(HubState {
            pool,
            game,
            devices: Vec::new(),
            save_path,
        })),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/questions", get(get_questions))
        .route("/api/state", get(get_state))
        .route("/api/save-info", get(get_save_info))
        .route("/api/game/start", post(start_game))
        .route("/api/game/resume", post(resume_game))
        .route("/api/game/end", post(end_game))
        .route("/api/game/begin-questions", post(begin_questions))
        .route("/api/game/show-check-in", post(show_check_in_overlay))
        .route("/api/game/judge", post(judge))
        .route("/api/game/skip", post(skip_question))
        .route("/api/game/next", post(next_question))
        .route("/api/game/reveal", post(reveal_answer))
        .route("/api/view/toggle", post(toggle_leaderboard))
        .route("/api/buzz", post(post_buzz))
        .route("/api/check-in", post(post_check_in))
        .nest_service("/images", ServeDir::new(IMAGES_DIR))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8787));
    println!("Pico trivia hub listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind backend listener");

    axum::serve(listener, app)
        .await
        .expect("backend server failed");
}

async fn health() -> Json<AckResponse> {
    Json(AckResponse {
        ok: true,
        message: "ok".to_string(),
    })
}

async fn get_questions(State(state): State<AppState>) -> Json<QuestionPool> {
    let guard = state.inner.lock().expect("hub state lock poisoned");
    Json(guard.pool.clone())
}

async fn get_state(State(state): State<AppState>) -> Json<StateResponse> {
    let guard = state.inner.lock().expect("hub state lock poisoned");
    Json(snapshot(&guard))
}

async fn get_save_info(State(state): State<AppState>) -> Json<SaveInfo> {
    let guard = state.inner.lock().expect("hub state lock poisoned");
    Json(read_save_info(&guard.save_path))
}

async fn start_game(
    State(state): State<AppState>,
    Json(payload): Json<StartGameRequest>,
) -> impl IntoResponse {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    let pool_ids: Vec<String> = match payload.include_ids {
        Some(ids) if !ids.is_empty() => ids
            .into_iter()
            .filter(|id| guard.pool.questions.iter().any(|q| &q.id == id))
            .collect(),
        _ => guard.pool.questions.iter().map(|q| q.id.clone()).collect(),
    };

    if pool_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "message": "no questions to play" })),
        );
    }

    let mut order = pool_ids;

    if payload.shuffled {
        let mut rng = StdRng::from_entropy();
        order.shuffle(&mut rng);
    }

    let now_ms = now_ms();

    let game = TriviaGame {
        game_id: format!("game-{}", now_ms),
        shuffled: payload.shuffled,
        question_order: order,
        current_index: 0,
        phase: GamePhase::CheckIn,
        locked_out: HashSet::new(),
        buzz_queue: VecDeque::new(),
        current_answerer: None,
        scores: HashMap::new(),
        player_names: HashMap::new(),
        show_leaderboard: false,
        answer_revealed: false,
        started_at_ms: now_ms,
        last_saved_at_ms: now_ms,
        questions_skipped: HashSet::new(),
        questions_correct: HashSet::new(),
        checked_in: HashSet::new(),
        show_check_in: false,
    };

    guard.game = Some(game);
    persist(&mut guard);

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "state": snapshot(&guard) })),
    )
}

async fn resume_game(State(state): State<AppState>) -> impl IntoResponse {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if guard.game.is_some() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "state": snapshot(&guard) })),
        );
    }

    match load_save(&guard.save_path) {
        Ok(game) => {
            guard.game = Some(game);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "ok": true, "state": snapshot(&guard) })),
            )
        }
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "message": format!("{error}") })),
        ),
    }
}

async fn end_game(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");
    guard.game = None;
    let _ = fs::remove_file(&guard.save_path);
    Json(snapshot(&guard))
}

async fn judge(
    State(state): State<AppState>,
    Json(payload): Json<JudgeRequest>,
) -> impl IntoResponse {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    let Some(game) = guard.game.as_mut() else {
        return (StatusCode::BAD_REQUEST, Json(snapshot(&guard)));
    };

    if game.phase != GamePhase::Answering {
        return (StatusCode::OK, Json(snapshot(&guard)));
    }

    let Some(answerer) = game.current_answerer.take() else {
        return (StatusCode::OK, Json(snapshot(&guard)));
    };

    if payload.correct {
        let entry = game.scores.entry(answerer.player_id.clone()).or_insert(0);
        *entry += 1;

        if let Some(name) = answerer.player_name.clone() {
            game.player_names.insert(answerer.player_id.clone(), name);
        }

        if let Some(qid) = game.question_order.get(game.current_index).cloned() {
            game.questions_correct.insert(qid);
        }

        game.phase = GamePhase::BetweenQuestions;
        game.answer_revealed = true;
    } else if let Some(next) = game.buzz_queue.pop_front() {
        game.current_answerer = Some(next);
    } else {
        game.phase = GamePhase::QuestionOpen;
    }

    persist(&mut guard);

    (StatusCode::OK, Json(snapshot(&guard)))
}

async fn skip_question(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        if matches!(
            game.phase,
            GamePhase::QuestionOpen | GamePhase::Answering | GamePhase::BetweenQuestions
        ) {
            if let Some(qid) = game.question_order.get(game.current_index).cloned() {
                if !game.questions_correct.contains(&qid) {
                    game.questions_skipped.insert(qid);
                }
            }
            game.current_answerer = None;
            game.buzz_queue.clear();
            game.phase = GamePhase::BetweenQuestions;
            game.answer_revealed = true;
        }
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn next_question(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        let next_index = game.current_index + 1;

        game.locked_out.clear();
        game.buzz_queue.clear();
        game.current_answerer = None;
        game.answer_revealed = false;

        if next_index >= game.question_order.len() {
            game.phase = GamePhase::Finished;
        } else {
            game.current_index = next_index;
            game.phase = GamePhase::QuestionOpen;
        }
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn begin_questions(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        if game.phase == GamePhase::CheckIn {
            game.phase = GamePhase::QuestionOpen;
        }
        game.show_check_in = false;
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn show_check_in_overlay(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        if game.phase != GamePhase::CheckIn {
            game.show_check_in = true;
            game.checked_in.clear();
        }
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn reveal_answer(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        game.answer_revealed = true;
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn toggle_leaderboard(State(state): State<AppState>) -> Json<StateResponse> {
    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    if let Some(game) = guard.game.as_mut() {
        game.show_leaderboard = !game.show_leaderboard;
    }

    persist(&mut guard);
    Json(snapshot(&guard))
}

async fn post_buzz(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRequest>,
) -> impl IntoResponse {
    let player_id = resolve_player_id(&payload);
    let player_name = resolve_player_name(&payload);
    let now_ms = now_ms();

    let mut guard = state.inner.lock().expect("hub state lock poisoned");

    upsert_device(
        &mut guard.devices,
        &payload,
        &player_id,
        player_name.as_deref(),
        now_ms,
    );

    let Some(game) = guard.game.as_mut() else {
        return (
            StatusCode::OK,
            Json(BuzzResponse {
                accepted: false,
                message: "no game running".to_string(),
            }),
        );
    };

    if let Some(name) = player_name.clone() {
        game.player_names.insert(player_id.clone(), name);
    }

    game.scores.entry(player_id.clone()).or_insert(0);

    if game.phase == GamePhase::CheckIn || game.show_check_in {
        game.checked_in.insert(player_id.clone());
        persist(&mut guard);
        return (
            StatusCode::OK,
            Json(BuzzResponse {
                accepted: true,
                message: "checked in".to_string(),
            }),
        );
    }

    if !matches!(game.phase, GamePhase::QuestionOpen | GamePhase::Answering) {
        return (
            StatusCode::OK,
            Json(BuzzResponse {
                accepted: false,
                message: "buzzer not open".to_string(),
            }),
        );
    }

    if game.locked_out.contains(&player_id) {
        return (
            StatusCode::OK,
            Json(BuzzResponse {
                accepted: false,
                message: "already buzzed this question".to_string(),
            }),
        );
    }

    game.locked_out.insert(player_id.clone());

    let entry = BuzzEntry {
        player_id: player_id.clone(),
        player_name,
        device_id: payload.device_id.clone(),
        received_at_ms: now_ms,
    };

    if game.phase == GamePhase::QuestionOpen && game.current_answerer.is_none() {
        game.current_answerer = Some(entry);
        game.phase = GamePhase::Answering;
    } else {
        game.buzz_queue.push_back(entry);
    }

    persist(&mut guard);

    (
        StatusCode::OK,
        Json(BuzzResponse {
            accepted: true,
            message: "buzz recorded".to_string(),
        }),
    )
}

async fn post_check_in(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRequest>,
) -> Json<AckResponse> {
    let player_id = resolve_player_id(&payload);
    let player_name = resolve_player_name(&payload);
    let now_ms = now_ms();

    let mut guard = state.inner.lock().expect("hub state lock poisoned");
    upsert_device(
        &mut guard.devices,
        &payload,
        &player_id,
        player_name.as_deref(),
        now_ms,
    );

    if let Some(game) = guard.game.as_mut() {
        game.scores.entry(player_id.clone()).or_insert(0);
        if let Some(name) = player_name {
            game.player_names.insert(player_id, name);
        }
    }

    Json(AckResponse {
        ok: true,
        message: "check-in recorded".to_string(),
    })
}

fn snapshot(state: &HubState) -> StateResponse {
    StateResponse {
        pool_version: state.pool.version.clone(),
        pool_title: state.pool.title.clone(),
        pool_size: state.pool.questions.len(),
        online_window_ms: ONLINE_WINDOW_MS,
        devices: state.devices.clone(),
        game: state.game.as_ref().map(|game| game_snapshot(game, &state.pool)),
        save_info: read_save_info(&state.save_path),
    }
}

fn game_snapshot(game: &TriviaGame, pool: &QuestionPool) -> GameSnapshot {
    let current_question_id = game.question_order.get(game.current_index).cloned();
    let current_question = current_question_id
        .as_ref()
        .and_then(|id| pool.questions.iter().find(|q| &q.id == id).cloned());

    let mut scores: Vec<ScoreEntry> = game
        .scores
        .iter()
        .map(|(player_id, score)| ScoreEntry {
            player_id: player_id.clone(),
            player_name: game.player_names.get(player_id).cloned(),
            score: *score,
        })
        .collect();

    scores.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.player_id.cmp(&right.player_id))
    });

    GameSnapshot {
        game_id: game.game_id.clone(),
        shuffled: game.shuffled,
        phase: game.phase,
        current_index: game.current_index,
        total_questions: game.question_order.len(),
        current_question,
        locked_out: game.locked_out.iter().cloned().collect(),
        buzz_queue: game.buzz_queue.iter().cloned().collect(),
        current_answerer: game.current_answerer.clone(),
        checked_in: game.checked_in.iter().cloned().collect(),
        show_check_in: game.show_check_in,
        scores,
        show_leaderboard: game.show_leaderboard,
        answer_revealed: game.answer_revealed,
        started_at_ms: game.started_at_ms,
        last_saved_at_ms: game.last_saved_at_ms,
        finished: game.phase == GamePhase::Finished,
    }
}

fn persist(state: &mut HubState) {
    let Some(game) = state.game.as_mut() else {
        return;
    };

    game.last_saved_at_ms = now_ms();

    if let Some(parent) = state.save_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let temp_path = state.save_path.with_extension("json.tmp");

    let json = match serde_json::to_string_pretty(game) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("failed to serialize save: {error}");
            return;
        }
    };

    if let Err(error) = fs::write(&temp_path, json) {
        eprintln!("failed to write temp save: {error}");
        return;
    }

    if let Err(error) = fs::rename(&temp_path, &state.save_path) {
        eprintln!("failed to commit save: {error}");
    }
}

fn read_save_info(path: &PathBuf) -> SaveInfo {
    match load_save(path) {
        Ok(game) => SaveInfo {
            exists: true,
            summary: Some(SaveSummary {
                started_at_ms: game.started_at_ms,
                last_saved_at_ms: game.last_saved_at_ms,
                current_index: game.current_index,
                total_questions: game.question_order.len(),
                player_count: game.scores.len(),
                finished: game.phase == GamePhase::Finished,
            }),
        },
        Err(_) => SaveInfo {
            exists: false,
            summary: None,
        },
    }
}

fn load_save(path: &PathBuf) -> Result<TriviaGame, String> {
    let contents = fs::read_to_string(path).map_err(|error| format!("read: {error}"))?;
    serde_json::from_str(&contents).map_err(|error| format!("parse: {error}"))
}

fn load_pool() -> QuestionPool {
    let contents = fs::read_to_string(QUESTIONS_PATH)
        .unwrap_or_else(|error| panic!("could not read {QUESTIONS_PATH}: {error}"));
    serde_json::from_str(&contents)
        .unwrap_or_else(|error| panic!("could not parse questions.json: {error}"))
}

fn upsert_device(
    devices: &mut Vec<DevicePresence>,
    payload: &DeviceRequest,
    player_id: &str,
    player_name: Option<&str>,
    seen_at_ms: u128,
) {
    if let Some(device) = devices.iter_mut().find(|device| {
        device.device_id.as_deref() == payload.device_id.as_deref()
            || device.player_id == player_id
    }) {
        device.player_id = player_id.to_string();
        device.player_name = player_name.map(str::to_string).or(device.player_name.clone());
        device.last_seen_ms = seen_at_ms;

        if let Some(button_pin) = payload.button_pin {
            device.button_pin = Some(button_pin);
        }

        if let Some(button_pressed) = payload.button_pressed {
            device.button_pressed = button_pressed;
        }

        if let Some(firmware_version) = payload.firmware_version.clone() {
            device.firmware_version = Some(firmware_version);
        }

        if let Some(ip_address) = payload.ip_address.clone() {
            device.last_ip = Some(ip_address);
        }

        return;
    }

    devices.push(DevicePresence {
        player_id: player_id.to_string(),
        player_name: player_name.map(str::to_string),
        device_id: payload.device_id.clone(),
        button_pin: payload.button_pin,
        button_pressed: payload.button_pressed.unwrap_or(false),
        firmware_version: payload.firmware_version.clone(),
        last_ip: payload.ip_address.clone(),
        last_seen_ms: seen_at_ms,
    });
}

fn resolve_player_id(payload: &DeviceRequest) -> String {
    if let Some(ip_address) = payload.ip_address.as_deref() {
        if let Some(last_octet) = ip_address.rsplit('.').next() {
            if !last_octet.is_empty() && last_octet.chars().all(|c| c.is_ascii_digit()) {
                return format!("player-{}", last_octet);
            }
        }
    }

    if let Some(mac_address) = payload.mac_address.as_deref() {
        let compact: String = mac_address
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
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

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis()
}
