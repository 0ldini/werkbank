#!/usr/bin/env python3
"""Browser-Version mit Zwischenspeicher: legt einen Service Worker (werk-cache.js) in das
Web-ZIP und meldet ihn in index.html an.

Beim ersten Öffnen speichert der Browser alle Programmdateien in seinem Cache-Speicher; danach
kommen sie von dort (auch offline, und der Browser kann die übersetzte .wasm-Datei behalten),
unabhängig davon, wie der Webserver eingestellt ist. index.html holt er zuerst vom Server, damit
ein Update ankommt; ist eine neue Version da, lädt er sie einmal und räumt die alte weg.

Aufruf:  python3 werk/tools/web_cache.py <web.zip> <app> <version>
"""

import json
import pathlib
import re
import sys
import tempfile
import zipfile

# Dateien, die das Programm nicht zum Laufen braucht, kommen nicht in den Speicher.
SKIP = re.compile(r"(^|/)(README|HOSTING|NOTICE|ATTRIBUTION|LICENSE[^/]*|_headers|\.htaccess)(\.[a-z]+)?$", re.I)

WORKER = """// Werkbank: Zwischenspeicher der Browser-Version (erzeugt von werk/tools/web_cache.py).
const CACHE = %(cache)s;
const PREFIX = %(prefix)s;
const FILES = %(files)s;

self.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // Einzeln, damit eine fehlende Datei nicht alles verhindert.
    await Promise.all(FILES.map((f) => cache.add(new Request(f, { cache: "reload" })).catch(() => {})));
    await self.skipWaiting();
  })());
});

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    for (const name of await caches.keys()) {
      if (name.startsWith(PREFIX) && name !== CACHE) await caches.delete(name);
    }
    await self.clients.claim();
  })());
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== location.origin || !url.pathname.startsWith(new URL("./", location).pathname)) return;
  const page = req.mode === "navigate" || url.pathname.endsWith("/") || url.pathname.endsWith("/index.html");
  if (page) {
    // Zuerst der Server (Updates), ohne Netz die gespeicherte Seite.
    event.respondWith((async () => {
      try {
        const res = await fetch(req);
        if (res.ok) (await caches.open(CACHE)).put("./index.html", res.clone());
        return res;
      } catch (e) {
        return (await caches.match("./index.html", { cacheName: CACHE })) || Response.error();
      }
    })());
    return;
  }
  // Programmdateien: aus dem Speicher, sonst vom Server (und dann merken).
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(req, { ignoreSearch: true });
    if (hit) return hit;
    const res = await fetch(req);
    if (res.ok && res.type === "basic") cache.put(req, res.clone());
    return res;
  })());
});
"""

REGISTER = (
    '<script>if ("serviceWorker" in navigator && window.isSecureContext) '
    '{ navigator.serviceWorker.register("./werk-cache.js").catch(() => {}); }</script>\n'
)


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__)
        return 2
    zpath, app, version = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    with zipfile.ZipFile(zpath) as z:
        entries = [(i, z.read(i)) for i in z.infolist()]
    index = [i for i, _ in entries if i.filename.endswith("index.html")]
    if not index:
        print(f"{zpath.name}: keine index.html, nichts geändert")
        return 0
    idx = min(index, key=lambda i: i.filename.count("/"))
    root = idx.filename[: -len("index.html")]
    # Ein schon vorhandener Helfer (zweiter Aufruf) wird ersetzt, nicht verdoppelt.
    files = ["./index.html"]
    for i, _ in entries:
        if i.is_dir() or not i.filename.startswith(root):
            continue
        rel = i.filename[len(root):]
        if rel in ("index.html", "werk-cache.js") or SKIP.search(rel):
            continue
        files.append("./" + rel)
    prefix = f"werk-{app}-"
    worker = WORKER % {
        "cache": json.dumps(prefix + version),
        "prefix": json.dumps(prefix),
        "files": json.dumps(sorted(files), indent=2),
    }
    out = []
    for i, data in entries:
        if i.filename == root + "werk-cache.js":
            continue
        if i.filename == idx.filename:
            html = data.decode("utf-8")
            if "werk-cache.js" not in html:
                html = html.replace("</head>", REGISTER + "</head>", 1) if "</head>" in html else REGISTER + html
            data = html.encode("utf-8")
        elif i.filename == root + ".htaccess":
            text = data.decode("utf-8", "replace")
            if "werk-cache.js" not in text:
                text += (
                    "\n# Werkbank: den Zwischenspeicher-Helfer immer frisch prüfen.\n"
                    "<IfModule mod_headers.c>\n  <Files \"werk-cache.js\">\n"
                    "    Header set Cache-Control \"no-cache\"\n  </Files>\n</IfModule>\n"
                )
            data = text.encode("utf-8")
        out.append((i, data))
    with tempfile.NamedTemporaryFile(delete=False, dir=zpath.parent, suffix=".zip") as tmp:
        tmp_path = pathlib.Path(tmp.name)
    with zipfile.ZipFile(tmp_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for i, data in out:
            z.writestr(i, data)
        info = zipfile.ZipInfo(root + "werk-cache.js", date_time=idx.date_time)
        info.external_attr = 0o644 << 16
        z.writestr(info, worker, zipfile.ZIP_DEFLATED)
    tmp_path.replace(zpath)
    print(f"{zpath.name}: Zwischenspeicher eingebaut ({len(files)} Dateien, Speicher {prefix + version})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
