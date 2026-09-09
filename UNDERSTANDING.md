We're making silicon omni. this is a python, rust, cli client connecting over a rust binary deamon to communicate with claude code, codex or gemini models powered via subscriptions.

we'll use `claude -p` for claude code with json streaming
we'll use `chatgpt app server` for connecting with openai models.
we'll use `agy` for google's antigravity.

at the end, this will be exposed as a simple interface to access any of these inference providers.




ARCHITECTURE:
A rust based deamon is setup and runs on the system keeping claude, codex and agy hot for fast responses. Everything happens on this layer.
Keeping the inference providers hot is imp because it makes omni significantly faster.
A client side is established so that other interfaces can connect to it.
Events and Logs are streamed over Unix Sockets.
then we'll have a python package, a rust package, and a cli each running by connecting to the rust based deamon running.
even the rust client connects via the client to the deamon. its to be treated the same as python and cli.





```python
from omni import Inference, Event

PROVIDERS = Inference.get_available_providers() # list["claude-code-cli", "antigravity-cli", "codex-app-server"], can be defined manually to limit providers available

chat = Inference.load_or_create_session("session_id")
chat.active_inference_providers(PROVIDERS)
chat.model(cost=7, speed=10, intelligence=5, benchmark=" GDPval-AA v2") # best model fetched from omni.teamofsilicons.com for the given set of providers. combines model and effort.
# replaces the session prompt given by the provider. to append, use .append_system_prompt, or .append_system_prompt_file
chat.system_prompt("...") or chat.system_prompt_file("/../../abc.txt") # either one

chat.disable_subagents()
chat.disable_mcp()

last_event = None

def fetch_new_messages():
    pass # reads from a local store where a global fetch instance keeps on finding new messages and queuing.

@chat.on_event
def handle_event(event):
    event.type == Event.THINKING # START, THINKING, TOOL.CALL, TOOL.RESULT, END, INJECTED, TEXT, ERROR (auth, limit error, unavailable, etc etc. use using event.kind), SWITCH_PROVIDER, NEW_SESSION, some of these could be type of configs EVENT.CONFIG. and with seq. numbers to keep a track of until which point was a provider synced or needs syncing.
    last_event = event.type
    pass # do whatever you want for events

def check_stop_flag():
    return True or False


chat.start()

# how to send a msg for inference. could be when the run is complete, or the inference is ongoing
# if the inference is ongoing, then it should be injected immedietely after the ongoing turn (tool call, or something else) finishes.
chat.send(...)

# if they want to do it syncronously with while. or use async or pub/sub. implementation upto the user. chat.send is all that omni aspects.
while chat.status in ["busy", "waiting"]:
    # chat.status could be 'busy' or 'waiting'. busy is something is being running actively. and waiting is running & waiting for new msgs.
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
1. persistent sessions loadable via a session id. (sessions are cross provider and written on disk at ~/.omni/sessions/ as {session_id}.jsonl) along with a meta json file to store any session information needed.
2. event hook on decorators @chat.on_event
3. omni is a translation layer so we can handle cross provider switching any time.

Disable all subagents and workflows using `chat.disable_subagents()`:
`CLAUDE_CODE_DISABLE_WORKFLOWS=1 claude "use subagents and find out about the files in this dir" --disallowedTools "Agent(*)"`
`codex app-server --stdio --disable apps --disable plugins -c agents.enabled=false -c project_doc_max_bytes=0`
`agy cli doesn't support turning off subagents.`
^ these commands when run don't use subagents. good. we do this so that any workers defined explicitely is used instead of their subagents.

we do a similar thing, by allowing to disable all connected mcp servers & external connectors.



we want to cross-provider switching.
because we rely only on preference scale based on all providers available, changing preference can result in changing the model provider we're using.

for this, first its imp to understand how each provider streams and stores data inside sessions. then we maintain the same chat (ongoing in provider A) also in all other providers on trigger to use it with another provider's models.

eg: say a chat starts off with gemini 3.7 on high flash on antigravity. then mid-way, its decided to increase the intelligence and it now uses claude opus 5 on medium. the chat should continue exactly. this will require us to translate the agy session into a claude code session. while it is true that it is often lossy, we will try to patch it as much as possible. a tool call that can not be translated exactly, can be passed as text. in a way that when (if) translating back to gemini, that should be exactly the same. so, i'll not say lossy, but rather that its preserved. maybe something like [GoogleSearch: "..."] will only be in gemini.

store all session details inside ~/.omni/sessions/ as {session_id}.jsonl which is incapsulating all common as well as unique tools that a provider can call. this is the source of truth.
we will only load the sections that can be loaded into a provider. eg, GoogleSearch can be loaded as text. but reasoning can't be because its encrypted.


Auth
There should be a auth status for all cli's installed. And a way to login into it without doing so via the cli itself. they usually give a link to open, then a link they ask for, or a callback. It should be automated. If it can't be done for some reason, surface the problem to the user. or ask them to login directly themselves. No multi-account support yet.

