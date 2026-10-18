# Prüft ladewerk.exe unter Windows: HTTPS-Downloads, Pause, Entpacken, Konto (DPAPI), Zwischenablage, Fenster.
$ErrorActionPreference = 'Stop'
$root = $PWD.Path
$exe = "$root\ladewerk.exe"
$warn = @()

function Api($port, $path, $body) {
  if ($null -eq $body) { return Invoke-RestMethod "http://127.0.0.1:$port$path" }
  $bytes = [Text.Encoding]::UTF8.GetBytes(($body | ConvertTo-Json -Depth 5))
  Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port$path" -ContentType 'application/json; charset=utf-8' -Body $bytes
}
function Items($port) { @((Api $port '/api/state').items) }
function Show($port) { Items $port | Format-Table stage, status, pkg, name, done, size, conns, extract, extractMsg, error -AutoSize | Out-String -Width 260 | Write-Host }
function WaitFor($port, [scriptblock]$cond, $secs, $what) {
  foreach ($i in 1..$secs) { if (& $cond) { return }; Start-Sleep 1 }
  Show $port
  throw "Zeitüberschreitung: $what"
}
function StartApp($port, $data, $envs, $appArgs) {
  $env:LADEWERK_DATA = "$root\$data"
  foreach ($k in $envs.Keys) { Set-Item "env:$k" $envs[$k] }
  $a = @("--port=$port") + $appArgs
  $p = Start-Process $exe -ArgumentList $a -PassThru -RedirectStandardOutput "$root\app-$port.log" -RedirectStandardError "$root\app-$port-err.log"
  foreach ($k in $envs.Keys) { Remove-Item "env:$k" }
  Start-Sleep 2
  return $p
}
function QuitApp($port, $proc) {
  Api $port '/api/quit' @{} | Out-Null
  if (-not $proc.WaitForExit(15000)) { throw "Ladewerk (Port $port) hat sich nach «Beenden» nicht beendet" }
}
function Sha($f) { (Get-FileHash $f -Algorithm SHA256).Hash.ToLower() }

# ===== 1. Internet: HTTPS, mehrere Verbindungen, Pause, Weiterleitung, toter Link, Entpacken =====
Write-Host '===== 1. Internet-Downloads'
$p1 = StartApp 8799 't1' @{} @('--browser', '--no-open')
$dl = "$root\dl1"
Api 8799 '/api/settings' @{ dir = $dl; speedLimitKB = 1500; clipboard = $false } | Out-Null
$ex = (Api 8799 '/api/state').extractor
Write-Host "Entpacker: $ex"
if ($ex -ne '7-Zip (eingebaut)') { throw "Eingebautes 7-Zip wird nicht verwendet: $ex" }
$big = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
$c = (Invoke-WebRequest "$big.sha256" -UseBasicParsing).Content
if ($c -is [byte[]]) { $c = [Text.Encoding]::ASCII.GetString($c) }
$want = ([string]$c).Trim().Split(' ')[0].ToLower()
Api 8799 '/api/grab' @{ text = "$big  sha256: $($want.ToUpper())`nhttps://github.com/rust-lang/rust/archive/refs/tags/1.0.0.zip https://gibt-es-nicht.invalid/x.zip"; pkg = 'Test' } | Out-Host
WaitFor 8799 { @(Items 8799 | Where-Object status -eq 'checking').Count -eq 0 } 60 'Links prüfen'
Show 8799
$ready = @(Items 8799 | Where-Object status -eq 'ready' | ForEach-Object id)
if ($ready.Count -ne 2) { throw "Erwartet: 2 Links bereit, gefunden: $($ready.Count)" }
if (@(Items 8799 | Where-Object status -eq 'offline').Count -ne 1) { throw 'Toter Link wurde nicht als offline erkannt' }
Api 8799 '/api/action' @{ action = 'confirm'; ids = $ready } | Out-Null
Start-Sleep 3
Write-Host '--- läuft (gedrosselt)'; Show 8799
$run = @(Items 8799 | Where-Object status -eq 'running' | ForEach-Object id)
if ($run.Count -eq 0) { throw 'Kein Download läuft' }
Api 8799 '/api/action' @{ action = 'pause'; ids = $run } | Out-Null
Start-Sleep 2
Write-Host '--- pausiert'; Show 8799
if (@(Items 8799 | Where-Object status -eq 'running').Count -ne 0) { throw 'Pause hat nicht gewirkt' }
if (Get-Process curl -ErrorAction SilentlyContinue) { throw 'curl läuft trotz Pause weiter' }
Api 8799 '/api/settings' @{ speedLimitKB = 0 } | Out-Null
Api 8799 '/api/action' @{ action = 'resume'; ids = $run } | Out-Null
WaitFor 8799 { $q = @(Items 8799 | Where-Object stage -eq 'queue'); $q.Count -eq 2 -and @($q | Where-Object status -ne 'done').Count -eq 0 } 180 'Downloads fertig'
$have = Sha "$dl\Test\rustup-init.exe"
Write-Host "SHA256 erwartet $want"; Write-Host "SHA256 erhalten $have"
if ($want -ne $have) { throw 'Prüfsumme stimmt nicht' }
WaitFor 8799 { @(Items 8799 | Where-Object { $_.name -eq 'rustup-init.exe' -and $_.shaState -eq 'ok' }).Count -eq 1 } 60 'SHA-256 aus der Linkzeile geprüft'
Write-Host 'SHA-256 aus der Linkzeile: übernommen und bestätigt'

