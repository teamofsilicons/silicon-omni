"""Feed a chat from a subscription.

``chat.send`` never blocks and is safe from any thread, so omni does not care
whether you drive it from asyncio, a queue, or a webhook.
"""

import asyncio

import nats  # pip install nats-py

from omni import Event, Inference

chat = Inference.load_or_create_session("subscribe-example")
chat.intelligence(7)
chat.disable_subagents()

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
        chat.send(msg.data.decode())  # opens a turn, or lands mid-flight
        await msg.ack()

    await nc.subscribe("agent.msgs", cb=on_msg)
    await stop.wait()
    await nc.drain()
    chat.stop()


asyncio.run(main())