if mid-run a provider turns unauthenticated, and the turn fails and gets captured via Event.ERROR and kind AUTH, then it should remove that provider from the provider's list, report the auth error, and then automcatically switch to the same inteligence level among the remaining providers.

this behaviour is turned on by default and can be stopped using `chat.disable_autoremoving_unauthenticated_providers()`, in which case it will report that its unauthenticated, and end the turn.

```python
from omni import Inference

Inference.claude_code_cli.auth_status # authenticated or unauthenticated
print(Inference.claude_code_cli.start_auth())
# > print the url to login from
Inference.claude_code_cli.finish_auth("pass in code or redirect url")
# > returns authenticated or unauthenticated
# this can be done for any provider.
```


CWD:
while the files for storing omni details are stored inside ~/.omni/, the sessions that the providers will run will be from the dir that the command was run from.
eg. if i run `cd ~/Downloads/ && python ~/Documents/abc.py`, and say a claude session is created, then if i ask claude which dir is it in... it should be ~/Downloads/

when its started and omni session meta file has written about this dir as the starting point for this session. all providers triggered within a given omni session will be tied to this dir.

if the cwd is changed mid omni-session, then we can simply port the current running provider session into this new dir and run the command.




Session Limit
```python
from omni import Inference

Inference.claude_code_cli.limits
# > {"5h": {"used": 0.24, "reset": ISO string}, "7d": {"used": 0.88, "reset": ISO string}}
# ISO string or None incase no session limit reset time. Used can also be None.
# make sure all reset time is in ISO strings. for all providers.
# or it could return "unauthenticated"
```
0.24 -> 24%
the 5h and 7d session is the standard way that claude, codex and antigravity show session limits and time remaining. this is a standard practice among them all.


Logging
```python

@chat.logs
def log():
    # queues, or writes to file
    pass

```
sends everything over. start, end, errors, even events (they are sent to both .on_event and .logs), every model change, every new sesison, msg in, tool calls, everything.
these logs should be programmically parsable.

Store things inside a .jsonl file and keep it the json. this is the source of truth when switching providers.


Providers & Intelligence
providers does 2 things. check if the cli is installed, and then checks which ones are active (authenticated).

chat.model(...) can take 3 kinds of things. either 0-10 intelligence score we have, or straight up model names and effort (model="gemini-3.8-flash", effort="low", fast=false}), or special keyword

oh, and codex and claude support a /fast for their models. false by default. but can be passed as true. make sure there is a way to do that. research on how its done. google does not have fast, so its ignored.

key based model selection:
best one per provider we have, ranked from left to right.
"fast": [gpt-6-astra-low-fast, gemini-3.8-flash-low, claude-opus-5-low-fast]
"code": [gpt-6-astra-high, claude-opus-5-max, gemini-3.8-flash-high]
"design": [claude-opus-5-max, gpt-6-astra-high, gemini-3.8-flash-high]
"research": [gpt-6-astra-high, claude-opus-5-max, gemini-3.8-flash-high]
"cost": [gpt-6-astra-low, gemini-3.8-flash-low, claude-opus-5-low]
"general": [gpt-6-astra-high, claude-opus-5-medium, gemini-3.8-flash-high]

the dict you give for model and effort should not be maintained as a local dict. it should be directly pluggable into the model switcher. this is done so that when a new model is launched, the slug can be changed on the remote, and it will be implemeted everywhere. programatic changes to model name or effort is ok but it should require no upkeep when new models drop. follow the same pattern that proviers use right now.

intelligence scale will only include models from providers that you have access to.

make sure to map a omni session to a session on claude, codex and/or antigravity. when switching, a new session will be seeded with history from omni, but when continuing, the existing session should be used.

eg: omni session A on claude session B. now switch to google, with session C, chat a little and come back to claude, but now it will first seed the session B to come to the same place as session C. more chatting will continue on session B. All the history is being written to omni session A, and then getting seeded to providers.

Do not store reasoning tokens/encrypted tokens inside omni. just enter a reasoning block that is empty for logging purpose. it will not be used to seed another chat. besides reasoning, log everything else.

DO NOT LIMIT how long in the history is loaded. Load the complete context when switching.





GENERAl:
NOTHING CHANGES MID TURN. ALL CHANGES HAPPEN AFTER THE CURRENT TOOL IS DONE (turn completed).
all chat will be --dangerously-skip-permissions or equivalent.
Preserve, never lose.
Omni owns the history but is read from only on switch. Use the native continue/resume when using not switching providers. model-switch is easily possible even when using a provider.
Omni only observes the tools. Dont sit and define new ones to the providers to use.

enable_subagents() / enable_mcp() to turn subagent and mcp back on.

running any of the following commands anytime again will overwrite them. this is how intelligence is changed. this is how a new session is created. this is how inference providers are changed. these things can happen after the current running tool/task/turn is completed.
```python
chat = Inference.load_or_create_session("session_id")
chat.active_inference_providers(PROVIDERS)
chat.inteligence(7) # 0-10 fetched from omni.teamofsilicons.com for the given set of providers. combines model and effort.
chat.system_prompt("...") or chat.system_prompt_file("/../../abc.txt") # either one
```

