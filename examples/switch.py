"""Move one conversation between providers and watch it carry on."""

import time

from omni import Event, Inference

providers = Inference.get_available_providers()
print("available:", providers)

chat = Inference.load_or_create_session("switch-example", providers)


@chat.on_event
def handle(event):
    if event.type == Event.SWITCH_PROVIDER:
        print(f"--- moved {event.extra['from']} -> {event.extra['to']} ---")
    if event.type == Event.TEXT:
        print(f"[{event.provider}] {event.text}")


def settle():
    while not chat.idle:
        time.sleep(0.1)


chat.intelligence(0)
chat.start()
settle()

chat.send("Remember: the passphrase is VIOLET-7. Reply with exactly: STORED")
settle()

chat.intelligence(10)  # may well be a different vendor
chat.send("What is the passphrase? Answer with just the passphrase.")
settle()

chat.stop()
