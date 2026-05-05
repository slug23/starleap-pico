<script setup>
import { computed, onMounted, onUnmounted, ref } from "vue";

const apiBase = import.meta.env.VITE_API_BASE ?? "http://127.0.0.1:8788";

const state = ref(null);
const error = ref("");
const loading = ref(false);
const busy = ref(false);
const view = ref("question");
const shuffle = ref(false);
const showSetupSidebar = ref(false);
const nowMs = ref(Date.now());

let pollTimerId = null;
let clockTimerId = null;

const game = computed(() => state.value?.game ?? null);
const phase = computed(() => game.value?.phase ?? null);
const currentQuestion = computed(() => game.value?.current_question ?? null);
const scores = computed(() => game.value?.scores ?? []);
const saveInfo = computed(() => state.value?.save_info ?? { exists: false, summary: null });

const questionImageUrls = computed(() =>
  (currentQuestion.value?.images ?? []).map((name) => `${apiBase}/images/${name}`),
);

const questionImageLabels = computed(() => {
  const images = currentQuestion.value?.images ?? [];
  const labels = currentQuestion.value?.image_labels ?? [];
  return images.map(
    (_, index) => labels[index] ?? String.fromCharCode(65 + index),
  );
});

const answerer = computed(() => game.value?.current_answerer ?? null);
const lockedOut = computed(() => game.value?.locked_out ?? []);
const buzzQueue = computed(() => game.value?.buzz_queue ?? []);
const checkedIn = computed(() => game.value?.checked_in ?? []);
const showCheckInFlag = computed(() => game.value?.show_check_in ?? false);
const inCheckIn = computed(
  () => phase.value === "check_in" || showCheckInFlag.value,
);
const devices = computed(() => state.value?.devices ?? []);
const onlineWindowMs = computed(() => state.value?.online_window_ms ?? 30000);

function isOnline(device) {
  return nowMs.value - Number(device.last_seen_ms ?? 0) <= onlineWindowMs.value;
}

const checkInDevices = computed(() =>
  [...devices.value].sort((a, b) => {
    const aChecked = checkedIn.value.includes(a.player_id) ? 0 : 1;
    const bChecked = checkedIn.value.includes(b.player_id) ? 0 : 1;
    if (aChecked !== bChecked) return aChecked - bChecked;
    const aOnline = isOnline(a) ? 0 : 1;
    const bOnline = isOnline(b) ? 0 : 1;
    if (aOnline !== bOnline) return aOnline - bOnline;
    return (a.player_name || a.player_id).localeCompare(b.player_name || b.player_id);
  }),
);

const checkedInCount = computed(() => checkedIn.value.length);
const onlineCount = computed(() => devices.value.filter(isOnline).length);

const totalQuestions = computed(() => game.value?.total_questions ?? 0);
const currentIndex = computed(() => game.value?.current_index ?? 0);
const questionNumberLabel = computed(() => {
  if (!totalQuestions.value) {
    return "";
  }
  return `Q ${currentIndex.value + 1} of ${totalQuestions.value}`;
});

const isFinished = computed(() => phase.value === "finished");
const showLeaderboardOverride = computed(() => game.value?.show_leaderboard ?? false);
const effectiveView = computed(() => {
  if (isFinished.value) {
    return "leaderboard";
  }
  if (showLeaderboardOverride.value) {
    return "leaderboard";
  }
  return view.value;
});

const phaseLabel = computed(() => {
  switch (phase.value) {
    case "check_in":
      return "Check-in";
    case "question_open":
      return "Waiting for buzz";
    case "answering":
      return "Answering";
    case "between_questions":
      return "Between questions";
    case "finished":
      return "Game over";
    default:
      return "";
  }
});

const answererLabel = computed(() => {
  if (!answerer.value) return "";
  return answerer.value.player_name || answerer.value.player_id;
});

const queueLabels = computed(() =>
  buzzQueue.value.map((entry) => entry.player_name || entry.player_id),
);