every session allows multiple connects for both reading events & logs, and write/send.

a script can be run on any system, it installs & uses omni. it also sets up the omni cli and puts it to path.




CODEX:
we use app server. not exec.

use a fake home folder, then turn off skills over the protocol.

CODEX_HOME=~/.omni/jails/<session>/codex \
codex app-server --stdio --disable apps --disable plugins \
  -c agents.enabled=false -c project_doc_max_bytes=0
Put only two things in that folder: a symlink to the real auth.json, and a small config.toml you wrote. Then after connecting, call skills/list and skills/config/write {name, enabled:false} for each skill.

Why it works: codex reads all its settings from one folder, and lets you choose the folder. Point it at an empty one and it has nothing to load — no MCP servers, no hooks, no AGENTS.md. Login still works because you symlinked the one file that holds it. Skills are the exception: they live in a different shared folder outside that one, so the folder trick misses them and you have to switch them off one by one. That's a normal request, it doesn't error, and it saves into your folder so you only do it once.

codex app-server supports seeding of conversation when switching to codex from any other provider.

by default, use the jailed codex. and also disable any preloaded memories. subagents are opt in.
 enable_mcp() wont work for codex because its always jailed.



Antigravity:
increase the timeout to more than 5mins. it should never timeout.

agy -p --output-format stream-json --input-format stream-json \
  --disable-slash-commands --print-timeout 24h --dangerously-skip-permissions

agy has no settings for disabling this. It has no flag for MCP, no flag for subagents, and no way to remove a tool. The fake-home trick that works for codex fails here, because agy's login is tied to the real home folder. Its ok. let antigravity load whatever it wants.

there is no native way to seed, so we flatten a msg into one user msg and then continue. make sure this one msg is enough to seed back natively into other providers. this will cost one turn, but will seed the model with what it needs.

since some of the things are not supported in antigravity. lets emit an announce for a certain config being unsupported. and do it only once when setting the config or switching to agy from another provider.



Claude:
use flags. Nothing else needed.

CLAUDE_CODE_DISABLE_WORKFLOWS=1 CLAUDE_CODE_DISABLE_AUTO_MEMORY=1 \
CLAUDE_CODE_DISABLE_CLAUDE_MDS=1 CLAUDE_CODE_DISABLE_ORG_MEMORY=1 \
claude -p --output-format stream-json --input-format stream-json --verbose \
  --strict-mcp-config --setting-sources "" --disable-slash-commands \
  --disallowedTools "Agent(*)" --dangerously-skip-permissions

stops all subagents as well as mcps.

to seed claude with existing conversation, write to the claude's session file.

by default, disable mcp. and also disable any preloaded memories. subagents are opt in.



TEST:
Implement a simple test model provider to run automatic and deterministic tests.
Use live tests with real providers to make sure its real world resilient.
when running live tests, keep it in the same OMNI_HOME as the real usage. then run a cleanup script after the tests are done. this will ensure that all things are as they would be during a live usage.
use  ~/.omni/cwd/ so that its easy to cleanup after testing. seed

Omni Web:
`omni web` runs a webserver on localhost:1998 (first preference, or it moves down (1997, 1996...) and finds the next available port)

`omni web connect` then it shows a 8 digit connect key "RNDM-PORT" (XULA-1998)
this is a single provider code, once connected via this code, its used up and exchanged with omniauth.

this will give all the information needed to connect to the omni-web.

once a 3rd party has connected via this token, then it sends an omniauth which is short lived while this bridge is on the same port. if the bridge dies & starts on a new port, all previous auth is cleared and must be reconnected. the bridge tries to restart on the last used port saved inside .omni. if it cant then it restarts the back counting from 1998.


# codebase
structure it in modules. each part here becomes a module. nesting module is possible into submodules. define modules based on how i've seperated ideas here.
dont write code that starts with _func. abstract only when it will be used atleast thrice.
create a shared dir for shared code.
keep the code to a minimum. if it can be done in less, lets do it in less.
we are following a event/callback driven code style.
this project will be open sourced, so make sure it can receive contributors. write good documentation and structure the code for understandability.
omni will be published as a python package, rust package, and a cli.
follow a sync approach when its for simple tasks, event/callback driven > async for complex. async otherwise.
write test cases, mention what you're testing a test-group, and then at the end, give results.
all tools you need are installed natively and feel free to install any package.


# codebase thinking
- writing code is not just about implementation, maintainability & elegance matter as much.
- test and try things before you implement. try a simpler version to see how it works, what works what doesn't work. think in extremes.
- smaller code is reliable code. write less.
- writing once is not enough. its v0.0, iterate. make it smaller, faster, reliable, resilient, elegant, & largely maintainable.
- use pre installed libraries before you need to reach out for external onces. feel free to use them when you want.
- codebase is a form of art.
- use workflows well... not just for writing code, but thinking, evaluating, testing, researching, organizing, and critiquing yourself.
- run agents to get critiques on what you have done. what you have thought.
- don't implement more than this UNDERSTANDING.md asks you until its truely needed.