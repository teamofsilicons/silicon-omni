"""Feed a chat from a subscription.

``chat.send`` waits for durable daemon acceptance, not for model output. It is
safe from any thread; async callers should move that short socket round trip to
a worker so omni does not care whether messages came from a queue or webhook.
"""

import asyncio

import nats  # pip install nats-py

from omni import Event, Inference

chat = Inference.load_or_create_session("subscribe-example")
chat.intelligence(7)

stop = asyncio.Event()


@chat.on_event
def handle(event):
    if event.type == Event.TEXT:
        print(event.text)
    if event.type == Event.ERROR and event.kind == "limit":
        chat.intelligence(2)  # applied at the next turn boundary


async def main():
    nc = await nats.connect("nats://localhost:4222")
    chat.start()

    async def on_msg(msg):
        # Acceptance crosses the daemon socket, so do not block this event loop.
        await asyncio.to_thread(chat.send, msg.data.decode())
        await msg.ack()

    await nc.subscribe("agent.msgs", cb=on_msg)
    await stop.wait()
    await nc.drain()
    chat.stop()


asyncio.run(main())
