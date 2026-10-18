"""Stellt den Inhalt für den Server (Ordner «software» auf jarns.ch) aus den GitHub-Releases zusammen.

Aufruf: python3 setup/server_site.py <owner/repo> <arbeitsordner> [force]
Braucht gh mit Anmeldung (GH_TOKEN). Umgebungsvariable SERVER_URL: Adresse des Ordners auf dem
Server (Standard https://jarns.ch/software); daraus wird gelesen, was schon dort liegt.

Ergebnis im Arbeitsordner:
  site/launcher/<version>/…      neuester Launcher (Windows, Linux, Prüfsummen)
  site/releases/<tag>/…          neueste Version jedes Programms (alle Dateien des Releases)
  ctl/apps.toml                  Programmliste
  ctl/index.json                 Welche Versionen auf dem Server liegen (liest der Launcher)
  ctl/launcher/latest.json       Neuester Launcher mit Prüfsummen (liest der Launcher beim Start)
Hochgeladen wird erst `site` (nur Fehlendes), danach `ctl`, damit nie ein Verweis ins Leere zeigt.
Mit «force» wird alles neu geholt, auch was der Server schon hat.
"""
import datetime, json, os, pathlib, subprocess, sys, tomllib, urllib.request

repo, work = sys.argv[1], pathlib.Path(sys.argv[2])
force = len(sys.argv) > 3 and sys.argv[3] == "force"
server = os.environ.get("SERVER_URL", "https://jarns.ch/software").rstrip("/")

site, ctl = work / "site", work / "ctl"
(ctl / "launcher").mkdir(parents=True, exist_ok=True)
site.mkdir(parents=True, exist_ok=True)


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True)


def remote_json(path):
    try:
        with urllib.request.urlopen(f"{server}/{path}", timeout=30) as r:
            return json.load(r)
    except Exception as e:  # noch nichts auf dem Server, oder nicht erreichbar: alles neu holen
        print(f"{server}/{path}: {e}")
        return {}


def version(tag, prefix):
    rest = tag[len(prefix):]
    if "-" in rest:  # Vorabversion
        return None
    try:
        return tuple(int(x) for x in rest.split("."))
    except ValueError:
        return None


tags = [t["tagName"] for t in json.loads(gh("release", "list", "--repo", repo, "--limit", "300", "--json", "tagName"))]


def latest(prefix):
    found = [(version(t, prefix), t) for t in tags if t.startswith(prefix)]
    found = [f for f in found if f[0] is not None]
    return max(found)[1] if found else None


def fmt(v):
    return ".".join(map(str, v))


old = {} if force else remote_json("index.json")
index = {"launcher": {}, "apps": {}}

# --- Launcher -----------------------------------------------------------------------------------
ltag = latest("werkbank-v")
if not ltag:
    sys.exit("kein Launcher-Release gefunden")
lver = ltag[len("werkbank-v"):]
files = {"windows-x64": "werkbank-windows-x64.zip", "linux-x86_64": "werkbank-linux-x86_64.tar.gz"}
tmp = work / "tmp-launcher"
tmp.mkdir(exist_ok=True)
gh("release", "download", ltag, "--repo", repo, "--pattern", "SHA256SUMS.txt", "--dir", str(tmp), "--clobber")
sums = {}
for line in (tmp / "SHA256SUMS.txt").read_text().splitlines():
    parts = line.split()
    if len(parts) == 2:
        sums[parts[1].lstrip("*")] = parts[0].lower()
latest_json = {"version": lver, "files": {}}
for key, name in files.items():
    if name not in sums:
        sys.exit(f"{name} fehlt im Release {ltag}")
    latest_json["files"][key] = {"file": name, "sha256": sums[name]}
if old.get("launcher", {}).get("version") != lver:
    dest = site / "launcher" / lver
    dest.mkdir(parents=True, exist_ok=True)
    gh("release", "download", ltag, "--repo", repo, "--dir", str(dest), "--clobber",
       *sum((["--pattern", n] for n in [*files.values(), "SHA256SUMS.txt"]), []))
    print(f"Launcher {lver}: wird hochgeladen")
else:
    print(f"Launcher {lver}: liegt schon auf dem Server")
index["launcher"] = {"version": lver}
(ctl / "launcher" / "latest.json").write_text(json.dumps(latest_json, indent=1))

# --- Programme ----------------------------------------------------------------------------------
apps = tomllib.loads(pathlib.Path("launcher/apps.toml").read_text(encoding="utf-8"))["app"]
for a in apps:
    prefix = a.get("tag", "v")
    t = latest(prefix)
    if not t:
        print(f"::warning::{a['id']}: kein Release")
        continue
    v = t[len(prefix):]
    known = set(old.get("apps", {}).get(prefix, []))
    if v not in known:
        dest = site / "releases" / t
        dest.mkdir(parents=True, exist_ok=True)
        gh("release", "download", t, "--repo", repo, "--dir", str(dest), "--clobber")
        print(f"{a['id']} {v}: wird hochgeladen")
    known.add(v)
    index["apps"][prefix] = sorted(known, key=lambda s: tuple(int(x) for x in s.split(".")))

index["updated"] = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
(ctl / "index.json").write_text(json.dumps(index, indent=1))
(ctl / "apps.toml").write_text(pathlib.Path("launcher/apps.toml").read_text(encoding="utf-8"), encoding="utf-8")
print("fertig:", work)
