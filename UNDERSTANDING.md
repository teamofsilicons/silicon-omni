We're making silicon omni. this is a unified python package to communicate with claude code, codex or gemini models powered via subscriptions.

we'll use `claude -p` for claude code with json streaming
we'll use `chatgpt app server` for connecting with openai models.
we'll use `agy` for google's antigravity.

at the end, this will be exposed as a simple python interface to access any of these inference providers.

```python
from omni import Inference, Event

PROVIDERS = Inference.get_available_providers() # list["claude", "google", "openai"], can be defined manually to limit providers available

chat = Inference.load_or_create_session("session_id")
chat.active_inference_providers(PROVIDERS)
chat.inteligence(7) # 0-10 fetched from omni.teamofsilicons.com for the given set of providers. combines model and effort.
chat.system_prompt("...") or chat.system_prompt_file("/../../abc.txt") # either one

chat.disable_subagents()
chat.disable_mcp()

last_event = None

def fetch_new_messages():
    pass # reads from a local store where a global fetch instance keeps on finding new messages and queuing.

@chat.on_event
def handle_event(event):
    event_type == Event.THINKING # THINKING, TOOL.CALL, TOOL.RESULT, END, INJECTED, TEXT, AUTH_ERROR, LIMIT_REACHED
    last_event = event_type
    pass # do whatever you want for events

def check_stop_flag():
    return True or False


chat.start()

# how to send a msg for inference. could be when the run is complete, or the inference is ongoing
# if the inference is ongoing, then it should be injected immedietely after the ongoing turn (tool call, or something else) finishes.
chat.send(...)

# if they want to do it syncronously with while. or use async or pub/sub. implementation upto the user. chat.send is all that omni aspects.
while chat.running:
    # chat.running.status could be 'busy' or 'waiting'. busy is something is being running actively. and waiting is running & waiting for new msgs.
    new_msg = fetch_new_messages()
    if new_msg:
        chat.send(new_msg)
    else:
        time.sleep()

    if check_stop_flag() and last_event == Event.END:
        chat.stop()


# async msg implementation for the user:
stop_event = asyncio.Event()

async def main():
    nc = await nats.connect("nats://localhost:4222")
    chat.start()

    async def on_msg(m):
        chat.send(m.data.decode())     # opens a turn or lands mid-flight
        await m.ack()

    await nc.subscribe("agent.msgs", cb=on_msg)
    await stop_event.wait()            # keepalive
    await nc.drain()

asyncio.run(main())
```

weather the user of silicon omni wants to keep the system alive using while loop, or async, or pub sub is their choice.
the .send method should be able to send a msg to a new chat or inject in in between an already running turn.


features needed:
1. persistent sessions loadable via a session id. (sessions are cross provider and written on disk at ~/.omni/sessions/ as {session_id}.txt)
2. event hook on decorators @chat.on_event
3. omni is a translation layer so we can handle cross provider switching any time.

Disable all subagents and workflows using `chat.disable_subagents()`:
`CLAUDE_CODE_DISABLE_WORKFLOWS=1 claude "use subagents and find out about the files in this dir" --disallowedTools "Agent(*)"`
`codex "use subagents and find out about the files in this dir" -c "agents.enabled=false"`
```agy cli doesn't support turning off subagents.```
^ these commands when run don't use subagents. good. we do this so that any workers defined explicitely is used instead of their subagents.

we do a similar thing, by allowing to disable all connected mcp servers & external connectors.



we want to cross-provider switching.
because we rely only on intelligence scale based on all providers available, changing intelligence can result in changing the model provider we're using.

for this, first its imp to understand how each provider streams and stores data inside sessions. then we maintain the same chat (ongoing in provider A) also in all other providers on trigger to use it with another provider's models.

eg: say a chat starts off with gemini 3.7 on high flash on antigravity. then mid-way, its decided to increase the intelligence and it now uses claude opus 5 on medium. the chat should continue exactly. this will require us to translate the agy session into a claude code session. while it is true that it is often lossy, we will try to patch it as much as possible. a tool call that can not be translated exactly, can be passed as text. in a way that when (if) translating back to gemini, that should be exactly the same. so, i'll not say lossy, but rather that its preserved. maybe something like [GoogleSearch: "..."] will only be in gemini.

store all session details inside ~/.omni/sessions/ which is incapsulating all common as well as unique tools that a provider can call. this is the source of truth.
we will only load the sections that can be loaded into a provider. eg, GoogleSearch can be loaded as text. but reasoning can't be because its encrypted.


Auth
There should be a auth status for all cli's installed. And a way to login into it without doing so via the cli itself. they usually give a link to open, then a link they ask for, or a callback. It should be automated.

```python
from omni import Inference

Inference.claude.auth_status # authenticated or unauthenticated
print(Inference.claude.start_auth())
# > print the url to login from
Inference.claude.finish_auth("pass in code or redirect url")
# > returns authenticated or unauthenticated
# this can be done for any provider.
```


Session Limit
```python
from omni import Inference

Inference.claude.limits
# > {"5h": {"used": 0.24, "reset": timestamp}, "7d": {"used": 0.88, "reset": timestamp}}
# or it could return "unauthenticated"
```
0.24 -> 24%


Logging
```python

@chat.logs
def log():
    # queues, or writes to file
    pass

```
sends everything over. start, end, errors, even events (they are sent to both .on_event and .logs), every model change, every new sesison, msg in, tool calls, everything.
these logs should be programmically parsable.


Providers & Intelligence
providers does 2 things. check if the cli is installed, and then checks which ones are active (authenticated).

then intelligence is a scale from 0 to 10. each number is mapped to a model + effort and will be hosted on omni.teamofsilicons.com but for now, just keep a local json file. {0: {"provider": "google", "model": "gemini-3.7-flash", "effort": "high"}, ...} like this. at this endpoint, you can give it the providers you have, and it will give you 0-10 intelligence ranking. call this when switching providers or burst the cache after 60mins.

the string you give for model and effort should not be maintained as a local dict. it should be directly pluggable into the model switcher. this is done so that when a new model is launched, the slug can be changed on the remote, and it will be implemeted upstream. programatic changes to model name or effort is ok but it should require no upkeep when new models drop. follow the same pattern.

intelligence scale will only include models from providers that you have access to.





# codebase
structure it in modules. each part here becomes a module. nesting module is possible into submodules.
dont write code that starts with _func
keep the code to a minimum. if it can be done in less, lets do it in less.
we are following a webhook style of code.
this project will be open sourced, so make sure it can receive contributors. write good documentation and structure the code for understandability.
this will be published as a python package.