"""A fake ``CODEX_HOME``.

Codex reads all of its settings from one folder and lets you choose which one.
Point it at a folder holding nothing but a link to the real credentials and a
config file omni wrote, and there is nothing left to auto-load: no MCP servers,
no hooks, no ``AGENTS.md``.

Login still works because the one file that holds it is linked through. Skills
are the exception — they live in a shared folder outside ``CODEX_HOME``, so the
folder trick misses them and they get switched off over the protocol instead.
"""

from pathlib import Path

from ...shared import paths

MINIMAL = "# written by silicon omni — deliberately almost empty\n"


def real_home() -> Path:
    return Path.home() / ".codex"


def build(session_id: str) -> Path:
    """Make (or refresh) this session's codex home and return it.

    Always near-empty: the jail *is* how omni isolates codex, so MCP servers,
    hooks and ``AGENTS.md`` never load, whether or not ``disable_mcp`` was asked
    for.
    """
    home = paths.ensure(paths.jail(session_id, "codex"))
    link, real = home / "auth.json", real_home() / "auth.json"
    if link.is_symlink() or link.exists():
        link.unlink()  # a link left from a previous run may point nowhere now
    if real.exists():
        link.symlink_to(real)  # linked, not copied, so a token refresh is not lost
    (home / "config.toml").write_text(MINIMAL)
    return home
