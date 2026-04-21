from machine import unique_id
from time import sleep, ticks_diff, ticks_ms
import network
import socket
import ubinascii

try:
    import ujson as json
except ImportError:
    import json


class BuzzerHubClient:
    def __init__(
        self,
        wifi_ssid,
        wifi_password,
        server_host,
        player_name=None,
        app_kind="buzzer",
        server_port=8787,
        server_path="/api/buzz",
        light_path="/api/light-reading",
        checkin_path="/api/check-in",
        debug_wifi=False,
        wifi_timeout_seconds=15,
        http_timeout_seconds=5,
    ):
        self.wifi_ssid = wifi_ssid
        self.wifi_password = wifi_password
        self.server_host = server_host
        self.player_name = player_name.strip() if isinstance(player_name, str) else None
        self.app_kind = app_kind
        self.server_port = server_port
        self.server_path = server_path
        self.light_path = light_path
        self.checkin_path = checkin_path
        self.debug_wifi = debug_wifi
        self.wifi_timeout_seconds = wifi_timeout_seconds
        self.http_timeout_seconds = http_timeout_seconds
        self.wlan = network.WLAN(network.STA_IF)
        self.device_id = ubinascii.hexlify(unique_id()).decode()

    def get_mac_address(self):
        try:
            return self._format_mac_address(self.wlan.config("mac"))
        except (AttributeError, OSError, TypeError, ValueError):
            return None

    def get_ip_address(self):
        if not self.wlan.isconnected():
            return None

        try:
            return self.wlan.ifconfig()[0]
        except (OSError, IndexError, TypeError):
            return None

    def make_payload(self, button_pin=None, button_pressed=None, extra_fields=None):
        payload = {
            "device_id": self.device_id,
            "app_kind": self.app_kind,
        }

        if self.player_name:
            payload["player_name"] = self.player_name

        if button_pin is not None:
            payload["button_pin"] = button_pin

        if button_pressed is not None:
            payload["button_pressed"] = button_pressed

        if extra_fields:
            payload.update(extra_fields)

        mac_address = self.get_mac_address()
        if mac_address:
            payload["mac_address"] = mac_address

        ip_address = self.get_ip_address()
        if ip_address:
            payload["ip_address"] = ip_address

        return payload

    def send_check_in(self, button_pin=None, button_pressed=None, extra_fields=None):
        payload = self.make_payload(button_pin, button_pressed, extra_fields)
        return self._post_json(self.checkin_path, payload)

    def send_buzz(self, button_pin, button_pressed, extra_fields=None):
        payload = self.make_payload(button_pin, button_pressed, extra_fields)
        return self._post_json(self.server_path, payload)

    def send_light_reading(self, light_pin, light_raw, light_percent, extra_fields=None):
        payload = self.make_payload(
            extra_fields={
                "light_pin": light_pin,
                "light_raw": light_raw,
                "light_percent": light_percent,
            }
        )

        if extra_fields:
            payload.update(extra_fields)

        return self._post_json(self.light_path, payload)

    def connect_wifi(self):
        if not self.wlan.active():
            self.wlan.active(True)

        if self.wlan.isconnected():
            if self.debug_wifi:
                print("Already connected:", self.wlan.ifconfig()[0])
            return True

        if self.debug_wifi:
            self.debug_scan()

        try:
            self.wlan.disconnect()
        except OSError:
            pass

        print('Connecting to Wi-Fi "{}"...'.format(self.wifi_ssid))

        try:
            self.wlan.connect(self.wifi_ssid, self.wifi_password)
        except OSError as exc:
            print("wlan.connect failed immediately:", exc)
            return False

        start = ticks_ms()
        last_status = None

        while not self.wlan.isconnected():
            status = self.wlan.status()

            if status != last_status and self.debug_wifi:
                print("Wi-Fi status:", self._wifi_status_name(status), "(", status, ")")
                last_status = status

            if status in (
                getattr(network, "STAT_WRONG_PASSWORD", -3),
                getattr(network, "STAT_NO_AP_FOUND", -2),
                getattr(network, "STAT_CONNECT_FAIL", -1),
            ):
                self._print_wifi_failure(status)
                return False

            if ticks_diff(ticks_ms(), start) > self.wifi_timeout_seconds * 1000:
                print("Wi-Fi connection timed out after", self.wifi_timeout_seconds, "seconds.")
                self._print_wifi_failure(self.wlan.status())

                if self.debug_wifi:
                    self.debug_scan()

                return False

            sleep(0.25)

        print("Connected:", self.wlan.ifconfig()[0])

        if self.debug_wifi:
            try:
                print("Signal strength (RSSI):", self.wlan.status("rssi"), "dBm")
            except OSError:
                pass

        return True

    def debug_scan(self):
        print('Scanning for Wi-Fi networks near "{}"...'.format(self.wifi_ssid))

        try:
            networks = self.wlan.scan()
        except OSError as exc:
            print("Wi-Fi scan failed:", exc)
            return False

        if not networks:
            print("No Wi-Fi networks were found.")
            return False

        target_found = False

        for ssid_raw, _bssid, channel, rssi, authmode, hidden in networks:
            ssid = self._decode_ssid(ssid_raw)

            if ssid == self.wifi_ssid:
                target_found = True
                print(
                    'Target SSID found: channel={}, RSSI={}, security={}, hidden={}'.format(
                        channel,
                        rssi,
                        self._authmode_name(authmode),
                        hidden,
                    )
                )

        if target_found:
            return True

        print('Target SSID "{}" was not seen in the scan.'.format(self.wifi_ssid))
        print("Nearby networks:")

        for index, (ssid_raw, _bssid, channel, rssi, authmode, hidden) in enumerate(networks[:8]):
            ssid = self._decode_ssid(ssid_raw) or "<hidden>"
            print(
                "  {}. {} (channel {}, RSSI {}, {}, hidden={})".format(
                    index + 1,
                    ssid,
                    channel,
                    rssi,
                    self._authmode_name(authmode),
                    hidden,
                )
            )

        return False

    def _post_json(self, path, payload):
        body = json.dumps(payload)
        address = socket.getaddrinfo(self.server_host, self.server_port)[0][-1]
        client = socket.socket()
        client.settimeout(self.http_timeout_seconds)

        try:
            client.connect(address)
            request = (
                "POST {} HTTP/1.1\r\n"
                "Host: {}\r\n"
                "Content-Type: application/json\r\n"
                "Content-Length: {}\r\n"
                "Connection: close\r\n\r\n{}"
            ).format(path, self.server_host, len(body), body)
            client.send(request.encode())
            response = client.recv(128)
        finally:
            client.close()

        status_line = response.split(b"\r\n", 1)[0]

        try:
            status_text = status_line.decode()
        except UnicodeError:
            status_text = repr(status_line)

        parts = status_text.split(" ", 2)

        if len(parts) >= 2:
            try:
                status_code = int(parts[1])
            except ValueError:
                status_code = None
        else:
            status_code = None

        ok = status_code is not None and 200 <= status_code < 300
        return ok, status_text

    def _format_mac_address(self, value):
        if not value:
            return None

        raw = ubinascii.hexlify(value).decode()
        parts = []

        for index in range(0, len(raw), 2):
            parts.append(raw[index : index + 2])

        return ":".join(parts)

    def _decode_ssid(self, value):
        if isinstance(value, bytes):
            try:
                return value.decode()
            except UnicodeError:
                return repr(value)

        return str(value)

    def _authmode_name(self, value):
        names = {
            0: "open",
            1: "WEP",
            2: "WPA-PSK",
            3: "WPA2-PSK",
            4: "WPA/WPA2-PSK",
        }
        return names.get(value, "unknown")

    def _wifi_status_name(self, value):
        names = {
            getattr(network, "STAT_IDLE", 0): "idle",
            getattr(network, "STAT_CONNECTING", 1): "connecting",
            getattr(network, "STAT_WRONG_PASSWORD", -3): "wrong password",
            getattr(network, "STAT_NO_AP_FOUND", -2): "no AP found",
            getattr(network, "STAT_CONNECT_FAIL", -1): "connect failed",
            getattr(network, "STAT_GOT_IP", 3): "connected",
        }
        return names.get(value, "unknown")

    def _print_wifi_failure(self, status):
        print("Wi-Fi status:", self._wifi_status_name(status), "(", status, ")")

        if status == getattr(network, "STAT_WRONG_PASSWORD", -3):
            print("The access point rejected the password.")
        elif status == getattr(network, "STAT_NO_AP_FOUND", -2):
            print("The Pico could not find that SSID.")
            print("Check spelling, range, and that the network is 2.4 GHz.")
        elif status == getattr(network, "STAT_CONNECT_FAIL", -1):
            print("The router saw the request, but the connection still failed.")
            print("This can happen with unsupported security settings or weak signal.")
