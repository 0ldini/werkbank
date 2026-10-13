"""Gibt pro Programm (und für den Launcher) das neueste Release-Tag aus: «<id> <tag>» je Zeile.

Aufruf: python3 setup/collect.py <owner/repo>   (braucht gh mit Anmeldung)
"""
import json, pathlib, subprocess, sys, tomllib

repo = sys.argv[1]
tags = json.loads(subprocess.check_output(["gh", "release", "list", "--repo", repo, "--limit", "300", "--json", "tagName"]))
tags = [t["tagName"] for t in tags]


def version(tag, prefix):
    rest = tag[len(prefix):]
    if "-" in rest:  # Vorabversion
        return None
    try:
        return tuple(int(x) for x in rest.split("."))
    except ValueError:
        return None


def latest(prefix):
    found = [(version(t, prefix), t) for t in tags if t.startswith(prefix)]
    found = [f for f in found if f[0] is not None]
    return max(found)[1] if found else None


apps = tomllib.loads(pathlib.Path("launcher/apps.toml").read_text(encoding="utf-8"))["app"]
l = latest("werkbank-v")
if not l:
    sys.exit("kein Launcher-Release")
print("werkbank", l)
for a in apps:
    t = latest(a.get("tag", "v"))
    if t:
        print(a["id"], t)
    else:
        print(f"::warning::{a['id']}: kein Release", file=sys.stderr)
