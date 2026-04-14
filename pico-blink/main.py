from machine import Pin
from time import sleep

led = Pin(15, Pin.OUT)
button = Pin(14, Pin.IN, Pin.PULL_UP)

DOT = 0.15
DASH = DOT * 3
SYMBOL_GAP = DOT
LETTER_GAP = DOT * 3


def blink(duration):
    led.on()
    sleep(duration)
    led.off()


def blink_letter(pattern):
    for index, symbol in enumerate(pattern):
        blink(DOT if symbol == "." else DASH)

        if index < len(pattern) - 1:
            sleep(SYMBOL_GAP)

    sleep(LETTER_GAP)


def blink_sos():
    blink_letter("...")
    blink_letter("---")
    blink_letter("...")


print("Press the button on GP14 to blink SOS on the LED on GP15.")

while True:
    if button.value() == 0:
        sleep(0.02)

        if button.value() == 0:
            blink_sos()

            while button.value() == 0:
                sleep(0.01)
