"""Take the live tests' leavings back out of the real ~/.omni.

Live tests run against the same OMNI_HOME as real usage, so that they exercise
the same paths, caches and jails a user's run does. The cost is that they leave
sessions behind. This puts the home back::

    pytest -m live && python3 scripts/cleanup.py

Only sessions whose id starts with the prefix are touched, and only inside
OMNI_HOME. Everything removed is printed, so a surprise is visible rather than
silent. Pass ``--dry-run`` to see the list without deleting anything.
"""

import argparse
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from omni.providers.test import NAME as TEST_PROVIDER  # noqa: E402
from omni.shared import paths  # noqa: E402

PREFIX = "live-"

#: Everywhere under the home that gets named after a session.
FOLDERS = ("sessions", "jails", "cwd")


def leavings(prefix: str) -> list[Path]:
    """Every file and folder under the home named after a matching session.

    A plain name comparison, deliberately not a glob. ``--prefix '*'`` as a
    pattern means the folder itself, and this script deletes what it is given.
    """
    found = []
    for folder in FOLDERS:
        here = paths.home() / folder
        if here.is_dir():
            found += [path for path in here.iterdir() if path.name.startswith(prefix)]
    return sorted(found)


def dial_entry() -> tuple[Path, dict] | None:
    """The pinned test-provider dial, if ``omni.providers.test`` wrote one."""
    cache = paths.cache() / "intelligence.json"
    try:
        blob = json.loads(cache.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return (cache, blob) if TEST_PROVIDER in blob else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--prefix", default=PREFIX, help=f"session ids to remove (default: {PREFIX!r})")
    parser.add_argument("--dry-run", action="store_true", help="list what would go, remove nothing")
    args = parser.parse_args()

    if not args.prefix:
        parser.error("an empty prefix would match every session; refusing")
    if set(args.prefix) & set("/\\") or ".." in args.prefix:
        parser.error("a prefix is a session id, not a path; refusing")

    home = paths.home().resolve()
    print(f"omni home: {home}")
    targets = leavings(args.prefix)
    for path in targets:
        # Belt and braces. Resolved, because an unresolved ``a/../../b`` still
        # looks like it lives under ``a``.
        settled = path.resolve()
        if home != settled and home not in settled.parents:
            print(f"  skipped (outside the home): {path}")
            continue
        print(f"  {'would remove' if args.dry_run else 'removing'} {settled.relative_to(home)}")
        if args.dry_run:
            continue
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()  # rmtree refuses a symlink; unlink drops the link, not its target

    pinned = dial_entry()
    if pinned:
        cache, blob = pinned
        print(f"  {'would drop' if args.dry_run else 'dropping'} the {TEST_PROVIDER!r} dial from the cache")
        if not args.dry_run:
            blob.pop(TEST_PROVIDER)
            cache.write_text(json.dumps(blob))

    if not targets and not pinned:
        print("  nothing to clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