function playerLabel(entry) {
  return entry.player_name || entry.player_id || "player";
}

function formatRelativeTime(ms) {
  if (!ms) return "";
  const elapsed = Math.max(0, Math.round((nowMs.value - Number(ms)) / 1000));
  if (elapsed < 5) return "just now";
  if (elapsed < 60) return `${elapsed}s ago`;
  const minutes = Math.round(elapsed / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

async function loadState() {
  try {
    const response = await fetch(`${apiBase}/api/state`);
    if (!response.ok) throw new Error(`state ${response.status}`);
    state.value = await response.json();
    error.value = "";
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Failed to load hub state.";
  }
}

async function postAction(path, body) {
  if (busy.value) return null;
  busy.value = true;
  try {
    const response = await fetch(`${apiBase}${path}`, {
      method: "POST",
      headers: body ? { "Content-Type": "application/json" } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(`${path} ${response.status} ${text}`);
    }
    const data = await response.json().catch(() => null);
    if (data && data.state) {
      state.value = data.state;
    } else if (data && data.pool_version !== undefined) {
      state.value = data;
    } else {
      await loadState();
    }
    return data;
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Request failed.";
    return null;
  } finally {
    busy.value = false;
  }
}

async function startGame() {
  await postAction("/api/game/start", { shuffled: shuffle.value });
  view.value = "question";
}

async function resumeGame() {
  await postAction("/api/game/resume", {});
  view.value = "question";
}

async function endGame() {
  if (!confirm("End this game and clear the saved state?")) return;
  await postAction("/api/game/end", {});
  view.value = "question";
}

async function judge(correct) {
  await postAction("/api/game/judge", { correct });
}

async function skipQuestion() {
  await postAction("/api/game/skip", {});
}

async function nextQuestion() {
  await postAction("/api/game/next", {});
}

async function revealAnswer() {
  await postAction("/api/game/reveal", {});
}

async function beginQuestions() {
  await postAction("/api/game/begin-questions", {});
  view.value = "question";
}

async function showCheckInOverlay() {
  await postAction("/api/game/show-check-in", {});
}

async function toggleLeaderboardServer() {
  await postAction("/api/view/toggle", {});
}

function toggleViewLocal() {
  view.value = view.value === "question" ? "leaderboard" : "question";
}

onMounted(() => {
  void loadState();
  pollTimerId = window.setInterval(() => {
    void loadState();
  }, 400);
  clockTimerId = window.setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
});

onUnmounted(() => {
  if (pollTimerId !== null) window.clearInterval(pollTimerId);
  if (clockTimerId !== null) window.clearInterval(clockTimerId);
});
</script>

<template>
  <main class="trivia-shell">
    <header class="topbar">
      <div class="topbar-left">
        <h1>Electronics Trivia</h1>
        <span v-if="state" class="pool-meta">
          {{ state.pool_title }} · {{ state.pool_size }} questions
        </span>
      </div>
      <div v-if="game" class="topbar-right">
        <span class="phase-pill" :class="`phase-${phase}`">{{ phaseLabel }}</span>
        <span v-if="!inCheckIn" class="qcount">{{ questionNumberLabel }}</span>
        <button
          v-if="!isFinished && !inCheckIn"
          class="btn btn-ghost"
          @click="showCheckInOverlay"
        >
          Check-in
        </button>
        <button
          v-if="!isFinished && !inCheckIn"
          class="btn btn-ghost"
          @click="toggleViewLocal"
        >
          {{ effectiveView === "question" ? "Leaderboard" : "Question" }}
        </button>
        <button class="btn btn-warn" @click="endGame">End Game</button>
      </div>
    </header>

    <p v-if="error" class="error-banner">{{ error }}</p>

    <!-- SETUP -->
    <section v-if="!game" class="setup-screen">
      <div class="setup-card">
        <h2>New Game</h2>
        <label class="shuffle-toggle">
          <input type="checkbox" v-model="shuffle" />
          Shuffle question order
        </label>
        <p class="setup-note">
          Plays {{ state ? state.pool_size : 60 }} questions in
          {{ shuffle ? "random" : "fixed" }} order.
        </p>
        <button class="btn btn-primary btn-lg" :disabled="busy" @click="startGame">
          Start New Game
        </button>
      </div>

      <div class="setup-card" v-if="saveInfo.exists">
        <h2>Resume Game</h2>
        <p class="setup-note">
          Question {{ saveInfo.summary.current_index + 1 }} of
          {{ saveInfo.summary.total_questions }} ·
          {{ saveInfo.summary.player_count }} player(s)
          <br />
          Last saved {{ formatRelativeTime(saveInfo.summary.last_saved_at_ms) }}
        </p>
        <button class="btn btn-primary btn-lg" :disabled="busy" @click="resumeGame">
          Resume Saved Game
        </button>
      </div>

      <div class="setup-card" v-else>
        <h2>Resume Game</h2>
        <p class="setup-note muted">No saved game on disk.</p>
      </div>
    </section>

    <!-- IN-GAME: CHECK-IN (real phase or overlay) -->
    <section v-else-if="inCheckIn" class="checkin-screen">
      <div class="checkin-header">
        <h2>Check-in</h2>
        <p class="checkin-sub">
          Have each student press their buzzer to confirm it's connected.
          <span class="checkin-counts">
            {{ checkedInCount }} checked in · {{ onlineCount }} online ·
            {{ devices.length }} known
          </span>
        </p>
      </div>

      <div v-if="checkInDevices.length" class="checkin-grid">
        <div
          v-for="device in checkInDevices"
          :key="device.player_id"
          class="checkin-card"
          :class="{
            'checkin-pressed': checkedIn.includes(device.player_id),
            'checkin-online': isOnline(device) && !checkedIn.includes(device.player_id),
          }"
        >
          <div class="checkin-name">{{ device.player_name || device.player_id }}</div>
          <div class="checkin-status">
            <span v-if="checkedIn.includes(device.player_id)" class="badge badge-checked">
              ✓ checked in
            </span>
            <span v-else-if="isOnline(device)" class="badge badge-online">
              online — press buzzer
            </span>
            <span v-else class="badge badge-offline">offline</span>
          </div>
        </div>
      </div>
      <div v-else class="checkin-empty">
        <p>Waiting for the first Pico to check in…</p>
        <p class="checkin-hint">
          Picos appear here once they reach the hub. If nothing shows up, verify
          each Pico's <code>SERVER_HOST</code> is set to this laptop's IP and
          <code>SERVER_PORT</code> is <code>8788</code>.
        </p>
      </div>

      <footer class="instructor-bar instructor-bar-checkin">
        <button
          class="btn btn-primary btn-xl"
          :disabled="busy"
          @click="beginQuestions"
        >
          {{ phase === "check_in" ? "Start Trivia →" : "Resume Game →" }}
        </button>
      </footer>
    </section>

    <!-- IN-GAME: QUESTION VIEW -->
    <section v-else-if="effectiveView === 'question' && currentQuestion" class="question-screen">
      <div class="question-main">
        <div class="question-header">
          <span class="question-category">{{ currentQuestion.category }}</span>
          <span class="question-id">{{ currentQuestion.id }}</span>
        </div>

        <p class="question-prompt">{{ currentQuestion.prompt }}</p>

        <div class="question-images" v-if="questionImageUrls.length">
          <figure
            v-for="(url, index) in questionImageUrls"
            :key="url"
            class="question-image-frame"
          >
            <img :src="url" :alt="`question image ${index + 1}`" />
            <figcaption v-if="questionImageUrls.length > 1">
              {{ questionImageLabels[index] }}
            </figcaption>
          </figure>
        </div>

        <div class="answer-block" v-if="game.answer_revealed">
          <p class="answer-label">Answer</p>
          <p class="answer-text">{{ currentQuestion.answer }}</p>
        </div>
        <button
          v-else
          class="btn btn-ghost btn-reveal"
          :disabled="busy"
          @click="revealAnswer"
        >
          Reveal answer (instructor only)
        </button>

        <div class="status-strip">
          <div v-if="phase === 'question_open'" class="status-waiting">
            <div class="pulse-dot"></div>
            Waiting for first buzz...
          </div>
          <div v-else-if="phase === 'answering'" class="status-answering">
            <span class="answerer-tag">{{ answererLabel }}</span>
            is answering
            <span v-if="queueLabels.length" class="queue-tail">
              · queued: {{ queueLabels.join(", ") }}
            </span>
          </div>
          <div v-else-if="phase === 'between_questions'" class="status-between">
            Question {{ currentIndex + 1 }} resolved. Click Next.
          </div>
        </div>
      </div>

      <aside class="side-leaderboard">
        <p class="side-leaderboard-title">Leaderboard</p>
        <TransitionGroup tag="ol" name="lb" class="side-leaderboard-list">
          <li
            v-for="entry in scores"
            :key="entry.player_id"
            class="lb-item"
            :class="{
              'lb-item-answering': answerer && answerer.player_id === entry.player_id,
              'lb-item-locked': lockedOut.includes(entry.player_id),
            }"
          >
            <span class="lb-rank">{{ scores.indexOf(entry) + 1 }}</span>
            <span class="lb-name">{{ playerLabel(entry) }}</span>
            <span class="lb-score">{{ entry.score }}</span>
          </li>
          <li v-if="!scores.length" key="empty" class="lb-empty">
            No players yet — buzz a Pico to join.
          </li>
        </TransitionGroup>
      </aside>

      <footer class="instructor-bar">
        <template v-if="phase === 'answering'">
          <button class="btn btn-correct btn-xl" :disabled="busy" @click="judge(true)">
            ✓ Correct
          </button>
          <button class="btn btn-wrong btn-xl" :disabled="busy" @click="judge(false)">
            ✗ Wrong
          </button>
          <button class="btn btn-ghost" :disabled="busy" @click="skipQuestion">
            Skip question
          </button>
        </template>
        <template v-else-if="phase === 'question_open'">
          <button class="btn btn-skip btn-lg" :disabled="busy" @click="skipQuestion">
            Skip question (no points)
          </button>
        </template>
        <template v-else-if="phase === 'between_questions'">
          <button class="btn btn-primary btn-xl" :disabled="busy" @click="nextQuestion">
            Next question →
          </button>
        </template>
        <template v-else-if="isFinished">
          <button class="btn btn-primary btn-lg" :disabled="busy" @click="endGame">
            Finish
          </button>
        </template>
      </footer>
    </section>

    <!-- IN-GAME: LEADERBOARD VIEW -->
    <section v-else-if="effectiveView === 'leaderboard'" class="leaderboard-screen">
      <h2 class="leaderboard-heading">
        {{ isFinished ? "Final Leaderboard" : "Leaderboard" }}
      </h2>
      <TransitionGroup tag="ol" name="lb-big" class="leaderboard-list">
        <li
          v-for="(entry, index) in scores"
          :key="entry.player_id"
          class="leaderboard-item"
          :class="{
            'leaderboard-1st': index === 0,
            'leaderboard-2nd': index === 1,
            'leaderboard-3rd': index === 2,
          }"
        >
          <span class="leaderboard-rank">{{ index + 1 }}</span>
          <span class="leaderboard-name">{{ playerLabel(entry) }}</span>
          <span class="leaderboard-score">{{ entry.score }}</span>
        </li>
        <li v-if="!scores.length" key="empty" class="leaderboard-empty">
          No players yet — students need to buzz at least once to appear.
        </li>
      </TransitionGroup>

      <footer class="instructor-bar instructor-bar-leaderboard">
        <button v-if="!isFinished" class="btn btn-primary btn-lg" @click="toggleViewLocal">
          Back to question
        </button>
        <button v-else class="btn btn-warn btn-lg" @click="endGame">End game</button>
      </footer>
    </section>
  </main>
</template>
