from machine import ADC, Pin, PWM
from time import sleep_ms, ticks_add, ticks_diff, ticks_ms, ticks_us


# Hardware pins. Change these only if the wires move.
AUDIO_IN_PIN = 26        # ADC0 / GP26, after the guitar input buffer
AUDIO_OUT_PIN = 16       # PWM audio output, before the RC output filter
EFFECT_BUTTON_PIN = 14   # Button or footswitch to GND
STATUS_LED_PIN = 15      # External status LED
KNOB_PIN = 27            # Optional potentiometer on ADC1 / GP27

# Audio settings. 8000 Hz is intentionally lo-fi but realistic for MicroPython.
SAMPLE_RATE = 8000
SAMPLE_PERIOD_US = 1000000 // SAMPLE_RATE
PWM_FREQUENCY = 125000
PWM_CENTER = 32768

# Effects.
EFFECTS = ("clean", "fuzz", "tremolo", "bitcrush", "tone")
START_EFFECT = "fuzz"

# Default effect settings.
FUZZ_GAIN = 4
FUZZ_LIMIT = 14000
TREMOLO_HALF_PERIOD = SAMPLE_RATE // 16
BITCRUSH_SHIFT = 6
TONE_FREQUENCY = 440
TONE_AMPLITUDE = 9000

# Input debugging. Printing from the audio loop adds some jitter, but it is useful
# while checking the input circuit. Set this to False after the input works.
DEBUG_INPUT = True
INPUT_LOG_INTERVAL_MS = 1000
INPUT_CENTER_LOW = 26000
INPUT_CENTER_HIGH = 40000
INPUT_WEAK_PEAK = 1200
INPUT_HOT_PEAK = 22000
INPUT_RAIL_LOW = 1500
INPUT_RAIL_HIGH = 64000

# Set this to True after wiring a 10k potentiometer:
# one outside leg to 3V3, the other outside leg to GND, middle leg to GP27.
USE_KNOB = False

# Control timing. Larger values leave more time for the audio loop.
CONTROL_INTERVAL_SAMPLES = 64
BUTTON_DEBOUNCE_MS = 30


audio_in = ADC(Pin(AUDIO_IN_PIN))
knob = ADC(Pin(KNOB_PIN))
audio_out = PWM(Pin(AUDIO_OUT_PIN))
button = Pin(EFFECT_BUTTON_PIN, Pin.IN, Pin.PULL_UP)
status_led = Pin(STATUS_LED_PIN, Pin.OUT)

audio_out.freq(PWM_FREQUENCY)
audio_out.duty_u16(PWM_CENTER)
status_led.off()

effect_index = EFFECTS.index(START_EFFECT)
adc_center = PWM_CENTER

button_last_reading = button.value()
button_stable_value = button_last_reading
button_changed_at_ms = ticks_ms()

input_min = 65535
input_max = 0
input_total = 0
input_count = 0
input_last_log_ms = ticks_ms()


def clamp(value, low, high):
    if value < low:
        return low
    if value > high:
        return high
    return value


def read_average_adc(samples=128):
    total = 0

    for _ in range(samples):
        total += audio_in.read_u16()
        sleep_ms(1)

    return total // samples


def reset_input_stats():
    global input_count
    global input_last_log_ms
    global input_max
    global input_min
    global input_total

    input_min = 65535
    input_max = 0
    input_total = 0
    input_count = 0
    input_last_log_ms = ticks_ms()


def input_status(raw_min, raw_avg, raw_max, peak):
    if raw_min <= INPUT_RAIL_LOW or raw_max >= INPUT_RAIL_HIGH:
        return "CLIPPING_OR_RAIL"
    if raw_avg < INPUT_CENTER_LOW or raw_avg > INPUT_CENTER_HIGH:
        return "CENTER_OFF"
    if peak < 250:
        return "quiet"
    if peak < INPUT_WEAK_PEAK:
        return "weak"
    if peak > INPUT_HOT_PEAK:
        return "hot"
    return "active"


def record_input_sample(raw_sample):
    global input_count
    global input_max
    global input_min
    global input_total

    if not DEBUG_INPUT:
        return

    if raw_sample < input_min:
        input_min = raw_sample

    if raw_sample > input_max:
        input_max = raw_sample

    input_total += raw_sample
    input_count += 1


