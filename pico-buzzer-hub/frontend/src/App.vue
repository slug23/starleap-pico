<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from "vue";

const apiBase = import.meta.env.VITE_API_BASE ?? "http://127.0.0.1:8787";
const activeMode = ref("buzzer");
const buzzes = ref([]);
const devices = ref([]);
const lightReadings = ref([]);
const loading = ref(false);
const resetting = ref(false);
const error = ref("");
const onlineWindowMs = ref(30000);
const lightHistoryLimit = ref(240);
const expectedBuzzerFirmwareVersion = ref("");
const nowMs = ref(Date.now());

let pollTimerId = null;
let gameTimerId = null;
let gameLiveTimerId = null;
let audioContext = null;
let audioMaster = null;
let raceStartBuffer = null;
let audioWarmupPlayed = false;
let hasHydratedBuzzes = false;
let knownBuzzKeys = new Set();

const gamePhase = ref("idle");
const gameCountdownStartMs = ref(null);
const gameReleaseAtMs = ref(null);
const gameLiveAtUs = ref(null);
const gameWinnerPlayerId = ref("");
const gameArmingLive = ref(false);
const disqualifiedPlayerIds = ref([]);

const displaySegments = {
  "0": ["a", "b", "c", "d", "e", "f"],
  "1": ["b", "c"],
  "2": ["a", "b", "d", "e", "g"],
  "3": ["a", "b", "c", "d", "g"],
  "4": ["b", "c", "f", "g"],
  "5": ["a", "c", "d", "f", "g"],
  "6": ["a", "c", "d", "e", "f", "g"],
  "7": ["a", "b", "c"],
  "8": ["a", "b", "c", "d", "e", "f", "g"],
  "9": ["a", "b", "c", "d", "f", "g"],
  "-": ["g"],
  " ": [],
};

const collator = new Intl.Collator([], {
  numeric: true,
  sensitivity: "base",
});

const heroCopy = computed(() => {
  if (activeMode.value === "light") {
    return {
      eyebrow: "PHOTON UPLINK",
      title: "LIGHT ARRAY",
      subtitle: "Live luminance telemetry. Multi-node sensor history.",
    };
  }

  return {
    eyebrow: "TACTICAL UPLINK",
    title: "BUZZER COMMAND",
    subtitle: "Priority lock. Node telemetry. Live trigger surveillance.",
  };
});

const buzzerDevices = computed(() => devices.value.filter((device) => isBuzzerDevice(device)));
const sensorDevices = computed(() => devices.value.filter((device) => isLightSensorDevice(device)));

const modeTabs = computed(() => [
  {
    id: "buzzer",
    label: "BUZZER OPS",
    count: buzzerDevices.value.length,
  },
  {
    id: "light",
    label: "PHOTON ARRAY",
    count: sensorDevices.value.length,
  },
]);

const eligibleBuzzes = computed(() =>
  buzzes.value.filter((entry) => !disqualifiedPlayerIds.value.includes(entry.player_id)),
);

const activeBuzzes = computed(() => {
  if (gamePhase.value === "countdown" || gamePhase.value === "hold" || gamePhase.value === "armed") {
    return [];
  }

  if (gamePhase.value === "live" || gamePhase.value === "won") {
    return sortBuzzesForRanking(eligibleBuzzes.value);
  }

  return sortBuzzesForRanking(buzzes.value);
});

const firstBuzzer = computed(() => activeBuzzes.value[0] ?? null);
const gameWinnerLabel = computed(() => {
  if (firstBuzzer.value) {
    return displayPlayerLabel(firstBuzzer.value);
  }

  return gameWinnerPlayerId.value || "";
});

const sortedBuzzerDevices = computed(() =>
  [...buzzerDevices.value].sort((left, right) => {
    const onlineDelta = Number(isOnline(right)) - Number(isOnline(left));

    if (onlineDelta !== 0) {
      return onlineDelta;
    }

    return (right.last_seen_ms ?? 0) - (left.last_seen_ms ?? 0);
  }),
);

const buzzerOnlineCount = computed(() =>
  sortedBuzzerDevices.value.filter((device) => isOnline(device)).length,
);

const outdatedBuzzerFirmwareCount = computed(
  () => sortedBuzzerDevices.value.filter((device) => firmwareNeedsUpdate(device)).length,
);

const triggerMatrixDevices = computed(() =>
  [...buzzerDevices.value].sort((left, right) => compareNodeId(left, right)),
);

const priorityCount = computed(() => activeBuzzes.value.length);

const lightReadingsByPlayer = computed(() => {
  const grouped = new Map();

  for (const entry of lightReadings.value) {
    const playerId = entry.player_id ?? "player-unknown";
    const bucket = grouped.get(playerId) ?? [];
    bucket.push(entry);
    grouped.set(playerId, bucket);
  }

  for (const bucket of grouped.values()) {
    bucket.sort((left, right) => Number(left.received_at_ms ?? 0) - Number(right.received_at_ms ?? 0));
  }

  return grouped;
});

const sensorPanels = computed(() =>
  [...sensorDevices.value]
    .sort((left, right) => compareNodeId(left, right))
    .map((device) => buildSensorPanel(device)),
);

const onlineSensorPanels = computed(() =>
  sensorPanels.value.filter((panel) => isOnline(panel)),
);

const sensorOnlineCount = computed(() => onlineSensorPanels.value.length);
const averageSensorPercent = computed(() => {
  if (!onlineSensorPanels.value.length) {
    return null;
  }

  const total = onlineSensorPanels.value.reduce((sum, panel) => sum + panel.latestPercent, 0);
  return total / onlineSensorPanels.value.length;
});

const brightestSensor = computed(() => {
  if (!onlineSensorPanels.value.length) {
    return null;
  }

  return [...onlineSensorPanels.value].sort((left, right) => right.latestPercent - left.latestPercent)[0];
});

const darkestSensor = computed(() => {
  if (!onlineSensorPanels.value.length) {
    return null;
  }

  return [...onlineSensorPanels.value].sort((left, right) => left.latestPercent - right.latestPercent)[0];
});

