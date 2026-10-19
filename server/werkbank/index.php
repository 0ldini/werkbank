<?php
// Werkbank-Lizenzserver: Verwaltung. Beim ersten Aufruf Einrichtung (Passwort und Code),
// danach Anmeldung und Übersicht der Geräte.

declare(strict_types=1);
require __DIR__ . '/lib.php';

session_name('werkbank_admin');
session_set_cookie_params(['httponly' => true, 'samesite' => 'Strict', 'secure' => !empty($_SERVER['HTTPS'])]);
session_start();
header('Cache-Control: no-store');
header('X-Frame-Options: DENY');

if (empty($_SESSION['csrf'])) {
    $_SESSION['csrf'] = bin2hex(random_bytes(16));
}
$csrf = $_SESSION['csrf'];
$meldung = '';
$fehler = '';

function post_ok(string $csrf): bool {
    return ($_SERVER['REQUEST_METHOD'] ?? '') === 'POST' && hash_equals($csrf, (string)($_POST['csrf'] ?? ''));
}

// Voraussetzungen
$probleme = [];
if (!function_exists('sodium_crypto_sign_detached')) {
    $probleme[] = 'Die PHP-Erweiterung «sodium» fehlt. Beim Hoster PHP 7.2 oder neuer mit sodium einschalten.';
}
if (geheim() === null) {
    $probleme[] = 'Die Datei geheim.php fehlt. Sie gehört in denselben Ordner wie diese Seite.';
}
$data = __DIR__ . '/data';
if (!is_dir($data)) {
    @mkdir($data, 0775, true);
}
if (!is_dir($data) || !is_writable($data)) {
    $probleme[] = 'Der Ordner data/ ist nicht beschreibbar (Rechte 775, notfalls 777).';
}

// Einrichtung
if (!$probleme && !eingerichtet() && post_ok($csrf) && ($_POST['tun'] ?? '') === 'einrichten') {
    $pw = (string)($_POST['passwort'] ?? '');
    $code = code_norm((string)($_POST['code'] ?? ''));
    if (strlen($pw) < 8) {
        $fehler = 'Das Passwort braucht mindestens 8 Zeichen.';
    } elseif (strlen($code) < 8) {
        $fehler = 'Der Code braucht mindestens 8 Buchstaben oder Ziffern.';
    } else {
        schreiben('einstellungen.json', [
            'admin' => password_hash($pw, PASSWORD_DEFAULT),
            'code' => password_hash($code, PASSWORD_DEFAULT),
            'code_anzeige' => trim((string)$_POST['code']),
            'generation' => 1,
        ]);
        $_SESSION['admin'] = true;
        $meldung = 'Eingerichtet. Gib den Code an alle weiter, die die Werkbank nutzen sollen.';
    }
}

// Anmelden / Abmelden
if (eingerichtet() && post_ok($csrf) && ($_POST['tun'] ?? '') === 'anmelden') {
    usleep(300000);
    if (password_verify((string)($_POST['passwort'] ?? ''), einstellungen()['admin'])) {
        session_regenerate_id(true);
        $_SESSION['admin'] = true;
    } else {
        $fehler = 'Passwort falsch.';
    }
}
if (isset($_GET['abmelden'])) {
    $_SESSION = [];
    session_destroy();
    header('Location: ./');
    exit;
}
$admin = !empty($_SESSION['admin']) && eingerichtet();

