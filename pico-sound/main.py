from machine import Pin, PWM
from time import sleep_ms

BUTTON_PIN = 14
STATUS_LED_PIN = 15
SPEAKER_PIN = 16

NOTE_FREQUENCY_HZ = 440
NOTE_DURATION_MS = 1000
NOTE_DUTY = 20000

BUTTON_DEBOUNCE_MS = 20
BUTTON_POLL_MS = 10

button = Pin(BUTTON_PIN, Pin.IN, Pin.PULL_UP)
status_led = Pin(STATUS_LED_PIN, Pin.OUT)
speaker = PWM(Pin(SPEAKER_PIN))


def button_is_pressed():
    return button.value() == 0


def play_note(frequency_hz=NOTE_FREQUENCY_HZ, duration_ms=NOTE_DURATION_MS):
    speaker.freq(frequency_hz)
    speaker.duty_u16(NOTE_DUTY)
    status_led.on()
    sleep_ms(duration_ms)
    speaker.duty_u16(0)
    status_led.off()


print("Press the button on GP14 to play a note on the speaker driver connected to GP16.")
print("Status LED on GP15 turns on while the note is playing.")
speaker.duty_u16(0)
status_led.off()
last_button_pressed = button_is_pressed()

try:
    while True:
        current_button_pressed = button_is_pressed()

        if current_button_pressed and not last_button_pressed:
            sleep_ms(BUTTON_DEBOUNCE_MS)

            if button_is_pressed():
                play_note()

                while button_is_pressed():
                    sleep_ms(BUTTON_POLL_MS)

                current_button_pressed = False

        last_button_pressed = current_button_pressed
        sleep_ms(BUTTON_POLL_MS)
finally:
    speaker.duty_u16(0)
    speaker.deinit()
    status_led.off()