const gameIsRunning = computed(() =>
  gamePhase.value === "countdown" ||
  gamePhase.value === "hold" ||
  gamePhase.value === "armed" ||
  gamePhase.value === "live",
);

const gameStatus = computed(() => {
  switch (gamePhase.value) {
    case "countdown":
      return "COUNTDOWN";
    case "hold":
      return "HOLD";
    case "armed":
      return "SYNC";
    case "live":
      return "LIVE";
    case "won":
      return "WINNER";
    default:
      return "STANDBY";
  }
});

const gamePanelClass = computed(() => ({
  "game-panel-idle": gamePhase.value === "idle",
  "game-panel-countdown": gamePhase.value === "countdown",
  "game-panel-hold": gamePhase.value === "hold" || gamePhase.value === "armed",
  "game-panel-live": gamePhase.value === "live",
  "game-panel-won": gamePhase.value === "won",
}));

const gameDisplay = computed(() => {
  if (gamePhase.value === "countdown" && gameCountdownStartMs.value !== null) {
    const elapsedSeconds = Math.floor((nowMs.value - gameCountdownStartMs.value) / 1000);
    const value = Math.max(0, 5 - elapsedSeconds);
    return String(value).padStart(2, "0");
  }

  if (gamePhase.value === "hold" || gamePhase.value === "armed") {
    return "--";
  }

  if (gamePhase.value === "live") {
    return "00";
  }

  if (gamePhase.value === "won") {
    return winnerCode(gameWinnerPlayerId.value);
  }

  return "--";
});

const gameDisplayChars = computed(() => {
  const value = gameDisplay.value.slice(-3);
  return value.length >= 2 ? value.split("") : value.padStart(2, "0").split("");
});

function normalizeAppKind(value) {
  const normalized = String(value ?? "").trim().toLowerCase();

  if (normalized === "light" || normalized === "sensor" || normalized === "photoresistor") {
    return "light-sensor";
  }

  if (!normalized) {
    return "generic";
  }

  return normalized;
}

function isBuzzerDevice(device) {
  const kind = normalizeAppKind(device.app_kind);
  return kind === "buzzer" || (kind === "generic" && device.button_pin !== null);
}

function isLightSensorDevice(device) {
  const kind = normalizeAppKind(device.app_kind);
  return (
    kind === "light-sensor" ||
    device.light_pin !== null ||
    device.last_light_raw !== null ||
    device.last_light_percent !== null
  );
}

function getBuzzKey(entry) {
  return `${entry.order ?? "x"}:${entry.player_id ?? "unknown"}:${entry.received_at_ms ?? "0"}`;
}

async function ensureAudioReady() {
  if (typeof window === "undefined") {
    return false;
  }

  const AudioContextClass = window.AudioContext ?? window.webkitAudioContext;

  if (!AudioContextClass) {
    return false;
  }

  if (!audioContext) {
    audioContext = new AudioContextClass();
    audioMaster = audioContext.createGain();
    audioMaster.gain.value = 1;
    audioMaster.connect(audioContext.destination);
  }

  if (audioContext.state === "suspended") {
    await audioContext.resume();
  }

  if (!raceStartBuffer || raceStartBuffer.sampleRate !== audioContext.sampleRate) {
    raceStartBuffer = buildRaceStartBuffer(audioContext);
  }

  warmAudioOutput();

  return audioContext.state === "running";
}

function warmAudioOutput() {
  if (!audioContext || audioContext.state !== "running" || audioWarmupPlayed) {
    return;
  }

  const source = audioContext.createBufferSource();
  source.buffer = audioContext.createBuffer(1, 1, audioContext.sampleRate);
  source.connect(audioMaster ?? audioContext.destination);
  source.start(audioContext.currentTime);
  source.addEventListener("ended", () => {
    source.disconnect();
  });
  audioWarmupPlayed = true;
}

function createNoiseGenerator() {
  let seed = 0x51f15e;

  return () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0x80000000 - 1;
  };
}

function buildRaceStartBuffer(context) {
  const sampleRate = context.sampleRate;
  const length = Math.ceil(sampleRate * 0.72);
  const buffer = context.createBuffer(1, length, sampleRate);
  const samples = buffer.getChannelData(0);
  const noise = createNoiseGenerator();

  for (let index = 0; index < length; index += 1) {
    const time = index / sampleRate;
    const crackEnvelope = Math.exp(-time * 76);
    const bodyEnvelope = Math.exp(-time * 16);
    const tailEnvelope = Math.exp(-time * 7);
    const snap = Math.sin(2 * Math.PI * (2100 * time + 4600 * time * time)) * crackEnvelope;
    const thumpFrequency = 118 - 72 * Math.min(time / 0.16, 1);
    const thump = Math.sin(2 * Math.PI * thumpFrequency * time) * Math.exp(-time * 23);
    const firstReflection =
      time > 0.045 ? noise() * Math.exp(-(time - 0.045) * 38) * 0.34 : 0;
    const secondReflection =
      time > 0.11 ? noise() * Math.exp(-(time - 0.11) * 24) * 0.18 : 0;
    const pressureWave =
      noise() * crackEnvelope * 1.05 +
      noise() * bodyEnvelope * 0.24 +
      noise() * tailEnvelope * 0.05 +
      snap * 0.42 +
      thump * 0.5 +
      firstReflection +
      secondReflection;

    samples[index] = Math.tanh(pressureWave * 1.55) * 0.72;
  }

  return buffer;
}

