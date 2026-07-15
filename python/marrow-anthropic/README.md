# marrow-anthropic

A backend for [Anthropic's memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
(`memory_20250818`). It implements the six memory commands — `view`, `create`,
`str_replace`, `insert`, `delete`, `rename` — over a directory you control, with the
return strings the model expects and strict path-traversal protection.

## Install

From a Marrow checkout:

```bash
pip install ./python/marrow-anthropic
pip install "./python/marrow-anthropic[sdk]"
```

Tagged releases also include a wheel and source archive on the GitHub release page.

## Use it as a memory-tool backend

```python
from anthropic import Anthropic
from marrow_anthropic import MarrowMemoryBackend

client = Anthropic()
memory = MarrowMemoryBackend("./.marrow/memories")

client.beta.messages.run_tools(
    model="claude-opus-4-8",
    messages=[{"role": "user", "content": "Remember that I prefer dark mode."}],
    tools=[memory],
).until_done()
```

`MarrowMemoryBackend` subclasses `anthropic.tools.BetaAbstractMemoryTool`, so it slots in
wherever the tool is expected.

## Use the store directly

The core `MemoryStore` has no dependencies and returns the same strings the tool produces:

```python
from marrow_anthropic import MemoryStore

store = MemoryStore("./.marrow/memories")
store.create("/memories/notes.txt", "Project kickoff notes\n")
print(store.view("/memories/notes.txt"))
```

## Safety

Every path is resolved and confined to the base directory. Attempts to escape `/memories`
(`../`, absolute paths elsewhere) are rejected rather than followed.

## License

Apache-2.0 (see [LICENSE](LICENSE)). This backend is meant to be embedded in your own
application, so it carries a permissive license. The rest of Marrow (the engine under
`crates/`) is AGPL-3.0-only; see [COMMERCIAL.md](../../COMMERCIAL.md).