def maybe_log_input(effect):
    global input_count
    global input_last_log_ms
    global input_max
    global input_min
    global input_total

    if not DEBUG_INPUT or effect == "tone":
        return

    now = ticks_ms()

    if input_count == 0 or ticks_diff(now, input_last_log_ms) < INPUT_LOG_INTERVAL_MS:
        return

    raw_avg = input_total // input_count
    peak_low = adc_center - input_min
    peak_high = input_max - adc_center
    peak = max(abs(peak_low), abs(peak_high))
    span = input_max - input_min
    avg_delta = raw_avg - adc_center

    print(
        "ADC",
        "effect={}".format(effect),
        "min={}".format(input_min),
        "avg={}".format(raw_avg),
        "max={}".format(input_max),
        "center={}".format(adc_center),
        "avg_delta={}".format(avg_delta),
        "peak={}".format(peak),
        "span={}".format(span),
        "status={}".format(input_status(input_min, raw_avg, input_max, peak)),
    )

    input_min = 65535
    input_max = 0
    input_total = 0
    input_count = 0
    input_last_log_ms = now


def knob_value():
    return knob.read_u16()


def current_effect():
    return EFFECTS[effect_index]


def choose_next_effect():
    global effect_index

    effect_index = (effect_index + 1) % len(EFFECTS)
    reset_input_stats()
    print("Effect:", current_effect())


def update_button():
    global button_changed_at_ms
    global button_last_reading
    global button_stable_value

    now = ticks_ms()
    reading = button.value()

    if reading != button_last_reading:
        button_last_reading = reading
        button_changed_at_ms = now

    if ticks_diff(now, button_changed_at_ms) < BUTTON_DEBOUNCE_MS:
        return

    if reading == button_stable_value:
        return

    button_stable_value = reading

    if button_stable_value == 0:
        choose_next_effect()


def update_controls(sample_number):
    if sample_number % CONTROL_INTERVAL_SAMPLES == 0:
        update_button()


def process_clean(centered_sample):
    return centered_sample


def process_fuzz(centered_sample):
    gain = FUZZ_GAIN
    limit = FUZZ_LIMIT

    if USE_KNOB:
        value = knob_value()
        gain = 2 + (value // 10923)       # 2 through 7
        limit = 7000 + (value // 4)       # 7000 through about 23383

    return clamp(centered_sample * gain, -limit, limit)


def process_tremolo(centered_sample, sample_number):
    depth_divisor = 3

    if USE_KNOB:
        depth_divisor = 2 + (knob_value() // 16384)

    phase = sample_number % (TREMOLO_HALF_PERIOD * 2)

    if phase < TREMOLO_HALF_PERIOD:
        return centered_sample

    return centered_sample // depth_divisor


def process_bitcrush(centered_sample):
    shift = BITCRUSH_SHIFT

    if USE_KNOB:
        shift = 4 + (knob_value() // 9363)  # 4 through 10

    return (centered_sample >> shift) << shift


def process_tone(sample_number):
    phase = (sample_number * TONE_FREQUENCY) % SAMPLE_RATE

    if phase < SAMPLE_RATE // 2:
        return TONE_AMPLITUDE

    return -TONE_AMPLITUDE


def process_input_sample(effect, sample_number):
    raw_sample = audio_in.read_u16()
    record_input_sample(raw_sample)
    centered = raw_sample - adc_center

    if effect == "clean":
        changed = process_clean(centered)
    elif effect == "fuzz":
        changed = process_fuzz(centered)
    elif effect == "tremolo":
        changed = process_tremolo(centered, sample_number)
    elif effect == "bitcrush":
        changed = process_bitcrush(centered)
    else:
        changed = centered

    return clamp(changed + PWM_CENTER, 0, 65535)


def next_output_sample(sample_number):
    effect = current_effect()
    maybe_log_input(effect)

    if effect == "tone":
        return clamp(process_tone(sample_number) + PWM_CENTER, 0, 65535)

    return process_input_sample(effect, sample_number)


def audio_loop():
    sample_number = 0
    next_sample_at = ticks_us()

    while True:
        audio_out.duty_u16(next_output_sample(sample_number))

        sample_number += 1
        update_controls(sample_number)

        next_sample_at = ticks_add(next_sample_at, SAMPLE_PERIOD_US)

        while ticks_diff(next_sample_at, ticks_us()) > 0:
            pass

        if ticks_diff(ticks_us(), next_sample_at) > SAMPLE_PERIOD_US:
            next_sample_at = ticks_us()


print("Pico guitar effects processor")
print("Input: GP26 / ADC0 through the input buffer")
print("Output: GP16 PWM through the output filter")
print("Button: GP14 to GND changes effects")
print("Starting effect:", current_effect())

status_led.on()
print("Measuring quiet input center. Do not play yet...")
adc_center = read_average_adc()
print("ADC center:", adc_center)
print("ADC center should usually be near 32768.")
print("Watch for status=active when playing and status=CENTER_OFF or CLIPPING_OR_RAIL if wiring is wrong.")
print("Ready. Start with amp volume low.")

try:
    audio_loop()
finally:
    audio_out.duty_u16(PWM_CENTER)
    status_led.off()