function scheduleVoice({
  startTime,
  frequency,
  endFrequency = frequency,
  type = "sine",
  gain = 0.05,
  attack = 0.01,
  duration = 0.2,
  detune = 0,
  q = null,
  filterType = null,
  filterFrequency = null,
}) {
  if (!audioContext || audioContext.state !== "running") {
    return;
  }

  const oscillator = audioContext.createOscillator();
  const envelope = audioContext.createGain();
  let filterNode = null;

  oscillator.type = type;
  oscillator.frequency.setValueAtTime(Math.max(1, frequency), startTime);
  oscillator.frequency.exponentialRampToValueAtTime(
    Math.max(1, endFrequency),
    startTime + duration,
  );
  oscillator.detune.setValueAtTime(detune, startTime);

  envelope.gain.setValueAtTime(0.0001, startTime);
  envelope.gain.linearRampToValueAtTime(gain, startTime + attack);
  envelope.gain.exponentialRampToValueAtTime(0.0001, startTime + duration);

  if (filterType && filterFrequency) {
    filterNode = audioContext.createBiquadFilter();
    filterNode.type = filterType;
    filterNode.frequency.setValueAtTime(filterFrequency, startTime);

    if (typeof q === "number") {
      filterNode.Q.setValueAtTime(q, startTime);
    }

    oscillator.connect(filterNode);
    filterNode.connect(envelope);
  } else {
    oscillator.connect(envelope);
  }

  envelope.connect(audioMaster ?? audioContext.destination);

  oscillator.start(startTime);
  oscillator.stop(startTime + duration + 0.05);

  oscillator.addEventListener("ended", () => {
    oscillator.disconnect();
    envelope.disconnect();

    if (filterNode) {
      filterNode.disconnect();
    }
  });
}

function playRaceStartSound(delaySeconds = 0) {
  if (!audioContext || audioContext.state !== "running") {
    return;
  }

  const startTime = audioContext.currentTime + delaySeconds;
  const source = audioContext.createBufferSource();
  const gain = audioContext.createGain();

  if (!raceStartBuffer || raceStartBuffer.sampleRate !== audioContext.sampleRate) {
    raceStartBuffer = buildRaceStartBuffer(audioContext);
  }

  source.buffer = raceStartBuffer;
  gain.gain.setValueAtTime(1.05, startTime);
  source.connect(gain);
  gain.connect(audioMaster ?? audioContext.destination);
  source.start(startTime);
  source.addEventListener("ended", () => {
    source.disconnect();
    gain.disconnect();
  });
}

function playBuzzSound(delaySeconds = 0) {
  if (!audioContext || audioContext.state !== "running") {
    return;
  }

  const startTime = audioContext.currentTime + delaySeconds;

  scheduleVoice({
    startTime,
    frequency: 190,
    endFrequency: 132,
    type: "square",
    gain: 0.028,
    attack: 0.004,
    duration: 0.16,
    filterType: "bandpass",
    filterFrequency: 900,
    q: 2.5,
  });
  scheduleVoice({
    startTime: startTime + 0.01,
    frequency: 380,
    endFrequency: 264,
    type: "sawtooth",
    gain: 0.018,
    attack: 0.003,
    duration: 0.12,
    filterType: "lowpass",
    filterFrequency: 1200,
    q: 0.7,
  });
}

function syncBuzzAudio(entries) {
  const nextKeys = new Set(entries.map((entry) => getBuzzKey(entry)));

  if (!hasHydratedBuzzes) {
    knownBuzzKeys = nextKeys;
    hasHydratedBuzzes = true;
    return;
  }

  const newEntries = entries
    .filter((entry) => !knownBuzzKeys.has(getBuzzKey(entry)))
    .sort((left, right) => Number(left.order ?? 0) - Number(right.order ?? 0));

  newEntries.forEach((_entry, index) => {
    playBuzzSound(index * 0.12);
  });

  knownBuzzKeys = nextKeys;
}

function formatTime(timestamp) {
  return new Intl.DateTimeFormat([], {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
  }).format(new Date(timestamp));
}

function formatDeltaUs(deltaUs) {
  return (Math.max(0, Number(deltaUs) || 0) / 1000).toFixed(3);
}

function formatSignedDeltaUs(deltaUs) {
  const value = (Number(deltaUs) || 0) / 1000;
  const sign = value > 0 ? "+" : "";
  return `${sign}${value.toFixed(3)}`;
}

function formatReactionValueUs(deltaUs) {
  const value = Number(deltaUs) || 0;
  return value < 0 ? formatSignedDeltaUs(value) : formatDeltaUs(value);
}

function formatLatencyUs(value) {
  if (value === null || value === undefined) {
    return "---.--- ms";
  }

  return `${formatDeltaUs(value)} ms`;
}

function firmwareLabel(device) {
  return String(device?.firmware_version ?? "").trim() || "UNKNOWN";
}

function firmwareNeedsUpdate(device) {
  const expected = expectedBuzzerFirmwareVersion.value;

  if (!expected || !isBuzzerDevice(device)) {
    return false;
  }

  return firmwareLabel(device) !== expected;
}

function entryReceivedUs(entry) {
  if (entry?.received_at_us !== undefined && entry?.received_at_us !== null) {
    return Number(entry.received_at_us);
  }

  return Number(entry?.received_at_ms ?? 0) * 1000;
}

function serverReactionUs(entry) {
  if (entry?.reaction_us !== undefined && entry?.reaction_us !== null) {
    return Number(entry.reaction_us);
  }

  if (gameLiveAtUs.value === null) {
    return null;
  }

  return entryReceivedUs(entry) - Number(gameLiveAtUs.value);
}

function clientReactionUs(entry) {
  if (entry?.client_reaction_us === undefined || entry?.client_reaction_us === null) {
    return null;
  }

  return Number(entry.client_reaction_us);
}

function rankingReactionUs(entry) {
  const clientValue = clientReactionUs(entry);

  if (clientValue !== null && clientValue >= 0) {
    return clientValue;
  }

  return serverReactionUs(entry) ?? entryReceivedUs(entry);
}

function sortBuzzesForRanking(entries) {
  return [...entries].sort((left, right) => {
    const reactionDelta = rankingReactionUs(left) - rankingReactionUs(right);

    if (reactionDelta !== 0) {
      return reactionDelta;
    }

    return Number(left.order ?? 0) - Number(right.order ?? 0);
  });
}

function formatDeltaFromFirst(entry) {
  if (!firstBuzzer.value) {
    return "";
  }

  return formatDeltaUs(rankingReactionUs(entry) - rankingReactionUs(firstBuzzer.value));
}

function formatClientReactionTime(entry) {
  const value = clientReactionUs(entry);
  return value === null ? "---.---" : formatReactionValueUs(value);
}

