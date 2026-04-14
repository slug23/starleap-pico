# Pico Sound

This folder is a small MicroPython project for a Raspberry Pi Pico that plays a note when a button is pressed.

## Wiring

- Button: `GP14` to `GND`
- Status LED: `GP15`
- Speaker control signal: `GP16`

## Important speaker note

Your `AST-01708MR-R` is an `8Ω` speaker, so do **not** connect it directly to a Pico GPIO pin.

Instead, connect `GP16` to the **input** of a small transistor driver or amplifier stage, and connect the speaker to that driver.

At minimum, use:

- a transistor stage or small audio amplifier module
- a shared ground between the Pico and the driver
- a current-limiting/output stage that is appropriate for an `8Ω`, `0.2W` speaker

If you want, we can sketch a simple transistor wiring next.

## What it does

- Waits for a button press on `GP14`
- Debounces the button
- Plays a single `440 Hz` note from `GP16`
- Lights the LED on `GP15` while the note is playing

## Tuning

You can change these values near the top of [`main.py`](/Users/slug/pico/pico-sound/main.py):

- `NOTE_FREQUENCY_HZ` to change pitch
- `NOTE_DURATION_MS` to change how long the note plays
- `NOTE_DUTY` to change the PWM duty cycle