// Aktionen
if ($admin && post_ok($csrf)) {
    $tun = (string)($_POST['tun'] ?? '');
    $id = (string)($_POST['geraet'] ?? '');
    try {
        if (in_array($tun, ['sperren', 'freigeben', 'loeschen'], true)) {
            gesperrt(function () use ($tun, $id, &$meldung) {
                $liste = lesen('geraete.json', []);
                if (!isset($liste[$id])) {
                    return;
                }
                $name = $liste[$id]['name'] ?: $id;
                if ($tun === 'loeschen') {
                    unset($liste[$id]);
                    $meldung = "«{$name}» entfernt. Meldet es sich wieder, muss der Code neu eingegeben werden.";
                } else {
                    $liste[$id]['gesperrt'] = $tun === 'sperren';
                    $meldung = $tun === 'sperren' ? "«{$name}» gesperrt." : "«{$name}» wieder freigegeben.";
                }
                schreiben('geraete.json', $liste);
            });
        } elseif ($tun === 'code') {
            $code = code_norm((string)($_POST['code'] ?? ''));
            if (strlen($code) < 8) {
                $fehler = 'Der Code braucht mindestens 8 Buchstaben oder Ziffern.';
            } else {
                $e = einstellungen();
                $e['code'] = password_hash($code, PASSWORD_DEFAULT);
                $e['code_anzeige'] = trim((string)$_POST['code']);
                if (!empty($_POST['alle'])) {
                    $e['generation'] = (int)($e['generation'] ?? 1) + 1;
                }
                schreiben('einstellungen.json', $e);
                $meldung = !empty($_POST['alle'])
                    ? 'Neuer Code gespeichert. Alle Geräte müssen ihn beim nächsten Start eingeben.'
                    : 'Neuer Code gespeichert. Bisherige Geräte laufen weiter, neue brauchen den neuen Code.';
            }
        } elseif ($tun === 'passwort') {
            $pw = (string)($_POST['passwort'] ?? '');
            if (strlen($pw) < 8) {
                $fehler = 'Das Passwort braucht mindestens 8 Zeichen.';
            } else {
                $e = einstellungen();
                $e['admin'] = password_hash($pw, PASSWORD_DEFAULT);
                schreiben('einstellungen.json', $e);
                $meldung = 'Passwort geändert.';
            }
        }
    } catch (Throwable $t) {
        $fehler = $t->getMessage();
    }
}

