<?php
// Werkbank-Lizenzserver: Schnittstelle für den Launcher.
//
// POST JSON {aktion: "aktivieren", code, geraet, name, benutzer, system, version, programme[]}
//   → {ok: true, lizenz} oder {ok: false, grund: "code" | "gesperrt" | ...}
// POST JSON {aktion: "melden", geraet, name, benutzer, system, version, programme[]}
//   → {ok: true, lizenz} oder {ok: false, grund: "unbekannt" | "gesperrt" | "neu"}
// Antwortet immer mit HTTP 200 und JSON, damit der Launcher Fehler einfach lesen kann.

declare(strict_types=1);
require __DIR__ . '/lib.php';

header('Content-Type: application/json; charset=utf-8');
header('Cache-Control: no-store');

function antwort(array $a): void {
    echo json_encode($a, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
    exit;
}

function kurz($v, int $max): string {
    $s = is_scalar($v) ? (string)$v : '';
    $s = preg_replace('/[\x00-\x1F\x7F]/u', '', $s) ?? '';
    return mb_substr(trim($s), 0, $max);
}

if (isset($_GET['status'])) {
    antwort(['ok' => true, 'eingerichtet' => eingerichtet(), 'sodium' => function_exists('sodium_crypto_sign_detached'), 'geheim' => geheim() !== null]);
}

if (($_SERVER['REQUEST_METHOD'] ?? '') !== 'POST') {
    antwort(['ok' => false, 'grund' => 'methode']);
}

$roh = file_get_contents('php://input');
if ($roh === false || strlen($roh) > 64 * 1024) {
    antwort(['ok' => false, 'grund' => 'anfrage']);
}
$a = json_decode($roh, true);
if (!is_array($a)) {
    antwort(['ok' => false, 'grund' => 'anfrage']);
}
if (!eingerichtet()) {
    antwort(['ok' => false, 'grund' => 'server']);
}

$geraet = kurz($a['geraet'] ?? '', 64);
if (!preg_match('/^[a-f0-9]{16,64}$/', $geraet)) {
    antwort(['ok' => false, 'grund' => 'geraet']);
}
$programme = [];
if (isset($a['programme']) && is_array($a['programme'])) {
    foreach (array_slice($a['programme'], 0, 100) as $p) {
        $p = kurz($p, 40);
        if ($p !== '') {
            $programme[] = $p;
        }
    }
}
$angaben = [
    'name' => kurz($a['name'] ?? '', 80),
    'benutzer' => kurz($a['benutzer'] ?? '', 80),
    'system' => kurz($a['system'] ?? '', 40),
    'version' => kurz($a['version'] ?? '', 24),
    'programme' => $programme,
    'ip' => kurz($_SERVER['REMOTE_ADDR'] ?? '', 64),
    'zuletzt' => time(),
];

try {
    $ergebnis = gesperrt(function () use ($a, $geraet, $angaben) {
        $e = einstellungen();
        $liste = lesen('geraete.json', []);
        $aktion = $a['aktion'] ?? '';
        $g = $liste[$geraet] ?? null;
        if ($aktion === 'aktivieren') {
            $code = code_norm(kurz($a['code'] ?? '', 64));
            if ($code === '' || !password_verify($code, $e['code'])) {
                usleep(400000); // Raten bremsen
                return ['ok' => false, 'grund' => 'code'];
            }
            if ($g !== null && !empty($g['gesperrt'])) {
                return ['ok' => false, 'grund' => 'gesperrt'];
            }
            $g = array_merge($g ?? ['erst' => time(), 'aktivierungen' => 0], $angaben);
            $g['aktivierungen'] = (int)($g['aktivierungen'] ?? 0) + 1;
            $g['generation'] = (int)($e['generation'] ?? 1);
        } elseif ($aktion === 'melden') {
            if ($g === null) {
                return ['ok' => false, 'grund' => 'unbekannt'];
            }
            if (!empty($g['gesperrt'])) {
                $g['zuletzt'] = time();
                $liste[$geraet] = $g;
                schreiben('geraete.json', $liste);
                return ['ok' => false, 'grund' => 'gesperrt'];
            }
            if ((int)($g['generation'] ?? 1) < (int)($e['generation'] ?? 1)) {
                return ['ok' => false, 'grund' => 'neu'];
            }
            // Nur Angaben übernehmen, die das Gerät auch geschickt hat.
            $neu = array_filter($angaben, fn($v, $k) => $v !== '' && ($k !== 'programme' || isset($a['programme'])), ARRAY_FILTER_USE_BOTH);
            $g = array_merge($g, $neu);
        } else {
            return ['ok' => false, 'grund' => 'aktion'];
        }
        $liste[$geraet] = $g;
        schreiben('geraete.json', $liste);
        return ['ok' => true, 'lizenz' => lizenz($geraet), 'tage' => LIZENZ_TAGE];
    });
    antwort($ergebnis);
} catch (Throwable $t) {
    error_log('werkbank api: ' . $t->getMessage());
    antwort(['ok' => false, 'grund' => 'server', 'meldung' => $t->getMessage()]);
}