function formatServerReactionTime(entry) {
  const value = serverReactionUs(entry);
  return value === null ? "---.---" : formatDeltaUs(value);
}

function formatMethodDifference(entry) {
  const clientValue = clientReactionUs(entry);
  const serverValue = serverReactionUs(entry);

  if (clientValue === null || serverValue === null) {
    return "---.---";
  }

  return formatSignedDeltaUs(clientValue - serverValue);
}

function formatClientTimingStatus(entry) {
  const status = String(entry?.client_timing_status ?? "").trim();

  if (!status || status === "ok") {
    return "";
  }

  return status.replaceAll("_", " ").toUpperCase();
}

function goDeliveryLabel(device) {
  const attemptRound = device?.last_go_attempt_round_id;

  if (attemptRound === null || attemptRound === undefined) {
    return "GO NONE";
  }

  return device?.last_go_success_round_id === attemptRound ? `GO OK R${attemptRound}` : `GO MISS R${attemptRound}`;
}

function goDeliveryNeedsAttention(device) {
  const attemptRound = device?.last_go_attempt_round_id;

  return (
    attemptRound !== null &&
    attemptRound !== undefined &&
    device?.last_go_success_round_id !== attemptRound
  );
}

function picoGoLabel(device) {
  const round = device?.last_client_go_round_id;
  const ticksSet = device?.last_client_go_ticks_set;

  if (ticksSet === true && round !== null && round !== undefined) {
    return `PICO GO R${round}`;
  }

  if (ticksSet === false) {
    return "PICO NO GO";
  }

  return "PICO GO ?";
}

function showPicoGoLabel(device) {
  return (
    device?.last_go_attempt_round_id !== null &&
    device?.last_go_attempt_round_id !== undefined
  ) || (
    device?.last_client_go_round_id !== null &&
    device?.last_client_go_round_id !== undefined
  ) || (
    device?.last_client_go_ticks_set !== null &&
    device?.last_client_go_ticks_set !== undefined
  );
}

function picoGoNeedsAttention(device) {
  const attemptRound = device?.last_go_attempt_round_id;

  if (attemptRound === null || attemptRound === undefined) {
    return false;
  }

  return device?.last_client_go_ticks_set !== true || device?.last_client_go_round_id !== attemptRound;
}

function compareNodeId(left, right) {
  return collator.compare(left.player_id ?? "", right.player_id ?? "");
}

function displayPlayerLabel(entity) {
  const playerName = String(entity?.player_name ?? "").trim();

  if (playerName) {
    return playerName;
  }

  return entity?.player_id ?? "player-unknown";
}

function isDisqualified(device) {
  return disqualifiedPlayerIds.value.includes(device.player_id);
}

function isEarlyPressed(device) {
  return (
    !isDisqualified(device) &&
    (gamePhase.value === "countdown" || gamePhase.value === "hold" || gamePhase.value === "armed") &&
    isOnline(device) &&
    device.button_pressed
  );
}

function winnerCode(playerId) {
  const match = String(playerId ?? "").match(/(\d+)(?!.*\d)/);

  if (!match) {
    return "--";
  }

  return match[1].slice(-3).padStart(2, "0");
}

function isSegmentOn(char, segment) {
  return displaySegments[char]?.includes(segment) ?? false;
}

function isOnline(device) {
  return nowMs.value - Number(device.last_seen_ms ?? 0) <= onlineWindowMs.value;
}

function triggerStateLabel(device) {
  if (isDisqualified(device)) {
    return "DISQUALIFIED";
  }

  if (isEarlyPressed(device)) {
    return "EARLY";
  }

  if (!isOnline(device)) {
    return "DORMANT";
  }

  return device.button_pressed ? "ENGAGED" : "ARMED";
}

function triggerStateClass(device) {
  if (isDisqualified(device)) {
    return "button-led-early";
  }

  if (isEarlyPressed(device)) {
    return "button-led-early";
  }

  if (!isOnline(device)) {
    return "button-led-inactive";
  }

  return device.button_pressed ? "button-led-active" : "button-led-armed";
}

function formatLastSeen(timestamp) {
  const elapsedSeconds = Math.max(0, Math.round((nowMs.value - Number(timestamp ?? 0)) / 1000));

  if (elapsedSeconds < 2) {
    return "NOW";
  }

  if (elapsedSeconds < 60) {
    return `-${elapsedSeconds}s`;
  }

  const elapsedMinutes = Math.round(elapsedSeconds / 60);
  return `-${elapsedMinutes}m`;
}

function formatPercent(value) {
  return `${Number(value ?? 0).toFixed(1)}%`;
}

function clampPercent(value) {
  return Math.max(0, Math.min(100, Number(value ?? 0)));
}

function formatSpan(ms) {
  const seconds = Math.max(0, Math.round(ms / 1000));

  if (seconds < 60) {
    return `${seconds}s`;
  }

  const minutes = Math.floor(seconds / 60);
  const remainderSeconds = seconds % 60;
  return remainderSeconds ? `${minutes}m ${remainderSeconds}s` : `${minutes}m`;
}

function sensorSeriesFor(device) {
  const series = lightReadingsByPlayer.value.get(device.player_id) ?? [];

  if (series.length) {
    return series;
  }

  if (device.last_light_percent === null && device.last_light_raw === null) {
    return [];
  }

  return [
    {
      player_id: device.player_id,
      light_pin: device.light_pin,
      raw: device.last_light_raw ?? 0,
      percent: device.last_light_percent ?? 0,
      received_at_ms: device.last_seen_ms,
    },
  ];
}

function buildChartPoints(series, width = 360, height = 132) {
  if (!series.length) {
    return "";
  }

  if (series.length === 1) {
    const y = height - (clampPercent(series[0].percent) / 100) * height;
    return `0,${y} ${width},${y}`;
  }

  const lastIndex = series.length - 1;

  return series
    .map((entry, index) => {
      const x = (index / lastIndex) * width;
      const y = height - (clampPercent(entry.percent) / 100) * height;
      return `${x},${y}`;
    })
    .join(" ");
}

