# Pico Buzzer Hub

This folder is the laptop-side app for the classroom buzzer system.

## Layout

- `backend/`: Rust API that accepts buzzes from Pico boards
- `frontend/`: Vue dashboard that shows the current buzz order

## Intended flow

1. Start the Rust backend on your laptop.
2. Put the Pico boards on the same Wi-Fi network.
3. Set each Pico's `SERVER_HOST` to your laptop's LAN IP address.
4. Open the Vue frontend in a browser to watch the buzz queue.
5. Press `Reset Round` to clear the queue for the next question.

## Current backend behavior

- Records buzzes in the order they arrive
- Accepts only the first buzz from each `student_id` per round
- Keeps everything in memory for now
- Exposes `POST /api/buzz`, `GET /api/state`, `POST /api/reset`, and `GET /api/health`

## Run the backend

```bash
cd /Users/slug/pico/pico-buzzer-hub/backend
cargo run
```

The backend listens on `0.0.0.0:8787`, so Pico clients on your local network can reach it.

## Run the frontend

```bash
cd /Users/slug/pico/pico-buzzer-hub/frontend
npm install
npm run dev -- --host
```

By default the frontend looks for the backend at `http://127.0.0.1:8787`. You can override that with `VITE_API_BASE`.

## Next steps you may want

- Lock a round after the first buzz only
- Add teacher controls for reopening a round
- Show student names instead of IDs
- Persist results to disk
- Pair Pico boards to roster entries
