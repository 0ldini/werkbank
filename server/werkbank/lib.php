<?php
// Werkbank-Lizenzserver: gemeinsame Teile für api.php und index.php.
// Daten liegen als JSON in data/ (per .htaccess gesperrt). Der geheime Schlüssel steht in
// geheim.php neben dieser Datei – sie gehört NICHT ins Git, sondern wird einmal von Hand
// hochgeladen.

declare(strict_types=1);

const LIZENZ_TAGE = 60;          // so lange gilt eine Lizenz ohne Verbindung zum Server
const AKTIV_TAGE = 30;           // «aktiv» heisst: in so vielen Tagen gemeldet

function datei(string $name): string {
    return __DIR__ . '/data/' . $name;
}

function lesen(string $name, $leer) {
    $p = datei($name);
    if (!is_file($p)) {
        return $leer;
    }
    $t = file_get_contents($p);
    $v = json_decode($t === false ? '' : $t, true);
    return is_array($v) ? $v : $leer;
}

function schreiben(string $name, $wert): void {
    $dir = __DIR__ . '/data';
    if (!is_dir($dir)) {
        @mkdir($dir, 0775, true);
    }
    $p = datei($name);
    $tmp = $p . '.' . bin2hex(random_bytes(4)) . '.tmp';
    if (file_put_contents($tmp, json_encode($wert, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES), LOCK_EX) === false) {
        throw new RuntimeException('data/ ist nicht beschreibbar');
    }
    rename($tmp, $p);
}

/** Sperre um eine ganze Lesen-Ändern-Schreiben-Folge. */
function gesperrt(callable $f) {
    $dir = __DIR__ . '/data';
    if (!is_dir($dir)) {
        @mkdir($dir, 0775, true);
    }
    $h = fopen($dir . '/.sperre', 'c');
    if ($h) {
        flock($h, LOCK_EX);
    }
    try {
        return $f();
    } finally {
        if ($h) {
            flock($h, LOCK_UN);
            fclose($h);
        }
    }
}

function einstellungen(): array {
    return lesen('einstellungen.json', []);
}

function eingerichtet(): bool {
    $e = einstellungen();
    return isset($e['admin'], $e['code']);
}

/** Code vergleichbar machen: Gross, ohne Leerzeichen und Striche. */
function code_norm(string $c): string {
    return strtoupper(preg_replace('/[^A-Za-z0-9]/', '', $c) ?? '');
}

function code_vorschlag(): string {
    $z = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
    $teile = [];
    for ($i = 0; $i < 3; $i++) {
        $t = '';
        for ($j = 0; $j < 4; $j++) {
            $t .= $z[random_int(0, strlen($z) - 1)];
        }
        $teile[] = $t;
    }
    return 'WERK-' . implode('-', $teile);
}

function geheim(): ?array {
    $p = __DIR__ . '/geheim.php';
    if (!is_file($p)) {
        return null;
    }
    $g = include $p;
    return is_array($g) && isset($g['seed']) ? $g : null;
}

function schluesselpaar(): string {
    $g = geheim();
    if ($g === null) {
        throw new RuntimeException('geheim.php fehlt');
    }
    if (!function_exists('sodium_crypto_sign_seed_keypair')) {
        throw new RuntimeException('PHP-Erweiterung sodium fehlt');
    }
    return sodium_crypto_sign_seed_keypair(hex2bin($g['seed']));
}

function oeffentlicher_schluessel(): string {
    return bin2hex(sodium_crypto_sign_publickey(schluesselpaar()));
}

/** Lizenz für ein Gerät: Zeile 1 die Angaben, Zeile 2 die Unterschrift (hex). */
function lizenz(string $geraet): string {
    $ablauf = time() + LIZENZ_TAGE * 86400;
    $payload = "werkbank|1|$geraet|$ablauf";
    $sig = sodium_crypto_sign_detached($payload, sodium_crypto_sign_secretkey(schluesselpaar()));
    return $payload . "\n" . bin2hex($sig);
}

function h(?string $s): string {
    return htmlspecialchars((string)$s, ENT_QUOTES, 'UTF-8');
}

function wann(?int $t): string {
    if (!$t) {
        return '–';
    }
    $d = time() - $t;
    if ($d < 120) return 'gerade eben';
    if ($d < 3600) return 'vor ' . intdiv($d, 60) . ' Min.';
    if ($d < 86400) return 'vor ' . intdiv($d, 3600) . ' Std.';
    if ($d < 86400 * 7) return 'vor ' . intdiv($d, 86400) . ' Tagen';
    return date('d.m.Y', $t);
}
