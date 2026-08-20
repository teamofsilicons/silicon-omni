"""Keep a chat alive with a plain loop — the simplest thing that works."""

import time

from omni import Event, Inference

chat = Inference.load_or_create_session("loop-example")
chat.intelligence(5)

last = None
queue = ["what is 2+2?", "and times ten?"]


def fetch_new_messages():
    return queue.pop(0) if queue else None


def should_stop():
    return not queue


@chat.on_event
def handle(event):
    global last
    last = event.type
    if event.type == Event.TEXT:
        print(event.text)


chat.start()

while chat.status in ("busy", "waiting"):
    message = fetch_new_messages()
    if message:
        chat.send(message)
    else:
        time.sleep(0.2)
    if should_stop() and last == Event.END:
        chat.stop()