function sensorSpanLabel(series) {
  if (series.length < 2) {
    return "LIVE";
  }

  const spanMs = Number(series[series.length - 1].received_at_ms) - Number(series[0].received_at_ms);
  return `SPAN ${formatSpan(spanMs)}`;
}

function buildSensorPanel(device) {
  const series = sensorSeriesFor(device);
  const latest = series[series.length - 1] ?? null;
  const percents = series.map((entry) => clampPercent(entry.percent));

  return {
    ...device,
    series,
    latestRaw: latest?.raw ?? device.last_light_raw ?? 0,
    latestPercent: latest?.percent ?? device.last_light_percent ?? 0,
    latestAtMs: latest?.received_at_ms ?? device.last_seen_ms,
    chartPoints: buildChartPoints(series),
    minPercent: percents.length ? Math.min(...percents) : 0,
    maxPercent: percents.length ? Math.max(...percents) : 0,
    spanLabel: sensorSpanLabel(series),
  };
}

function clearGameLiveTimer() {
  if (gameLiveTimerId !== null) {
    window.clearTimeout(gameLiveTimerId);
    gameLiveTimerId = null;
  }
}

function scheduleLiveCue(delayMs) {
  const delay = Math.max(0, Number(delayMs) || 0);
  const audioWasScheduled = Boolean(audioContext && audioContext.state === "running");

  clearGameLiveTimer();
  gamePhase.value = delay > 0 ? "armed" : "live";

  if (audioWasScheduled) {
    playRaceStartSound(delay / 1000);
  }

  gameLiveTimerId = window.setTimeout(() => {
    gameLiveTimerId = null;
    nowMs.value = Date.now();
    gamePhase.value = "live";

    if (!audioWasScheduled) {
      playRaceStartSound();
    }
  }, delay);
}

function clearLocalGameState() {
  clearGameLiveTimer();
  gamePhase.value = "idle";
  gameCountdownStartMs.value = null;
  gameReleaseAtMs.value = null;
  gameLiveAtUs.value = null;
  gameWinnerPlayerId.value = "";
  gameArmingLive.value = false;
  disqualifiedPlayerIds.value = [];
}

function markDisqualified(playerId) {
  if (!playerId || disqualifiedPlayerIds.value.includes(playerId)) {
    return;
  }

  disqualifiedPlayerIds.value = [...disqualifiedPlayerIds.value, playerId].sort((left, right) =>
    collator.compare(left, right),
  );
}

function trackEarlyPresses() {
  if (gamePhase.value !== "countdown" && gamePhase.value !== "hold" && gamePhase.value !== "armed") {
    return;
  }

  for (const entry of buzzes.value) {
    markDisqualified(entry.player_id);
  }

  for (const device of buzzerDevices.value) {
    if (isOnline(device) && device.button_pressed) {
      markDisqualified(device.player_id);
    }
  }
}

function applyBuzzerState(data) {
  buzzes.value = data.buzzes ?? [];
  devices.value = data.devices ?? [];
  onlineWindowMs.value = data.online_window_ms ?? 30000;
  gameLiveAtUs.value = data.live_at_us ?? null;
  expectedBuzzerFirmwareVersion.value = data.expected_buzzer_firmware_version ?? "";
}

async function loadBuzzerState() {
  const response = await fetch(`${apiBase}/api/state`);

  if (!response.ok) {
    throw new Error(`Buzzer state request failed with ${response.status}`);
  }

  const data = await response.json();
  syncBuzzAudio(data.buzzes ?? []);
  applyBuzzerState(data);
  trackEarlyPresses();
}

async function loadLightState() {
  const response = await fetch(`${apiBase}/api/light-state`);

  if (!response.ok) {
    throw new Error(`Light state request failed with ${response.status}`);
  }

  const data = await response.json();
  devices.value = data.devices ?? [];
  lightReadings.value = data.light_readings ?? [];
  onlineWindowMs.value = data.online_window_ms ?? 30000;
  lightHistoryLimit.value = data.light_history_limit ?? 240;
}

