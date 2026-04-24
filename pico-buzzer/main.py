from machine import Pin
from time import sleep, ticks_add, ticks_diff, ticks_ms, ticks_us
import socket

from buzzer_net import BuzzerHubClient


# Update these values before uploading.
WIFI_SSID = "starleap-24"
WIFI_PASSWORD = "starleapstem"
SERVER_HOST = "192.168.8.247"
SERVER_PORT = 8787
SERVER_PATH = "/api/buzz"
CHECKIN_PATH = "/api/check-in"
CALIBRATION_PORT = 8788
DEBUG_WIFI = True
PLAYER_NAME = "slug"
FIRMWARE_VERSION = "2026.04.23.5"

# Change these only if the wires move to different pins.
BUTTON_PIN = 14
STATUS_LED_PIN = 15

# These timing values can usually stay the same.
BUTTON_DEBOUNCE_MS = 20
BUTTON_POLL_SECONDS = 0.01
WIFI_TIMEOUT_SECONDS = 15
HTTP_TIMEOUT_SECONDS = 5
HEARTBEAT_INTERVAL_MS = 10000
GO_STATE_MAX_AGE_US = 30000000

button = Pin(BUTTON_PIN, Pin.IN, Pin.PULL_UP)
status_led = Pin(STATUS_LED_PIN, Pin.OUT)

# This helper object handles Wi-Fi, MAC addresses, and talking to the laptop server.
hub = BuzzerHubClient(
    wifi_ssid=WIFI_SSID,
    wifi_password=WIFI_PASSWORD,
    server_host=SERVER_HOST,
    player_name=PLAYER_NAME,
    app_kind="buzzer",
    server_port=SERVER_PORT,
    server_path=SERVER_PATH,
    checkin_path=CHECKIN_PATH,
    calibration_port=CALIBRATION_PORT,
    firmware_version=FIRMWARE_VERSION,
    debug_wifi=DEBUG_WIFI,
    wifi_timeout_seconds=WIFI_TIMEOUT_SECONDS,
    http_timeout_seconds=HTTP_TIMEOUT_SECONDS,
)

calibration_socket = None
current_round_id = None
go_ticks_us = None
last_go_delay_us = None
last_go_received_ticks_us = None
last_checkin_rtt_us = None


# Blink the LED to show success or failure.
def flash(times, on_seconds=0.08, off_seconds=0.08):
    for _ in range(times):
        status_led.on()
        sleep(on_seconds)
        status_led.off()
        sleep(off_seconds)


def button_is_pressed():
    return button.value() == 0


def ensure_calibration_server():
    global calibration_socket

    if calibration_socket is not None:
        return True

    try:
        listener = socket.socket()

        try:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        except (AttributeError, OSError):
            pass

        listener.bind(("0.0.0.0", CALIBRATION_PORT))
        listener.listen(2)
        listener.setblocking(False)
    except OSError as exc:
        print("Failed to start calibration server:", exc)

        try:
            listener.close()
        except (NameError, OSError):
            pass

        return False

    calibration_socket = listener
    print("Calibration auto-ack server listening on port", CALIBRATION_PORT)
    return True


def query_value(request_text, name):
    first_line = request_text.split("\r\n", 1)[0]
    parts = first_line.split(" ")

    if len(parts) < 2 or "?" not in parts[1]:
        return None

    query = parts[1].split("?", 1)[1]
    prefix = name + "="

    for item in query.split("&"):
        if item.startswith(prefix):
            return item[len(prefix) :]

    return None


def query_int(request_text, name):
    value = query_value(request_text, name)

    if value is None:
        return None

    try:
        return int(value)
    except ValueError:
        return None


def add_go_state_fields(extra_fields):
    clear_stale_go_state()
    extra_fields["client_go_ticks_set"] = go_ticks_us is not None

    if current_round_id is not None:
        extra_fields["client_go_round_id"] = current_round_id


def clear_go_state(reason):
    global current_round_id, go_ticks_us, last_go_delay_us, last_go_received_ticks_us

    if DEBUG_WIFI and (current_round_id is not None or go_ticks_us is not None):
        print("Clearing GO state:", reason, "round", current_round_id)

    current_round_id = None
    go_ticks_us = None
    last_go_delay_us = None
    last_go_received_ticks_us = None


def clear_stale_go_state():
    if last_go_received_ticks_us is None:
        return

    if ticks_diff(ticks_us(), last_go_received_ticks_us) > GO_STATE_MAX_AGE_US:
        clear_go_state("stale")


