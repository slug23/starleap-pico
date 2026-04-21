from machine import Pin
from time import sleep

lights = {
    1: {
        'status': 'RED', 
        'green': Pin(0, Pin.OUT),
        'yellow': Pin(1, Pin.OUT), 
        'red': Pin(2, Pin.OUT)
        },
    2: {
        'status': 'RED', 
        'green': Pin(3, Pin.OUT), 
        'yellow': Pin(4, Pin.OUT), 
        'red': Pin(5, Pin.OUT)
        }
}

SLEEP_TIME = {
    'GREEN': 5,
    'YELLOW': 2,
    'RED': 1
}


def set_light(light_num, color):
    light = lights[light_num]
    if color == 'GREEN':
        light['green'].on()
        light['yellow'].off()
        light['red'].off()
        light['status'] = 'GREEN'
        print(f"Light {light_num}: Green means Go!")
    elif color == 'YELLOW':
        light['green'].off()
        light['yellow'].on()
        light['red'].off()
        light['status'] = 'YELLOW'
        print(f"Light {light_num}: Yellow means Slow Down!")
    elif color == 'RED':
        light['green'].off()
        light['yellow'].off()
        light['red'].on()
        light['status'] = 'RED'
        print(f"Light {light_num}: Red means Stop!")
    sleep(SLEEP_TIME[color])

set_light(1, 'RED')
set_light(2, 'RED')
sleep(1)

while True:
    set_light(1, 'GREEN')
    set_light(1, 'YELLOW')
    set_light(1, 'RED')
    set_light(2, 'GREEN')
    set_light(2, 'YELLOW')
    set_light(2, 'RED')