from machine import Pin
from time import sleep, ticks_diff, ticks_ms

from buzzer_net import BuzzerHubClient


# Update these values before uploading.
WIFI_SSID = "starleap-24"
WIFI_PASSWORD = "starleapstem"
SERVER_HOST = "192.168.8.247"
SERVER_PORT = 8787
SERVER_PATH = "/api/buzz"
CHECKIN_PATH = "/api/check-in"
DEBUG_WIFI = True

# Change these only if the wires move to different pins.
BUTTON_PIN = 14
STATUS_LED_PIN = 15

# These timing values can usually stay the same.
BUTTON_DEBOUNCE_MS = 20
BUTTON_POLL_SECONDS = 0.01
WIFI_TIMEOUT_SECONDS = 15
HTTP_TIMEOUT_SECONDS = 5
HEARTBEAT_INTERVAL_MS = 10000

button = Pin(BUTTON_PIN, Pin.IN, Pin.PULL_UP)
status_led = Pin(STATUS_LED_PIN, Pin.OUT)

# This helper object handles Wi-Fi, MAC addresses, and talking to the laptop server.
hub = BuzzerHubClient(
    wifi_ssid=WIFI_SSID,
    wifi_password=WIFI_PASSWORD,
    server_host=SERVER_HOST,
    app_kind="buzzer",
    server_port=SERVER_PORT,
    server_path=SERVER_PATH,
    checkin_path=CHECKIN_PATH,
    debug_wifi=DEBUG_WIFI,
    wifi_timeout_seconds=WIFI_TIMEOUT_SECONDS,
    http_timeout_seconds=HTTP_TIMEOUT_SECONDS,
)


# Blink the LED to show success or failure.
def flash(times, on_seconds=0.08, off_seconds=0.08):
    for _ in range(times):
        status_led.on()
        sleep(on_seconds)
        status_led.off()
        sleep(off_seconds)


def button_is_pressed():
    return button.value() == 0


# Tell the laptop server that this Pico is online.
def send_check_in():
    if not hub.connect_wifi():
        return False

    try:
        ok, status_text = hub.send_check_in(BUTTON_PIN, button_is_pressed())
    except OSError as exc:
        print("Failed to send check-in:", exc)
        return False

    if DEBUG_WIFI:
        if ok:
            print("Check-in sent -", status_text)
        else:
            print("Check-in failed:", status_text)

    return ok


# Send a real buzz when the button is pressed.
def send_buzz():
    if not hub.connect_wifi():
        flash(3, 0.2, 0.1)
        return False

    try:
        ok, status_text = hub.send_buzz(BUTTON_PIN, button_is_pressed())
    except OSError as exc:
        print("Failed to reach server:", exc)
        flash(3, 0.2, 0.1)
        return False

    if ok:
        print("Buzz sent -", status_text)
        flash(2, 0.1, 0.08)
        return True

    print("Buzz failed:", status_text)
    flash(4, 0.05, 0.05)
    return False


print("Pico buzzer ready.")
print("Unique ID:", hub.device_id)
device_mac = hub.get_mac_address()
if device_mac:
    print("Wi-Fi MAC:", device_mac)
status_led.off()
flash(1, 0.2, 0.1)
last_check_in_ms = ticks_ms()
last_button_pressed = button_is_pressed()

if hub.connect_wifi():
    send_check_in()
    last_check_in_ms = ticks_ms()

# Keep watching the button forever.
while True:
    current_button_pressed = button_is_pressed()

    if current_button_pressed != last_button_pressed:
        sleep(BUTTON_DEBOUNCE_MS / 1000)
        current_button_pressed = button_is_pressed()

        if current_button_pressed != last_button_pressed:
            last_button_pressed = current_button_pressed

            if current_button_pressed:
                if send_buzz():
                    last_check_in_ms = ticks_ms()
            else:
                if send_check_in():
                    last_check_in_ms = ticks_ms()

    if ticks_diff(ticks_ms(), last_check_in_ms) >= HEARTBEAT_INTERVAL_MS:
        send_check_in()
        last_check_in_ms = ticks_ms()

    sleep(BUTTON_POLL_SECONDS)
