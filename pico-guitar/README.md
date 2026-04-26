# Pico Guitar

This folder is a MicroPython guitar effects processor project for a Raspberry Pi Pico 2 W.

## Safety

- Do not connect a guitar, amp output, speaker output, headphone output, or 9V pedal supply directly to a Pico pin.
- The guitar input must go through the bias and buffer circuit from the lesson before it reaches `GP26`.
- The Pico output must go through the output filter before it reaches an amp input.
- The Pico cannot drive a speaker or headphones directly.
- Start with the amplifier volume low.

## Wiring

- Breadboard `+5V` rail: Pico physical pin 39 / `VSYS`
- Breadboard `GND` rail: Pico physical pin 38 / `GND`
- LM358P pin 8: breadboard `+5V` rail
- LM358P pin 4: breadboard `GND` rail
- Bias circuit: Pico physical pin 36 / `3V3(OUT)` through two 100k resistors to make about 1.65V
- Guitar input buffer output: `GP26` / `ADC0`
- PWM audio output: `GP16` through the RC output filter
- Effect button or footswitch: `GP14` to `GND`
- Status LED: `GP15` to LED long leg, LED short leg through 220 ohm resistor to `GND`
- Optional effect knob: 10k potentiometer with outside legs to `3V3` and `GND`, middle leg to `GP27`
- All jack sleeves, Pico `GND`, op amp `GND`, and output-filter `GND` must share ground

## What it does

- Measures the quiet ADC center when the program starts
- Reads guitar audio from `GP26`
- Processes the sound with one of four effects
- Outputs filtered PWM audio from `GP16`
- Switches effects when the `GP14` button is pressed

Effects:

- `clean`
- `fuzz`
- `tremolo`
- `bitcrush`
- `tone` - generates a 440 Hz test tone and ignores the guitar input

## Upload notes

Upload `main.py` from this folder to the Pico 2 W.

If you wire the optional potentiometer, change this line near the top of `main.py`:

```python
USE_KNOB = True
```

Keep the guitar quiet while the program starts. The code measures the input's resting voltage for about a second before audio processing begins.

Use the `tone` effect to test the output filter, output buffer, output jack, and amp connection without relying on the guitar input circuit.

## Input debugging

`main.py` prints ADC input stats about once per second in every mode except `tone`.

Example:

```text
ADC effect=fuzz min=31820 avg=32910 max=34240 center=32888 avg_delta=22 peak=1352 span=2420 status=active
```

What to look for:

- `center` should usually be near `32768`
- `avg` should also stay near the center when you are not playing
- `peak` and `span` should grow when you play the guitar
- `status=quiet` means the input is barely moving
- `status=weak` means the input is moving, but not much
- `status=active` means the Pico is seeing a reasonable signal
- `status=CENTER_OFF` usually means the 1.65V bias is wrong
- `status=CLIPPING_OR_RAIL` usually means the signal is hitting 0V or 3.3V and the input circuit needs attention

After the input works, set this near the top of `main.py`:

```python
DEBUG_INPUT = False
```