WaitFor 8799 { @(Items 8799 | Where-Object { $_.name -like '*.zip' -and $_.extract -in 'ok', 'error' }).Count -eq 1 } 240 'ZIP entpacken'
Show 8799
if (-not (Test-Path "$dl\Test\rust-1.0.0\README.md")) { throw 'ZIP wurde nicht automatisch entpackt' }
if (Get-ChildItem $dl -Recurse -Filter *.part) { throw '.part-Datei übrig' }
$seven = Get-ChildItem "$root\t1\7zip" -Recurse -Filter 7z.exe | Select-Object -First 1
if (-not $seven) { throw 'Eingebautes 7-Zip wurde nicht ausgepackt' }
$info = (& $seven.FullName i) -join "`n"
if ($info -notmatch 'Rar5') { throw 'Eingebautes 7-Zip kann kein RAR' }
Write-Host "Eingebautes 7-Zip: $($seven.FullName) – RAR, 7z, ZIP vorhanden"
QuitApp 8799 $p1
Write-Host 'Teil 1 bestanden'

# ===== Lokaler Testserver: verschlüsseltes mehrteiliges 7z, ZIP, nachgestellte Rapidgator-Schnittstelle =====
New-Item -ItemType Directory "$root\www\api\v2\user", "$root\www\api\v2\file", "$root\src" -Force | Out-Null
$rnd = New-Object byte[] 3500000; (New-Object Random 7).NextBytes($rnd)
[IO.File]::WriteAllBytes("$root\src\daten.bin", $rnd)
'hallo' | Set-Content "$root\src\lies.txt"
$sz = (Get-Command 7z -ErrorAction SilentlyContinue).Source
if (-not $sz) { $sz = 'C:\Program Files\7-Zip\7z.exe' }
$have7z = Test-Path $sz
Write-Host "7-Zip vorhanden: $have7z ($sz)"
if ($have7z) {
  & $sz a -v1m -pgeheim -mhe=on "$root\www\geheim.7z" "$root\src\*" | Out-Null
  Get-ChildItem "$root\www" -Filter 'geheim.7z.*' | Format-Table Name, Length | Out-String | Write-Host
}
Compress-Archive "$root\src\*" "$root\www\plain.zip"
$zipSize = (Get-Item "$root\www\plain.zip").Length
$end = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() + 86400 * 30
[IO.File]::WriteAllText("$root\www\api\v2\user\login", "{`"response`":{`"token`":`"T1`",`"user`":{`"is_premium`":true,`"premium_end_time`":$end,`"traffic`":{`"left`":`"53687091200`"}}},`"status`":200,`"details`":null}")
[IO.File]::WriteAllText("$root\www\api\v2\file\info", "{`"response`":{`"file`":{`"name`":`"plain-rg.zip`",`"size`":$zipSize}},`"status`":200,`"details`":null}")
[IO.File]::WriteAllText("$root\www\api\v2\file\download", "{`"response`":{`"download_url`":`"http://127.0.0.1:9911/plain.zip?sess=1`"},`"status`":200,`"details`":null}")
$py = Start-Process python -ArgumentList '-m', 'http.server', '9911', '--bind', '127.0.0.1', '--directory', "$root\www" -PassThru -NoNewWindow -RedirectStandardOutput "$root\py.log" -RedirectStandardError "$root\py-err.log"
Start-Sleep 2

# ===== 2. Mehrteiliges Archiv mit Passwort, Konto, Zwischenablage =====
Write-Host '===== 2. Entpacken mit Passwort, Konto, Zwischenablage'
$rg = @{ LADEWERK_RG_API = 'http://127.0.0.1:9911/api/v2' }
$p2 = StartApp 8800 't2' $rg @('--browser', '--no-open')
$dl = "$root\dl2"
Api 8800 '/api/settings' @{ dir = $dl } | Out-Null
if ($have7z) {
  $urls = (Get-ChildItem "$root\www" -Filter 'geheim.7z.*' | ForEach-Object { "http://127.0.0.1:9911/$($_.Name)" }) -join ' '
  Api 8800 '/api/grab' @{ text = $urls } | Out-Host
  WaitFor 8800 { $g = @(Items 8800); $g.Count -ge 2 -and @($g | Where-Object status -ne 'ready').Count -eq 0 } 30 'Teile prüfen'
  if (@(Items 8800 | Where-Object pkg -ne 'geheim').Count -ne 0) { Show 8800; throw 'Automatisches Paket für Archivteile fehlt' }
  Api 8800 '/api/action' @{ action = 'confirm'; ids = @(Items 8800 | ForEach-Object id) } | Out-Null
  WaitFor 8800 { @(Items 8800 | Where-Object extract -eq 'error').Count -eq 1 } 90 'Entpacken ohne Passwort muss scheitern'
  Show 8800
  $first = @(Items 8800 | Where-Object extract -eq 'error')[0]
  if ($first.name -notlike '*.001' -or $first.extractMsg -notmatch 'asswor') { throw "Unerwartete Meldung: $($first.extractMsg)" }
  Api 8800 '/api/settings' @{ passwords = "falsch`ngeheim" } | Out-Null
  Api 8800 '/api/action' @{ action = 'extract'; ids = @($first.id) } | Out-Null
  WaitFor 8800 { @(Items 8800 | Where-Object extract -eq 'ok').Count -eq 1 } 90 'Entpacken mit Passwort'
  if ((Sha "$dl\geheim\daten.bin") -ne (Sha "$root\src\daten.bin")) { throw 'Entpackte Datei weicht ab' }
  Write-Host 'Mehrteiliges 7z mit Passwort: in Ordnung'
} else { $warn += '7-Zip fehlt auf diesem Testrechner – Passwort-/Mehrteil-Test übersprungen' }

Api 8800 '/api/account' @{ host = 'rapidgator.net'; user = 'oli@example.com'; password = 'p&w ö=1' } | Out-Null
$acc = @((Api 8800 '/api/state').accounts)[0]
Write-Host "Konto: $($acc.host) / $($acc.user) -> ok=$($acc.ok) $($acc.info)"
if ($acc.ok -ne $true -or $acc.info -notmatch 'Premium') { throw 'Kontoprüfung fehlgeschlagen' }
Start-Sleep 2
$stateText = Get-Content "$root\t2\state.json" -Raw
if ($stateText -notmatch '"secret":"dpapi:[0-9a-f]{40,}"') { throw 'Passwort ist nicht mit DPAPI geschützt abgelegt' }
if ($stateText -match 'p&w') { throw 'Passwort steht im Klartext in state.json' }
QuitApp 8800 $p2
$p2 = StartApp 8800 't2' $rg @('--browser', '--no-open')   # Neustart: Passwort muss wieder lesbar sein
Api 8800 '/api/grab' @{ text = 'https://rapidgator.net/file/abc123def/irgendwas.zip.html' } | Out-Null
WaitFor 8800 { @(Items 8800 | Where-Object { $_.name -eq 'plain-rg.zip' -and $_.status -eq 'ready' }).Count -eq 1 } 30 'Hoster-Link prüfen (Anmeldung nach Neustart)'
Api 8800 '/api/action' @{ action = 'confirm'; ids = @(Items 8800 | Where-Object name -eq 'plain-rg.zip' | ForEach-Object id) } | Out-Null
WaitFor 8800 { @(Items 8800 | Where-Object { $_.name -eq 'plain-rg.zip' -and $_.extract -eq 'ok' }).Count -eq 1 } 60 'Hoster-Download und Entpacken'
if ((Sha "$dl\plain-rg\daten.bin") -ne (Sha "$root\src\daten.bin")) { throw 'Hoster-Download weicht ab' }
Write-Host 'Konto (DPAPI) und Hoster-Ablauf: in Ordnung'

# falsche Prüfsumme: Datei bleibt, wird aber nicht entpackt
Api 8800 '/api/grab' @{ text = ('http://127.0.0.1:9911/plain.zip?falsch=1 ' + ('0' * 64)); pkg = 'Falsch' } | Out-Null
WaitFor 8800 { @(Items 8800 | Where-Object { $_.pkg -eq 'Falsch' -and $_.status -eq 'ready' }).Count -eq 1 } 30 'Link mit falscher Prüfsumme prüfen'
Api 8800 '/api/action' @{ action = 'confirm'; ids = @(Items 8800 | Where-Object pkg -eq 'Falsch' | ForEach-Object id) } | Out-Null
WaitFor 8800 { @(Items 8800 | Where-Object { $_.pkg -eq 'Falsch' -and $_.shaState -eq 'bad' }).Count -eq 1 } 60 'Falsche Prüfsumme erkennen'
Start-Sleep 2
$bad = @(Items 8800 | Where-Object pkg -eq 'Falsch')[0]
if ($bad.extract -ne '') { throw "Archiv mit falscher Prüfsumme wurde entpackt ($($bad.extract))" }
if ($bad.shaGot -ne (Sha "$root\www\plain.zip")) { throw 'Berechnete Prüfsumme weicht von Get-FileHash ab' }
# richtige Prüfsumme nachtragen -> wird geprüft und entpackt
Api 8800 '/api/hash' @{ id = $bad.id; sha256 = (Sha "$root\www\plain.zip") } | Out-Null
WaitFor 8800 { @(Items 8800 | Where-Object { $_.pkg -eq 'Falsch' -and $_.shaState -eq 'ok' -and $_.extract -eq 'ok' }).Count -eq 1 } 60 'Prüfsumme nachtragen, dann entpacken'
Write-Host 'Prüfsumme falsch/nachgetragen: in Ordnung'

# Passwort nur im Paket (nicht in der allgemeinen Liste)
if ($have7z) {
  Api 8800 '/api/settings' @{ passwords = '' } | Out-Null
  Copy-Item "$root\www\geheim.7z.001" "$root\www\paketpw.7z.001"; Get-ChildItem "$root\www" -Filter 'geheim.7z.0*' | Where-Object Name -ne 'geheim.7z.001' | ForEach-Object { Copy-Item $_.FullName ("$root\www\" + ($_.Name -replace '^geheim', 'paketpw')) }
  $urls = (Get-ChildItem "$root\www" -Filter 'paketpw.7z.*' | ForEach-Object { "http://127.0.0.1:9911/$($_.Name)" }) -join ' '
  Api 8800 '/api/pkgPasswords' @{ pkg = 'paketpw'; passwords = "falsch`ngeheim" } | Out-Null
  Api 8800 '/api/grab' @{ text = $urls } | Out-Null
  WaitFor 8800 { $g = @(Items 8800 | Where-Object pkg -eq 'paketpw'); $g.Count -ge 2 -and @($g | Where-Object status -ne 'ready').Count -eq 0 } 30 'Teile prüfen (Paket-Passwort)'
  Api 8800 '/api/action' @{ action = 'confirm'; ids = @(Items 8800 | Where-Object pkg -eq 'paketpw' | ForEach-Object id) } | Out-Null
  WaitFor 8800 { @(Items 8800 | Where-Object { $_.pkg -eq 'paketpw' -and $_.extract -eq 'ok' }).Count -eq 1 } 90 'Entpacken mit Paket-Passwort'
  Write-Host 'Passwort je Paket: in Ordnung'
}

# Click'n'Load: unverschlüsselt und verschlüsselt (CNL2, AES-128-CBC, IV = Schlüssel)
$cnlBefore = @(Items 8800).Count
$st = (Api 8800 '/api/state').cnl
Write-Host "Click'n'Load: $st"
if ($st -notmatch 'bereit') { throw "Click'n'Load nicht bereit: $st" }
if ((Invoke-WebRequest http://127.0.0.1:9666/jdcheck.js -UseBasicParsing).Content -notmatch 'jdownloader=true') { throw 'jdcheck.js fehlt' }
$r = Invoke-WebRequest -Method Post http://127.0.0.1:9666/flash/add -UseBasicParsing -ContentType 'application/x-www-form-urlencoded' -Body ('urls=' + [uri]::EscapeDataString("http://127.0.0.1:9911/cnl-a.zip`r`nhttp://127.0.0.1:9911/cnl-b.zip") + '&source=Test')
if ($r.Content -notmatch 'success') { throw "flash/add: $($r.Content)" }
$keyText = '1234567890987654'; $key = [Text.Encoding]::ASCII.GetBytes($keyText)
$aes = [Security.Cryptography.Aes]::Create(); $aes.Mode = 'CBC'; $aes.Padding = 'Zeros'; $aes.Key = $key; $aes.IV = $key
$plain = [Text.Encoding]::UTF8.GetBytes("http://127.0.0.1:9911/cnl-c.zip`r`nhttp://127.0.0.1:9911/cnl-d.rar")
$crypted = [Convert]::ToBase64String($aes.CreateEncryptor().TransformFinalBlock($plain, 0, $plain.Length))
$hex = -join ($key | ForEach-Object { $_.ToString('x2') })
$body = 'crypted=' + [uri]::EscapeDataString($crypted) + '&jk=' + [uri]::EscapeDataString("function f(){ return '$hex';}") + '&passwords=cnlpasswort&package=CNL-Paket'
$r = Invoke-WebRequest -Method Post http://127.0.0.1:9666/flash/addcrypted2 -UseBasicParsing -ContentType 'application/x-www-form-urlencoded' -Body $body
if ($r.Content -notmatch 'success') { throw "flash/addcrypted2: $($r.Content)" }
Start-Sleep 2
$cnl = @(Items 8800 | Where-Object url -like '*cnl-*')
$cnl | Format-Table pkg, url, status | Out-String | Write-Host
if ($cnl.Count -ne 4) { throw "Click'n'Load: $($cnl.Count) statt 4 Links angekommen" }
if (@($cnl | Where-Object pkg -eq 'CNL-Paket').Count -ne 2) { throw "Click'n'Load: Paketname fehlt" }
if ((Api 8800 '/api/state').pkgPasswords.'CNL-Paket' -notcontains 'cnlpasswort') { throw "Click'n'Load: Passwort nicht ins Paket übernommen" }
Write-Host "Click'n'Load: in Ordnung"

try {
  Set-Clipboard 'Schau mal: http://127.0.0.1:9911/clip-test.zip und https://example.com/seite'
  Start-Sleep 5
  $clip = @(Items 8800 | Where-Object url -like '*clip-test.zip')
  if ($clip.Count -eq 1 -and @(Items 8800 | Where-Object url -like '*example.com*').Count -eq 0) { Write-Host 'Zwischenablage: in Ordnung' }
  else { $warn += 'Zwischenablage: kopierter Link kam nicht im Linksammler an' }
} catch { $warn += "Zwischenablage nicht testbar: $_" }
Show 8800
QuitApp 8800 $p2
Write-Host 'Teil 2 bestanden'

# ===== 3. Entpacken nur mit Windows-Bordmitteln (tar.exe) =====
Write-Host '===== 3. Entpacken ohne 7-Zip'
$p3 = StartApp 8801 't3' @{ LADEWERK_EXTRACTOR = 'tar' } @('--browser', '--no-open')
$dl = "$root\dl3"
Api 8801 '/api/settings' @{ dir = $dl; clipboard = $false } | Out-Null
Write-Host "Entpacker: $((Api 8801 '/api/state').extractor)"
try { Api 8801 '/api/settings' @{ proxy = 'quatsch' } | Out-Null; throw 'Ungültiger Proxy wurde angenommen' } catch { if ("$_" -match 'angenommen') { throw } }
Api 8801 '/api/settings' @{ proxy = 'http://127.0.0.1:9' } | Out-Null
Api 8801 '/api/grab' @{ text = 'http://127.0.0.1:9911/proxy-test.zip' } | Out-Null
WaitFor 8801 { @(Items 8801 | Where-Object { $_.url -like '*proxy-test*' -and $_.status -eq 'offline' }).Count -eq 1 } 30 'Prüfen über Proxy'
$px = @(Items 8801 | Where-Object url -like '*proxy-test*')[0]
Write-Host "Über toten Proxy: $($px.error)"
if ($px.error -match '404') { throw 'Proxy wurde nicht benutzt' }
Api 8801 '/api/settings' @{ proxy = '' } | Out-Null
Api 8801 '/api/action' @{ action = 'recheck'; ids = @($px.id) } | Out-Null
WaitFor 8801 { @(Items 8801 | Where-Object { $_.url -like '*proxy-test*' -and $_.status -eq 'offline' -and $_.error -match '404' }).Count -eq 1 } 30 'Prüfen ohne Proxy'
Api 8801 '/api/clear' @{ what = 'offline' } | Out-Null
Write-Host 'Proxy: in Ordnung'
Api 8801 '/api/grab' @{ text = 'http://127.0.0.1:9911/plain.zip' } | Out-Null
WaitFor 8801 { @(Items 8801 | Where-Object status -eq 'ready').Count -eq 1 } 30 'ZIP prüfen'
Api 8801 '/api/action' @{ action = 'confirm'; ids = @(Items 8801 | ForEach-Object id) } | Out-Null
WaitFor 8801 { @(Items 8801 | Where-Object extract -in 'ok', 'error').Count -eq 1 } 60 'ZIP mit tar.exe entpacken'
Show 8801
if ((Sha "$dl\plain\daten.bin") -ne (Sha "$root\src\daten.bin")) { throw 'tar.exe-Entpacken weicht ab' }
QuitApp 8801 $p3
Write-Host 'Teil 3 bestanden'

# ===== 4. Eigenes Fenster (egui) =====
Write-Host '===== 4. Fenster'
$p4 = StartApp 8802 't2' @{} @()
Start-Sleep 10
$p4.Refresh()
$errLog = Get-Content "$root\app-8802-err.log" -Raw -ErrorAction SilentlyContinue
if ($errLog -match 'Fenster-Fehler') {
  $warn += "Fenster: kein OpenGL auf diesem Rechner – Ladewerk zeigt die Fehlermeldung mit Hinweis auf --browser ($($errLog.Trim()))"
  Stop-Process -Id $p4.Id -Force -ErrorAction SilentlyContinue
} elseif ($p4.HasExited) {
  $warn += "Fenster: Ladewerk hat sich sofort beendet (Exit $($p4.ExitCode)) – auf GitHub-Rechnern fehlt oft OpenGL"
} else {
  Write-Host "Fenstertitel: '$($p4.MainWindowTitle)'"
  try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save("$root\fenster.png")
  } catch { $warn += "Bildschirmfoto nicht möglich: $_" }
  if ($p4.MainWindowTitle -notlike '*Ladewerk*') { $warn += "Fenster: Titel «Ladewerk» nicht gefunden ('$($p4.MainWindowTitle)')" }
  # Strg+V im Fenster (Link ohne Dateiendung, damit nicht die Zwischenablage-Beobachtung greift)
  try {
    Add-Type -AssemblyName System.Windows.Forms
    Set-Clipboard 'Bitte laden: http://127.0.0.1:9911/strgv-windows'
    $ws = New-Object -ComObject WScript.Shell
    $ws.AppActivate($p4.Id) | Out-Null
    Start-Sleep 1
    [System.Windows.Forms.SendKeys]::SendWait('^v')
    Start-Sleep 4
    if (@(Items 8802 | Where-Object url -like '*strgv-windows*').Count -eq 1) { Write-Host 'Strg+V im Fenster: in Ordnung' }
    else { $warn += 'Strg+V im Fenster: Link kam nicht im Linksammler an' }
  } catch { $warn += "Strg+V nicht testbar: $_" }
  # zweiter Start: darf kein zweites Ladewerk werden
  $dup = Start-Process $exe -ArgumentList '--port=8802' -PassThru
  Start-Sleep 3
  if (-not $dup.HasExited) { Get-Process | Where-Object { $_.MainWindowTitle -eq 'Ladewerk' -and $_.Id -eq $dup.Id } | Out-Null; Write-Host 'Hinweisfenster «läuft bereits» offen – schliesse es'; Stop-Process -Id $dup.Id -Force }
  $p4.CloseMainWindow() | Out-Null
  if ($p4.WaitForExit(20000)) { Write-Host 'Ladewerk hat sich mit dem Fenster beendet' }
  else { $warn += 'Fenster: Ladewerk lief nach dem Schliessen weiter'; Stop-Process -Id $p4.Id -Force }
}
Stop-Process -Id $py.Id -Force -ErrorAction SilentlyContinue

foreach ($w in $warn) { Write-Host "WARNUNG: $w"; Write-Host "::warning::$w" }
Write-Host 'ALLE TESTS BESTANDEN'
