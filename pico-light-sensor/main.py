from machine import ADC, Pin
from time import sleep, ticks_diff, ticks_ms

from pico_hub import PicoHubClient


# Update these values before uploading.
WIFI_SSID = "starleap-24"
WIFI_PASSWORD = "starleapstem"
SERVER_HOST = "192.168.8.247"
SERVER_PORT = 8787
CHECKIN_PATH = "/api/check-in"
LIGHT_PATH = "/api/light-reading"
DEBUG_WIFI = True
DEBUG_SENSOR = True

# This assumes the photoresistor voltage divider is connected to ADC0 / GP26.
LIGHT_SENSOR_PIN = 26
STATUS_LED_PIN = "LED"

# These timing values can usually stay the same.
SAMPLE_INTERVAL_MS = 100
HEARTBEAT_INTERVAL_MS = 10000
WIFI_TIMEOUT_SECONDS = 15
HTTP_TIMEOUT_SECONDS = 5

light_sensor = ADC(LIGHT_SENSOR_PIN)
status_led = Pin(STATUS_LED_PIN, Pin.OUT)

hub = PicoHubClient(
    wifi_ssid=WIFI_SSID,
    wifi_password=WIFI_PASSWORD,
    server_host=SERVER_HOST,
    app_kind="light-sensor",
    server_port=SERVER_PORT,
    light_path=LIGHT_PATH,
    checkin_path=CHECKIN_PATH,
    debug_wifi=DEBUG_WIFI,
    wifi_timeout_seconds=WIFI_TIMEOUT_SECONDS,
    http_timeout_seconds=HTTP_TIMEOUT_SECONDS,
)


def flash(times, on_seconds=0.08, off_seconds=0.08):
    for _ in range(times):
        status_led.on()
        sleep(on_seconds)
        status_led.off()
        sleep(off_seconds)


def read_light_raw():
    return light_sensor.read_u16()


def raw_to_percent(raw_value):
    return (raw_value / 65535) * 100


def current_light_data():
    raw_value = read_light_raw()
    percent = raw_to_percent(raw_value)
    return raw_value, percent


def send_check_in():
    raw_value, percent = current_light_data()

    if not hub.connect_wifi():
        return False

    try:
        ok, status_text = hub.send_check_in(
            extra_fields={
                "light_pin": LIGHT_SENSOR_PIN,
                "light_raw": raw_value,
                "light_percent": percent,
            }
        )
    except OSError as exc:
        print("Failed to send check-in:", exc)
        return False

    if DEBUG_WIFI:
        if ok:
            print("Check-in sent -", status_text)
        else:
            print("Check-in failed:", status_text)

    return ok


def send_light_reading():
    raw_value, percent = current_light_data()

    if DEBUG_SENSOR:
        print("Light:", raw_value, "({:.1f}%)".format(percent))

    if not hub.connect_wifi():
        flash(3, 0.2, 0.1)
        return False

    try:
        ok, status_text = hub.send_light_reading(LIGHT_SENSOR_PIN, raw_value, percent)
    except OSError as exc:
        print("Failed to reach server:", exc)
        flash(3, 0.2, 0.1)
        return False

    if DEBUG_WIFI:
        if ok:
            print("Light sample sent -", status_text)
        else:
            print("Light sample failed:", status_text)

    return ok


print("Pico light sensor ready.")
print("Unique ID:", hub.device_id)
device_mac = hub.get_mac_address()
if device_mac:
    print("Wi-Fi MAC:", device_mac)
status_led.off()
flash(1, 0.2, 0.1)
last_sample_ms = ticks_ms() - SAMPLE_INTERVAL_MS
last_check_in_ms = ticks_ms()

if hub.connect_wifi():
    send_check_in()
    last_check_in_ms = ticks_ms()

while True:
    if ticks_diff(ticks_ms(), last_sample_ms) >= SAMPLE_INTERVAL_MS:
        if send_light_reading():
            last_sample_ms = ticks_ms()

    if ticks_diff(ticks_ms(), last_check_in_ms) >= HEARTBEAT_INTERVAL_MS:
        if send_check_in():
            last_check_in_ms = ticks_ms()

    sleep(0.05)