$geraete = $admin ? lesen('geraete.json', []) : [];
uasort($geraete, fn($a, $b) => ($b['zuletzt'] ?? 0) <=> ($a['zuletzt'] ?? 0));
$jetzt = time();
$aktiv = count(array_filter($geraete, fn($g) => empty($g['gesperrt']) && ($jetzt - ($g['zuletzt'] ?? 0)) < AKTIV_TAGE * 86400));
$gesperrt_n = count(array_filter($geraete, fn($g) => !empty($g['gesperrt'])));
$programme = [];
foreach ($geraete as $g) {
    foreach ($g['programme'] ?? [] as $p) {
        $programme[$p] = ($programme[$p] ?? 0) + 1;
    }
}
arsort($programme);
$e = einstellungen();
?><!doctype html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>Werkbank · Lizenzen</title>
<style>
:root{--bg:#F3F4F6;--panel:#fff;--border:#E1E4EA;--text:#17181C;--muted:#5B616E;--accent:#F2B544;--ink:#17181C;--red:#B91C1C;--green:#0F766E;--chip:#EDEFF3}
@media (prefers-color-scheme:dark){:root{--bg:#17181C;--panel:#212329;--border:#32353D;--text:#ECEEF2;--muted:#A9AEB9;--red:#F87171;--green:#5EEAD4;--chip:#2A2D35}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
header{display:flex;align-items:center;gap:12px;padding:14px 20px;background:var(--panel);border-bottom:1px solid var(--border)}
header .logo{width:30px;height:30px;border-radius:8px;background:var(--accent);display:grid;grid-template-columns:1fr 1fr;gap:3px;padding:6px}
header .logo i{background:var(--ink);border-radius:2px}
header h1{font-size:19px;margin:0;flex-grow:1}
header a{color:var(--muted)}
main{max-width:1100px;margin:0 auto;padding:20px 16px 40px;display:flex;flex-direction:column;gap:16px}
.karte{background:var(--panel);border:1px solid var(--border);border-radius:14px;padding:18px}
.karte h2{margin:0 0 10px;font-size:17px}
.zahlen{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px}
.zahl{background:var(--panel);border:1px solid var(--border);border-radius:14px;padding:14px 16px}
.zahl b{display:block;font-size:30px;line-height:1.1}
.zahl span{color:var(--muted);font-size:13px}
table{width:100%;border-collapse:collapse}
th{text-align:left;font-size:12px;color:var(--muted);font-weight:600;padding:6px 8px;border-bottom:1px solid var(--border)}
td{padding:9px 8px;border-bottom:1px solid var(--border);vertical-align:top}
tr:last-child td{border-bottom:0}
.klein{font-size:12.5px;color:var(--muted)}
.marke{display:inline-block;padding:1px 8px;border-radius:10px;font-size:12px;background:var(--chip)}
.marke.rot{color:var(--red)}.marke.gruen{color:var(--green)}
button,input{font:inherit}
input[type=text],input[type=password]{padding:9px 11px;border:1px solid var(--border);border-radius:10px;background:var(--bg);color:var(--text);min-width:0;width:100%;max-width:340px}
button{padding:8px 14px;border-radius:10px;border:1px solid var(--border);background:transparent;color:var(--text);cursor:pointer}
button.haupt{background:var(--accent);border-color:var(--accent);color:var(--ink);font-weight:600}
button.rot{color:var(--red)}
form.zeile{display:flex;gap:8px;flex-wrap:wrap;align-items:center}
.hinweis{padding:10px 14px;border-radius:10px;background:var(--chip)}
.hinweis.fehler{color:var(--red)}
.chips{display:flex;flex-wrap:wrap;gap:6px}
.code{font-family:ui-monospace,Consolas,monospace;font-size:17px;letter-spacing:1px}
@media (max-width:760px){
 thead{display:none}
 table,tbody,tr,td{display:block;width:100%}
 tr{border-bottom:1px solid var(--border);padding:8px 0}
 td{border:0;padding:3px 0}
}
</style>
</head>
<body>
<header><div class="logo"><i></i><i></i><i></i><i></i></div><h1>Werkbank · Lizenzen</h1><?php if ($admin): ?><a href="?abmelden">Abmelden</a><?php endif; ?></header>
<main>
<?php if ($meldung): ?><div class="hinweis"><?= h($meldung) ?></div><?php endif; ?>
<?php if ($fehler): ?><div class="hinweis fehler"><?= h($fehler) ?></div><?php endif; ?>

<?php if ($probleme): ?>
  <div class="karte"><h2>Noch nicht bereit</h2><?php foreach ($probleme as $p): ?><p><?= h($p) ?></p><?php endforeach; ?></div>

<?php elseif (!eingerichtet()): ?>
  <div class="karte">
    <h2>Einrichten</h2>
    <p class="klein">Lege das Passwort für diese Seite fest und den Code, den alle beim ersten Start der Werkbank eingeben.</p>
    <form method="post" style="display:flex;flex-direction:column;gap:10px">
      <input type="hidden" name="csrf" value="<?= h($csrf) ?>"><input type="hidden" name="tun" value="einrichten">
      <label>Passwort für diese Seite<br><input type="password" name="passwort" minlength="8" required autocomplete="new-password"></label>
      <label>Code für die Werkbank<br><input type="text" name="code" value="<?= h(code_vorschlag()) ?>" required class="code"></label>
      <div><button class="haupt">Einrichten</button></div>
    </form>
  </div>

<?php elseif (!$admin): ?>
  <div class="karte">
    <h2>Anmelden</h2>
    <form method="post" class="zeile">
      <input type="hidden" name="csrf" value="<?= h($csrf) ?>"><input type="hidden" name="tun" value="anmelden">
      <input type="password" name="passwort" placeholder="Passwort" required autofocus autocomplete="current-password">
      <button class="haupt">Anmelden</button>
    </form>
  </div>

<?php else: ?>
  <section class="zahlen">
    <div class="zahl"><b><?= count($geraete) ?></b><span>Geräte insgesamt</span></div>
    <div class="zahl"><b><?= $aktiv ?></b><span>aktiv (letzte <?= AKTIV_TAGE ?> Tage)</span></div>
    <div class="zahl"><b><?= $gesperrt_n ?></b><span>gesperrt</span></div>
    <div class="zahl"><b><?= count($programme) ?></b><span>verschiedene Programme installiert</span></div>
  </section>

  <div class="karte">
    <h2>Geräte</h2>
    <?php if (!$geraete): ?><p class="klein">Noch kein Gerät freigeschaltet.</p><?php else: ?>
    <table>
      <thead><tr><th>Gerät</th><th>System · Werkbank</th><th>Programme</th><th>Zuerst · zuletzt</th><th></th></tr></thead>
      <tbody>
      <?php foreach ($geraete as $id => $g): $alt = ($jetzt - ($g['zuletzt'] ?? 0)) >= AKTIV_TAGE * 86400; ?>
        <tr>
          <td><b><?= h($g['name'] ?: 'Unbenannt') ?></b><?php if (!empty($g['gesperrt'])): ?> <span class="marke rot">gesperrt</span><?php elseif (!$alt): ?> <span class="marke gruen">aktiv</span><?php endif; ?>
            <div class="klein"><?= h(implode(' · ', array_filter([$g['benutzer'] ?? '', $g['ip'] ?? '']))) ?></div></td>
          <td><?= h($g['system'] ?? '') ?><div class="klein">Werkbank <?= h($g['version'] ?? '?') ?></div></td>
          <td><?= count($g['programme'] ?? []) ?><div class="klein" title="<?= h(implode(', ', $g['programme'] ?? [])) ?>"><?= h(implode(', ', array_slice($g['programme'] ?? [], 0, 4))) ?><?= count($g['programme'] ?? []) > 4 ? ' …' : '' ?></div></td>
          <td><?= h(wann($g['erst'] ?? null)) ?><div class="klein"><?= h(wann($g['zuletzt'] ?? null)) ?></div></td>
          <td>
            <form method="post" class="zeile">
              <input type="hidden" name="csrf" value="<?= h($csrf) ?>"><input type="hidden" name="geraet" value="<?= h($id) ?>">
              <?php if (!empty($g['gesperrt'])): ?><button name="tun" value="freigeben">Freigeben</button><?php else: ?><button name="tun" value="sperren" class="rot">Sperren</button><?php endif; ?>
              <button name="tun" value="loeschen" onclick="return confirm('Gerät aus der Liste entfernen?')">Entfernen</button>
            </form>
          </td>
        </tr>
      <?php endforeach; ?>
      </tbody>
    </table>
    <?php endif; ?>
  </div>

  <?php if ($programme): ?>
  <div class="karte">
    <h2>Installierte Programme</h2>
    <div class="chips"><?php foreach ($programme as $p => $n): ?><span class="marke"><?= h($p) ?> · <?= $n ?></span><?php endforeach; ?></div>
  </div>
  <?php endif; ?>

  <div class="karte">
    <h2>Code</h2>
    <p>Aktueller Code: <span class="code"><?= h($e['code_anzeige'] ?? '') ?></span></p>
    <form method="post" style="display:flex;flex-direction:column;gap:8px">
      <input type="hidden" name="csrf" value="<?= h($csrf) ?>"><input type="hidden" name="tun" value="code">
      <input type="text" name="code" value="<?= h(code_vorschlag()) ?>" class="code" required>
      <label class="klein"><input type="checkbox" name="alle" value="1"> Alle Geräte müssen den neuen Code eingeben</label>
      <div><button class="haupt">Neuen Code speichern</button></div>
    </form>
  </div>

  <div class="karte">
    <h2>Passwort dieser Seite</h2>
    <form method="post" class="zeile">
      <input type="hidden" name="csrf" value="<?= h($csrf) ?>"><input type="hidden" name="tun" value="passwort">
      <input type="password" name="passwort" placeholder="Neues Passwort" minlength="8" required autocomplete="new-password">
      <button>Ändern</button>
    </form>
    <p class="klein">Eine Lizenz gilt <?= LIZENZ_TAGE ?> Tage ohne Verbindung; bei jedem Start mit Internet wird sie erneuert. Gesperrte Geräte werden beim nächsten Start mit Internet gesperrt, spätestens nach <?= LIZENZ_TAGE ?> Tagen.</p>
  </div>
<?php endif; ?>
</main>
</body>
</html>
