# Pico Trivia Hub

Classroom electronics trivia game. The instructor runs this hub on a laptop / Pi; students buzz in from their Pico boards using the existing **`pico-buzzer/`** firmware (no firmware changes needed).

## Layout

- `backend/` — Rust + axum API. Holds question pool, game state, scoring. Persists to `backend/data/save.json` after every state change so a crash never wipes the leaderboard.
- `frontend/` — Vue 3 + Vite. Two views: question (with skinny side leaderboard that animates on score changes) and full leaderboard.
- `questions/questions.json` — pool of 60 trivia questions.
- `questions/images/` — SVG placeholder images for the photo / drawing questions. **These are placeholders — swap them with real photos before showing students.**

## Game flow

1. Instructor starts (or resumes) a game from the setup screen. Choose **fixed** order or **shuffled**.
2. The first question appears. State: *waiting for buzz*.
3. The first student to buzz in becomes the answerer. The instructor clicks **✓ Correct** or **✗ Wrong**.
   - **Correct** → +1 point, state → *between questions*. Instructor clicks **Next**.
   - **Wrong** → that student is locked out for this question; the next student in the buzz queue takes over. If nobody else has buzzed, state goes back to *waiting for buzz*.
4. If everybody is stumped, the instructor clicks **Skip question** to give up and move on with no points awarded.
5. Once a student has buzzed in on a given question, they cannot try again on that question.
6. Buzzing too soon does nothing — there's no disqualification, the buzz just doesn't help them.

The instructor can switch between the **question view** and the **full leaderboard** at any time using the toggle button in the top bar. The skinny side leaderboard is always visible during the question view and animates rank changes.

## Run the backend

```bash
cd backend
cargo run
```

Listens on `0.0.0.0:8788`. Pico clients on your local network can POST buzzes to `http://<your-laptop-ip>:8788/api/buzz`.

The backend reads `../questions/questions.json` at startup and serves images at `/images/<filename>`.

## Run the frontend

```bash
cd frontend
npm install
npm run dev -- --host
```

Default URL: `http://localhost:5174`. Override the API base with `VITE_API_BASE=http://<host>:8788`.

## Pointing the Pico buzzers at this hub

The student buzzers are the same `pico-buzzer/` firmware as the buzzer-game project. Set each Pico's `SERVER_HOST` to your laptop / Pi IP and `SERVER_PORT` to `8788` (the trivia hub port — note that's different from the buzzer-game hub on `8787`).

The trivia backend accepts the same `POST /api/buzz` payload shape as `pico-buzzer-hub`, so no Pico-side code changes are required.

## Crash safety

Every state change (judge, skip, next, buzz, toggle, reveal) writes `backend/data/save.json` atomically. If the backend crashes mid-class, restart it; the frontend will show a "Resume Saved Game" card with the question index, player count, and how long ago it was saved.

To start fresh, click **End Game** — that deletes the save file.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET  | `/api/health`        | health check |
| GET  | `/api/questions`     | full question pool |
| GET  | `/api/state`         | current state snapshot (devices, game, save info) |
| GET  | `/api/save-info`     | whether a save file exists + summary |
| POST | `/api/game/start`    | new game (`{ shuffled: bool, include_ids?: [string] }`) |
| POST | `/api/game/resume`   | load `save.json` |
| POST | `/api/game/end`      | clear game + delete save file |
| POST | `/api/game/judge`    | judge current answerer (`{ correct: bool }`) |
| POST | `/api/game/skip`     | skip current question, no points |
| POST | `/api/game/next`     | advance to next question |
| POST | `/api/game/reveal`   | show the answer on screen |
| POST | `/api/view/toggle`   | toggle leaderboard view (server-controlled) |
| POST | `/api/buzz`          | student device buzzes in |
| POST | `/api/check-in`      | student device heartbeat |

## Replacing the placeholder images

The SVGs in `questions/images/` are stylized line drawings, not real photos. Swap any of them with a real photo (or sharper drawing) by replacing the file at the same path. Allowed file types: anything a browser can render (SVG, PNG, JPG). If you change the filename, update the matching entry in `questions.json`.