def service_calibration_server():
    global current_round_id, go_ticks_us, last_go_delay_us, last_go_received_ticks_us

    if calibration_socket is None:
        return

    try:
        client, address = calibration_socket.accept()
    except OSError:
        return

    try:
        client.settimeout(0.05)

        received_ticks_us = ticks_us()

        try:
            request = client.recv(256)
        except OSError:
            request = b""

        try:
            request_text = request.decode()
        except UnicodeError:
            request_text = ""

        round_id = query_int(request_text, "round_id")
        delay_us = query_int(request_text, "delay_us")

        if round_id is not None:
            current_round_id = round_id
            last_go_delay_us = delay_us
            last_go_received_ticks_us = received_ticks_us
            go_ticks_us = (
                ticks_add(received_ticks_us, delay_us)
                if delay_us is not None
                else received_ticks_us
            )
        elif DEBUG_WIFI:
            print("GO request missing round_id:", request_text.split("\r\n", 1)[0])

        ok_text = "true" if round_id is not None else "false"
        response_status = (
            "HTTP/1.1 200 OK"
            if round_id is not None
            else "HTTP/1.1 400 Bad Request"
        )
        response_round_id = str(current_round_id) if current_round_id is not None else "null"
        go_ticks_set = "true" if go_ticks_us is not None else "false"
        body = (
            '{{"ok":{},"device_id":"{}","ack_ticks_us":{},"round_id":{},'
            '"go_ticks_set":{}}}'
        ).format(
            ok_text,
            hub.device_id,
            ticks_us(),
            response_round_id,
            go_ticks_set,
        )
        response = (
            "{}\r\n"
            "Content-Type: application/json\r\n"
            "Content-Length: {}\r\n"
            "Connection: close\r\n\r\n{}"
        ).format(response_status, len(body), body)
        response_bytes = response.encode()

        try:
            client.sendall(response_bytes)
        except AttributeError:
            client.send(response_bytes)

        if DEBUG_WIFI:
            print("Calibration ack sent to", address)
            if round_id is not None:
                print(
                    "GO stored round",
                    current_round_id,
                    "delay_us",
                    last_go_delay_us,
                    "received_ticks_us",
                    last_go_received_ticks_us,
                    "go_ticks_us",
                    go_ticks_us,
                )
    except OSError as exc:
        if DEBUG_WIFI:
            print("Calibration ack failed:", exc)
    finally:
        client.close()


# Tell the laptop server that this Pico is online.
def send_check_in():
    global last_checkin_rtt_us

    if not hub.connect_wifi():
        return False

    ensure_calibration_server()
    service_calibration_server()

    extra_fields = {}
    add_go_state_fields(extra_fields)

    if last_checkin_rtt_us is not None:
        extra_fields["client_checkin_rtt_us"] = last_checkin_rtt_us

    started_ticks_us = ticks_us()

    try:
        ok, status_text = hub.send_check_in(BUTTON_PIN, button_is_pressed(), extra_fields)
    except OSError as exc:
        print("Failed to send check-in:", exc)
        return False

    last_checkin_rtt_us = ticks_diff(ticks_us(), started_ticks_us)

    if DEBUG_WIFI:
        if ok:
            print("Check-in sent -", status_text)
        else:
            print("Check-in failed:", status_text)

    return ok


# Send a real buzz when the button is pressed.
def send_buzz(press_ticks_us):
    if not hub.connect_wifi():
        flash(3, 0.2, 0.1)
        return False

    ensure_calibration_server()
    service_calibration_server()

    extra_fields = {}
    add_go_state_fields(extra_fields)

    if current_round_id is not None:
        extra_fields["round_id"] = current_round_id

    if go_ticks_us is not None:
        reaction_us = ticks_diff(press_ticks_us, go_ticks_us)
        extra_fields["client_reaction_us"] = reaction_us
        extra_fields["client_timing_status"] = (
            "before_pico_go" if reaction_us < 0 else "ok"
        )
    else:
        extra_fields["client_timing_status"] = "missing_go"

    if DEBUG_WIFI:
        print(
            "Buzz timing round",
            current_round_id,
            "go_ticks_us",
            go_ticks_us,
            "press_ticks_us",
            press_ticks_us,
            "status",
            extra_fields.get("client_timing_status"),
            "reaction_us",
            extra_fields.get("client_reaction_us"),
        )

    try:
        ok, status_text = hub.send_buzz(BUTTON_PIN, button_is_pressed(), extra_fields)
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
    ensure_calibration_server()
    send_check_in()
    last_check_in_ms = ticks_ms()

# Keep watching the button forever.
while True:
    service_calibration_server()
    current_button_pressed = button_is_pressed()

    if current_button_pressed != last_button_pressed:
        edge_ticks_us = ticks_us()
        sleep(BUTTON_DEBOUNCE_MS / 1000)
        current_button_pressed = button_is_pressed()

        if current_button_pressed != last_button_pressed:
            last_button_pressed = current_button_pressed

            if current_button_pressed:
                if send_buzz(edge_ticks_us):
                    last_check_in_ms = ticks_ms()
            else:
                if send_check_in():
                    last_check_in_ms = ticks_ms()

    if ticks_diff(ticks_ms(), last_check_in_ms) >= HEARTBEAT_INTERVAL_MS:
        send_check_in()
        last_check_in_ms = ticks_ms()

    sleep(BUTTON_POLL_SECONDS)
