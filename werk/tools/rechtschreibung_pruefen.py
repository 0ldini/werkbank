#!/usr/bin/env python3
"""Prüft nach dem Bau, ob das eingebaute Wörterbuch wirklich prüft (Schreibwerk, Linux).

Lässt die gebaute Befehlszeile (wordcraft-cli) einen deutschen Text in Deutsch (Schweiz) und einen
englischen in Englisch (USA) prüfen und meldet das Ergebnis als Anmerkung am Lauf: «::notice»,
wenn die Tippfehler markiert und die richtigen Wörter (auch «Strasse», «grösser») nicht markiert
sind, sonst «::warning». Bricht den Build nie ab.

Aufruf:  python3 werk/tools/rechtschreibung_pruefen.py <wordcraft-cli> <app> <art>
"""
import json
import subprocess
import sys

for stream in (sys.stdout, sys.stderr):
    stream.reconfigure(encoding="utf-8", errors="replace")

CLI, APP, KIND = sys.argv[1:4]

CASES = [
    (
        "de-CH",
        "Die Strasse ist nass, und das Wasser ist grösser als gedacht. Das Wassser ist kallt.",
        {"Wassser", "kallt"},
    ),
    ("en-US", "The house is red and the doog barks loudly.", {"doog"}),
]


def flagged(lang: str, text: str):
    cmds = [
        f"text.insert={json.dumps({'text': text})}",
        "select.all",
        f"review.language={json.dumps({'lang': lang})}",
        "review.issues",
    ]
    args = [CLI, "run", "--print"]
    for c in cmds:
        args += ["--cmd", c]
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", timeout=600)
    if r.returncode != 0:
        raise RuntimeError(f"{lang}: Exit {r.returncode}: {r.stderr.strip()[-300:]}")
    for line in r.stdout.splitlines():
        if line.startswith("review.issues: "):
            issues = json.loads(line[len("review.issues: "):])
            return {i.get("text", "") for i in issues if i.get("kind") == "spelling"}, {i.get("lang", "") for i in issues}
    raise RuntimeError(f"{lang}: keine Ausgabe von review.issues")


def main():
    ok = True
    report = []
    for lang, text, want in CASES:
        try:
            got, langs = flagged(lang, text)
        except Exception as e:  # nie den Build abbrechen
            ok = False
            report.append(f"{lang}: {e}")
            continue
        good = got == want
        ok &= good
        report.append(f"{lang}: markiert {sorted(got)} (erwartet {sorted(want)}, Sprachen {sorted(langs)}){'' if good else ' FALSCH'}")
    text = " | ".join(report).replace("%", "%25").replace("\n", " ")
    level = "notice" if ok else "warning"
    print(f"::{level} title={APP} {KIND} Rechtschreibprüfung::{text}")


main()
