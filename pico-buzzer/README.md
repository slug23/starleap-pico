# Pico Buzzer

This folder is the MicroPython client project for a single student's Raspberry Pi Pico 2 W.

## Wiring

- Button: `GP14` to `GND`
- Status LED: `GP15`

## What it does

- Connects the Pico to Wi-Fi
- Waits for the button to be pressed
- Sends an HTTP `POST` to the laptop backend at `/api/buzz`
- Flashes the LED to show whether the send worked

## Per-device setup

Before uploading [`main.py`](/Users/slug/pico/pico-buzzer/main.py), edit these values near the top of the file:

- `WIFI_SSID`
- `WIFI_PASSWORD`
- `SERVER_HOST`
- `STUDENT_ID`

Each student's Pico should have its own `STUDENT_ID`, for example:

- `student-01`
- `student-02`
- `alice`
- `table-3`

## Upload notes

Because the Pico runs `main.py` automatically, keep each hardware project in its own folder. For this buzzer project, upload the files from this directory to the board and make sure the server is reachable on the same Wi-Fi network.