async function loadCurrentState() {
  loading.value = true;
  error.value = "";
  nowMs.value = Date.now();

  try {
    if (activeMode.value === "light") {
      await loadLightState();
    } else {
      await loadBuzzerState();
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to load hub state.";
  } finally {
    loading.value = false;
  }
}

async function resetRound() {
  resetting.value = true;
  error.value = "";

  try {
    const response = await fetch(`${apiBase}/api/reset`, {
      method: "POST",
    });

    if (!response.ok) {
      throw new Error(`Reset failed with ${response.status}`);
    }

    const data = await response.json();
    applyBuzzerState(data);
    return data;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to reset the round.";
    return null;
  } finally {
    resetting.value = false;
  }
}

async function resetGame() {
  void ensureAudioReady();
  await resetRound();
  clearLocalGameState();
}

async function armLiveWindow() {
  if (gameArmingLive.value || gamePhase.value === "armed") {
    return;
  }

  gameArmingLive.value = true;

  try {
    const response = await fetch(`${apiBase}/api/arm-live`, {
      method: "POST",
    });

    if (!response.ok) {
      throw new Error(`Arm live failed with ${response.status}`);
    }

    const data = await response.json();
    applyBuzzerState(data);
    scheduleLiveCue(data.live_in_ms ?? 0);
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to arm the live round.";
  }

  gameArmingLive.value = false;
}

async function startGame() {
  if (gameIsRunning.value || gameArmingLive.value) {
    return;
  }

  clearGameLiveTimer();
  void ensureAudioReady();

  const data = await resetRound();

  if (!data) {
    return;
  }

  disqualifiedPlayerIds.value = [];
  const startedAt = Date.now();
  gameCountdownStartMs.value = startedAt;
  gameReleaseAtMs.value = startedAt + 6000 + 5000 + Math.floor(Math.random() * 5001);
  gameLiveAtUs.value = null;
  gameWinnerPlayerId.value = "";
  gamePhase.value = "countdown";
}

function tickGame() {
  nowMs.value = Date.now();

  if (gamePhase.value === "countdown" && gameCountdownStartMs.value !== null) {
    if (nowMs.value >= gameCountdownStartMs.value + 6000) {
      gamePhase.value = "hold";
    }

    return;
  }

  if (gamePhase.value === "hold" && gameReleaseAtMs.value !== null) {
    if (nowMs.value >= gameReleaseAtMs.value) {
      void armLiveWindow();
    }

    return;
  }

  if (gamePhase.value === "live" && firstBuzzer.value) {
    gameWinnerPlayerId.value = firstBuzzer.value.player_id;
    gamePhase.value = "won";
  }
}

function clearPollTimer() {
  if (pollTimerId !== null) {
    window.clearInterval(pollTimerId);
    pollTimerId = null;
  }
}

function pollIntervalMs() {
  return activeMode.value === "light" ? 1000 : 250;
}

function startPolling() {
  clearPollTimer();
  void loadCurrentState();
  pollTimerId = window.setInterval(() => {
    void loadCurrentState();
  }, pollIntervalMs());
}

watch(activeMode, () => {
  error.value = "";
  startPolling();
});

onMounted(() => {
  startPolling();
  gameTimerId = window.setInterval(tickGame, 100);
});

onUnmounted(() => {
  clearPollTimer();
  clearGameLiveTimer();

  if (gameTimerId !== null) {
    window.clearInterval(gameTimerId);
  }

  if (audioContext) {
    void audioContext.close();
    audioContext = null;
    audioMaster = null;
    raceStartBuffer = null;
    audioWarmupPlayed = false;
  }
});
</script>

<template>
  <main class="app-shell">
    <template v-if="activeMode === 'buzzer'">
      <section class="dashboard-grid dashboard-grid-buzzer">
        <div class="dashboard-stack buzzer-left-stack">
          <section class="hero">
            <p class="eyebrow">{{ heroCopy.eyebrow }}</p>
            <h1>{{ heroCopy.title }}</h1>

            <div class="mode-tabs" role="tablist" aria-label="Hub modes">
              <button
                v-for="tab in modeTabs"
                :key="tab.id"
                :class="['mode-tab', activeMode === tab.id ? 'mode-tab-active' : '']"
                :aria-selected="activeMode === tab.id"
                @click="activeMode = tab.id"
              >
                <span>{{ tab.label }}</span>
                <span class="mode-tab-count">{{ tab.count }}</span>
              </button>
            </div>
          </section>

          <section v-if="error" class="panel error-panel error-panel-inline">
            <strong>ALERT</strong> {{ error }}
          </section>

          <article class="panel game-panel buzzer-game-panel" :class="gamePanelClass">
            <div class="panel-header">
              <p class="panel-label">GAME CORE</p>
              <span class="queue-count">{{ gameStatus }}</span>
            </div>

            <div class="display-core">
              <div class="segment-display">
                <div v-for="(char, index) in gameDisplayChars" :key="`${char}-${index}`" class="segment-digit">
                  <span
                    v-for="segment in ['a', 'b', 'c', 'd', 'e', 'f', 'g']"
                    :key="segment"
                    :class="['segment', `segment-${segment}`, isSegmentOn(char, segment) ? 'segment-on' : '']"
                  ></span>
                </div>
              </div>

              <div class="game-status-row">
                <span class="game-status-chip">{{ gameStatus }}</span>
                <span v-if="gamePhase === 'won' && gameWinnerLabel" class="game-status-chip">
                  PLAYER {{ gameWinnerLabel }}
                </span>
              </div>
            </div>
            <div class="game-controls">
              <button class="primary-button" :disabled="resetting" @click="resetGame">
                {{ resetting ? "RESETTING..." : "RESET" }}
              </button>
              <button
                class="primary-button primary-button-go"
                :disabled="resetting || gameIsRunning || gameArmingLive"
                @click="startGame"
              >
                {{ gameIsRunning || gameArmingLive ? "RUNNING" : "GO" }}
              </button>
            </div>
          </article>

          <article class="panel buttons-panel">
            <div class="panel-header">
              <p class="panel-label">TRIGGER MATRIX</p>
              <span class="queue-count">{{ triggerMatrixDevices.length }} TRACKED</span>
            </div>

            <ul v-if="triggerMatrixDevices.length" class="button-grid">
              <li
                v-for="device in triggerMatrixDevices"
                :key="`button-${device.device_id ?? device.player_id}`"
                class="button-card"
              >
                <div :class="['button-led', triggerStateClass(device)]"></div>
                <div class="device-copy">
                  <p class="queue-name">{{ displayPlayerLabel(device) }}</p>
                  <p class="queue-meta">
                    {{ triggerStateLabel(device) }}
                    <span v-if="device.button_pin !== null"> · GP{{ device.button_pin }}</span>
                  </p>
                </div>
              </li>
            </ul>

            <div v-else class="empty-state compact-empty">
              <p>NO LIVE TRIGGER CHANNELS.</p>
            </div>
          </article>

          <article class="panel spotlight">
            <p class="panel-label">PRIMARY SIGNAL</p>
            <div v-if="firstBuzzer" class="winner-card">
              <div class="winner-order">#{{ firstBuzzer.order }}</div>
              <div>
                <h2>{{ displayPlayerLabel(firstBuzzer) }}</h2>
                <p>{{ formatTime(firstBuzzer.received_at_ms) }}</p>
              </div>
            </div>
            <div v-else class="empty-state">
              <p>{{ loading ? "AWAITING UPLINK..." : "AWAITING FIRST SIGNAL." }}</p>
            </div>
          </article>

          <article class="panel devices-panel">
            <div class="panel-header">
              <p class="panel-label">ACTIVE NODES</p>
              <span class="queue-count">
                {{ buzzerOnlineCount }} LINKED / {{ buzzerDevices.length }} TRACKED
                <span v-if="outdatedBuzzerFirmwareCount"> · {{ outdatedBuzzerFirmwareCount }} UPDATE</span>
              </span>
            </div>

            <ul v-if="sortedBuzzerDevices.length" class="device-list">
              <li
                v-for="device in sortedBuzzerDevices"
                :key="device.device_id ?? device.player_id"
                class="device-item"
              >
                <div class="device-copy">
                  <div class="device-title-row">
                    <p class="queue-name">{{ displayPlayerLabel(device) }}</p>
                    <span :class="['status-pill', isOnline(device) ? 'status-online' : 'status-quiet']">
                      {{ isOnline(device) ? "Online" : "Quiet" }}
                    </span>
                    <span
                      :class="[
                        'status-pill',
                        firmwareNeedsUpdate(device) ? 'status-warning' : 'status-firmware-ok',
                      ]"
                    >
                      {{ firmwareNeedsUpdate(device) ? "UPDATE FW" : "FW OK" }}
                    </span>
                    <span
                      :class="[
                        'status-pill',
                        goDeliveryNeedsAttention(device) ? 'status-warning' : 'status-firmware-ok',
                      ]"
                    >
                      {{ goDeliveryLabel(device) }}
                    </span>
                    <span
                      v-if="showPicoGoLabel(device)"
                      :class="[
                        'status-pill',
                        picoGoNeedsAttention(device) ? 'status-warning' : 'status-firmware-ok',
                      ]"
                    >
                      {{ picoGoLabel(device) }}
                    </span>
                  </div>
                  <p class="queue-meta">
                    SEEN {{ formatLastSeen(device.last_seen_ms) }}
                    <span> · {{ device.button_pressed ? "HOT" : "IDLE" }}</span>
                    <span> · FW {{ firmwareLabel(device) }}</span>
                    <span v-if="expectedBuzzerFirmwareVersion">
                      / EXPECT {{ expectedBuzzerFirmwareVersion }}
                    </span>
                    <span
                      v-if="
                        device.last_client_checkin_rtt_us !== null &&
                        device.last_client_checkin_rtt_us !== undefined
                      "
                    >
                      · PING {{ formatLatencyUs(device.last_client_checkin_rtt_us) }} · PJIT
                      {{ formatLatencyUs(device.last_client_checkin_jitter_us) }}
                    </span>
                    <span
                      v-if="
                        device.last_calibration_round_id !== null &&
                        device.last_calibration_round_id !== undefined
                      "
                    >
                      · RTT {{ formatLatencyUs(device.last_calibration_rtt_us) }} · JIT
                      {{ formatLatencyUs(device.last_calibration_jitter_us) }}
                    </span>
                    <span v-if="showPicoGoLabel(device)"> · {{ picoGoLabel(device) }}</span>
                    <span v-if="device.last_calibration_error"> · CAL {{ device.last_calibration_error }}</span>
                    <span v-if="device.last_ip"> · {{ device.last_ip }}</span>
                    <span v-if="device.mac_address"> · MAC {{ device.mac_address }}</span>
                    <span v-if="device.device_id"> · UID {{ device.device_id }}</span>
                  </p>
                </div>
                <div v-if="device.button_pin !== null" class="device-pin">GP{{ device.button_pin }}</div>
              </li>
            </ul>

            <div v-else class="empty-state compact-empty">
              <p>NODE GRID EMPTY.</p>
            </div>
          </article>
        </div>

        <article class="panel queue-panel buzzer-priority-panel">
          <div class="panel-header">
            <p class="panel-label">PRIORITY STACK</p>
            <span class="queue-count">{{ priorityCount }} RANKED</span>
          </div>

          <ol v-if="activeBuzzes.length" class="queue-list">
            <li
              v-for="(entry, index) in activeBuzzes"
              :key="`${entry.player_id}-${entry.order}`"
              class="queue-item"
            >
              <div class="queue-item-body">
                <div class="queue-order">#{{ index + 1 }}</div>
                <div class="queue-player-row">
                  <p class="queue-delta">
                    <span class="queue-delta-value">{{ formatDeltaFromFirst(entry) }}</span>
                    <span class="queue-delta-unit">ms</span>
                  </p>
                  <div class="queue-method-stack">
                    <p class="queue-live-delta queue-client-delta">
                      <span class="queue-live-label">PICO</span>
                      <span class="queue-live-value">{{ formatClientReactionTime(entry) }}</span>
                      <span class="queue-live-unit">ms</span>
                    </p>
                    <p class="queue-live-delta">
                      <span class="queue-live-label">SRV</span>
                      <span class="queue-live-value">{{ formatServerReactionTime(entry) }}</span>
                      <span class="queue-live-unit">ms</span>
                    </p>
                    <p class="queue-method-diff">DIFF {{ formatMethodDifference(entry) }} ms</p>
                    <p v-if="formatClientTimingStatus(entry)" class="queue-method-diff queue-method-warning">
                      {{ formatClientTimingStatus(entry) }}
                    </p>
                  </div>
                  <p class="queue-name">{{ displayPlayerLabel(entry) }}</p>
                </div>
              </div>
            </li>
          </ol>

          <div v-else class="empty-state">
            <p>SIGNAL STACK EMPTY.</p>
          </div>
        </article>
      </section>
    </template>

    <template v-else>
      <section class="hero">
        <p class="eyebrow">{{ heroCopy.eyebrow }}</p>
        <h1>{{ heroCopy.title }}</h1>
        <p class="subtitle">
          {{ heroCopy.subtitle }}
        </p>

        <div class="hero-actions">
          <span class="backend-pill">{{ sensorOnlineCount }} STREAMING</span>
          <span class="backend-pill">WINDOW {{ lightHistoryLimit }} SAMPLES</span>
          <span class="backend-pill">UPLINK {{ apiBase }}</span>
        </div>

        <div class="mode-tabs" role="tablist" aria-label="Hub modes">
          <button
            v-for="tab in modeTabs"
            :key="tab.id"
            :class="['mode-tab', activeMode === tab.id ? 'mode-tab-active' : '']"
            :aria-selected="activeMode === tab.id"
            @click="activeMode = tab.id"
          >
            <span>{{ tab.label }}</span>
            <span class="mode-tab-count">{{ tab.count }}</span>
          </button>
        </div>
      </section>

      <section v-if="error" class="panel error-panel">
        <strong>ALERT</strong> {{ error }}
      </section>

      <section class="dashboard-grid dashboard-grid-light">
        <div class="dashboard-stack">
          <article class="panel spotlight sensor-spotlight">
            <div class="panel-header">
              <p class="panel-label">LIVE ARRAY</p>
              <span class="queue-count">{{ sensorOnlineCount }} STREAMING</span>
            </div>

            <div v-if="sensorPanels.length" class="sensor-summary-grid">
              <div class="summary-stat">
                <p class="summary-label">ARRAY AVG</p>
                <p class="summary-value">{{ averageSensorPercent === null ? "--" : formatPercent(averageSensorPercent) }}</p>
              </div>
              <div class="summary-stat">
                <p class="summary-label">BRIGHTEST</p>
                <p class="summary-value">{{ brightestSensor ? displayPlayerLabel(brightestSensor) : "--" }}</p>
                <p class="summary-meta">{{ brightestSensor ? formatPercent(brightestSensor.latestPercent) : "NO DATA" }}</p>
              </div>
              <div class="summary-stat">
                <p class="summary-label">DARKEST</p>
                <p class="summary-value">{{ darkestSensor ? displayPlayerLabel(darkestSensor) : "--" }}</p>
                <p class="summary-meta">{{ darkestSensor ? formatPercent(darkestSensor.latestPercent) : "NO DATA" }}</p>
              </div>
              <div class="summary-stat">
                <p class="summary-label">HISTORY</p>
                <p class="summary-value">{{ lightHistoryLimit }}</p>
                <p class="summary-meta">SAMPLES / DEVICE</p>
              </div>
            </div>

            <div v-else class="empty-state">
              <p>{{ loading ? "AWAITING SENSOR FEED..." : "NO PHOTON CHANNELS ONLINE." }}</p>
            </div>
          </article>

          <article class="panel devices-panel">
            <div class="panel-header">
              <p class="panel-label">ACTIVE NODES</p>
              <span class="queue-count">{{ sensorOnlineCount }} LIVE / {{ sensorDevices.length }} TRACKED</span>
            </div>

            <ul v-if="sensorPanels.length" class="device-list">
              <li
                v-for="panel in sensorPanels"
                :key="panel.device_id ?? panel.player_id"
                class="device-item"
              >
                <div class="device-copy">
                  <div class="device-title-row">
                    <p class="queue-name">{{ displayPlayerLabel(panel) }}</p>
                    <span :class="['status-pill', isOnline(panel) ? 'status-online' : 'status-quiet']">
                      {{ isOnline(panel) ? "Online" : "Quiet" }}
                    </span>
                  </div>
                  <p class="queue-meta">
                    SEEN {{ formatLastSeen(panel.last_seen_ms) }}
                    <span> · LEVEL {{ formatPercent(panel.latestPercent) }}</span>
                    <span> · RAW {{ panel.latestRaw }}</span>
                    <span v-if="panel.last_ip"> · {{ panel.last_ip }}</span>
                    <span v-if="panel.mac_address"> · MAC {{ panel.mac_address }}</span>
                  </p>
                </div>
                <div v-if="panel.light_pin !== null" class="device-pin">ADC{{ panel.light_pin - 26 }}</div>
              </li>
            </ul>

            <div v-else class="empty-state compact-empty">
              <p>NODE GRID EMPTY.</p>
            </div>
          </article>
        </div>

        <div class="dashboard-stack dashboard-stack-light-right">
          <article class="panel sensor-history-panel">
            <div class="panel-header">
              <p class="panel-label">HISTORY TRACES</p>
              <span class="queue-count">{{ sensorPanels.length }} CHANNELS</span>
            </div>

            <div v-if="sensorPanels.length" class="sensor-card-grid">
              <article
                v-for="panel in sensorPanels"
                :key="`sensor-${panel.device_id ?? panel.player_id}`"
                class="sensor-card"
              >
                <div class="sensor-card-top">
                  <div>
                    <p class="queue-name">{{ displayPlayerLabel(panel) }}</p>
                    <p class="queue-meta">
                      {{ panel.spanLabel }}
                      <span v-if="panel.light_pin !== null"> · GP{{ panel.light_pin }}</span>
                    </p>
                  </div>
                  <span :class="['status-pill', isOnline(panel) ? 'status-online' : 'status-quiet']">
                    {{ isOnline(panel) ? "LIVE" : "QUIET" }}
                  </span>
                </div>

                <div class="sensor-metrics">
                  <div class="sensor-metric">
                    <span class="sensor-metric-label">LEVEL</span>
                    <strong class="sensor-metric-value">{{ formatPercent(panel.latestPercent) }}</strong>
                  </div>
                  <div class="sensor-metric">
                    <span class="sensor-metric-label">RAW</span>
                    <strong class="sensor-metric-value">{{ panel.latestRaw }}</strong>
                  </div>
                  <div class="sensor-metric">
                    <span class="sensor-metric-label">RANGE</span>
                    <strong class="sensor-metric-value">
                      {{ formatPercent(panel.minPercent) }} - {{ formatPercent(panel.maxPercent) }}
                    </strong>
                  </div>
                </div>

                <div class="sensor-meter">
                  <div class="sensor-meter-fill" :style="{ width: `${clampPercent(panel.latestPercent)}%` }"></div>
                </div>

                <div class="sensor-chart-frame">
                  <svg viewBox="0 0 360 132" class="sensor-chart" preserveAspectRatio="none" aria-hidden="true">
                    <line v-for="y in [0, 33, 66, 99, 132]" :key="`grid-${y}`" x1="0" :y1="y" x2="360" :y2="y" class="sensor-chart-grid" />
                    <polyline v-if="panel.chartPoints" :points="panel.chartPoints" class="sensor-chart-line" />
                  </svg>
                </div>

                <p class="queue-meta">
                  LAST {{ formatTime(panel.latestAtMs) }}
                  <span> · {{ panel.series.length }} SAMPLES</span>
                  <span v-if="panel.device_id"> · UID {{ panel.device_id }}</span>
                </p>
              </article>
            </div>

            <div v-else class="empty-state">
              <p>SENSOR HISTORY EMPTY.</p>
            </div>
          </article>
        </div>
      </section>
    </template>
  </main>
</template>
