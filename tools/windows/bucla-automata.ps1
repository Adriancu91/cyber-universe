<#
  Bucla automata Cyber Universe (P6-41, comanda C-53) - scriptul pentru Windows de pe workstation.

  Ce face (pe scurt, pentru Adrian):
    .\bucla-automata.ps1 pregateste   -> pune versiunea noua a sistemului pe stick (pastreaza kronos\, firmware\, cyber.rw),
                                         scrie lista de teste auto\teste.txt, cere o singura pornire de pe stick, reporneste.
    .\bucla-automata.ps1 citeste      -> dupa revenirea in Windows: ia rapoartele auto\raport-*.txt de pe stick si le pune
                                         intr-un dosar, gata de trimis in hub (rezultate\).

  Reguli (DECIZII 1 nov 19:45):
    - lista de teste DOAR citeste; sistemul refuza singur orice scriere in placa video in modul automat;
    - doar ziua (implicit 08:00-22:00), nu in noptile de antrenare Kronos;
    - dreptul de administrator e cerut doar pentru "porneste o data de pe stick" (bcdedit) si repornire;
    - scriptul nu contine si nu cere parole, chei sau tokenuri.

  Exemple:
    .\bucla-automata.ps1 pregateste -DosarRelease C:\cyber\os-213
    .\bucla-automata.ps1 pregateste -DosarRelease C:\cyber\os-213 -Teste C:\cyber\teste.txt -FaraRepornire
    .\bucla-automata.ps1 citeste
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet('pregateste', 'citeste')]
    [string]$Actiune,

    # Dosarul cu fisierele Release-ului (descarcate de pe GitHub -> Releases): BOOTX64.EFI, kernel, initrd.tar,
    # limine.conf, limine-uefi-cd.bin, VERSIUNE, SHA256SUMS. Gol = se pastreaza sistemul de pe stick.
    [string]$DosarRelease = '',

    # Lista de teste (un rand = o comanda). Gol = lista implicita de mai jos.
    [string]$Teste = '',

    # Identificatorul de firmware al stick-ului ({...}), daca scriptul gaseste mai multe intrari USB in `bcdedit /enum firmware`.
    [string]$IdStick = '',

    # Unde pune `citeste` rapoartele.
    [string]$DosarIesire = (Join-Path $HOME 'cyber-bucla\rapoarte'),

    # Nu reporneste (doar pregateste stick-ul si pornirea o data).
    [switch]$FaraRepornire,

    # Ruleaza si in afara orelor 08:00-22:00 (doar daca Adrian a spus ca nu e antrenare in noaptea asta).
    [switch]$SiNoaptea
)

$ErrorActionPreference = 'Stop'

# Lista implicita: doar citiri (viteza Kronos, citiri din placa video, raportul).
$ListaImplicita = @'
# lista bucla automata (doar citire); un rand = o comanda; # = comentariu
ceas 15
hw
lspci
gpu identifica
gpu gfw
gpu vbios
gpu fwsec
gpu firmware
gpu pregatire-i6
cpu porneste
run kronos --control
run kronos --profil --fara-ecran --n 64 Buna ziua
raport
'@

function Scrie([string]$text) { Write-Host "[bucla] $text" }
function Opreste([string]$text) { Write-Host "[bucla] OPRIT: $text" -ForegroundColor Red; exit 1 }

# Stick-ul Cyber Universe: un volum FAT32 cu boot\kernel si cyber.rw in radacina. Trebuie sa fie exact unul.
function Gaseste-Stick {
    $gasite = @(Get-Volume | Where-Object {
        $_.DriveLetter -and $_.FileSystem -eq 'FAT32' -and
        (Test-Path "$($_.DriveLetter):\boot\kernel") -and (Test-Path "$($_.DriveLetter):\cyber.rw")
    })
    if ($gasite.Count -eq 0) { Opreste 'nu gasesc stick-ul Cyber Universe (FAT32 cu boot\kernel si cyber.rw). E bagat?' }
    if ($gasite.Count -gt 1) { Opreste ('mai multe stick-uri Cyber Universe: ' + (($gasite | ForEach-Object { "$($_.DriveLetter):" }) -join ', ') + '; lasa unul singur') }
    return "$($gasite[0].DriveLetter):"
}

function Este-Administrator {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    return (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Verifica sumele SHA-256 din SHA256SUMS pentru fisierele pe care le copiem.
function Verifica-Release([string]$dosar, [string[]]$fisiere) {
    $sume = Join-Path $dosar 'SHA256SUMS'
    if (-not (Test-Path $sume)) { Opreste "lipseste $sume (descarca tot Release-ul)" }
    $asteptat = @{}
    foreach ($rand in Get-Content $sume) {
        $parti = $rand -split '\s+', 2
        if ($parti.Count -eq 2) { $asteptat[$parti[1].Trim().TrimStart('*')] = $parti[0].ToLower() }
    }
    foreach ($f in $fisiere) {
        $cale = Join-Path $dosar $f
        if (-not (Test-Path $cale)) { Opreste "lipseste $cale" }
        if (-not $asteptat.ContainsKey($f)) { Opreste "$f nu e in SHA256SUMS" }
        $suma = (Get-FileHash -Algorithm SHA256 $cale).Hash.ToLower()
        if ($suma -ne $asteptat[$f]) { Opreste "suma SHA-256 gresita la $f (fisier stricat sau din alta versiune)" }
    }
    Scrie "Release verificat: $($fisiere.Count) fisiere, sumele SHA-256 bune"
}

function Copiaza-Release([string]$dosar, [string]$stick) {
    # Varianta B din docs/USB.md: fisierele in dosarele lor; restul stick-ului (kronos\, firmware\, raport\, cyber.rw) ramane.
    $harta = [ordered]@{
        'BOOTX64.EFI'        = 'EFI\BOOT\BOOTX64.EFI'
        'kernel'             = 'boot\kernel'
        'initrd.tar'         = 'boot\initrd.tar'
        'limine.conf'        = 'boot\limine\limine.conf'
        'limine-uefi-cd.bin' = 'boot\limine\limine-uefi-cd.bin'
        'VERSIUNE'           = 'VERSIUNE'
    }
    Verifica-Release $dosar @($harta.Keys)
    foreach ($f in $harta.Keys) {
        $tinta = Join-Path $stick $harta[$f]
        New-Item -ItemType Directory -Force -Path (Split-Path $tinta) | Out-Null
        Copy-Item -Force (Join-Path $dosar $f) $tinta
    }
    if (-not (Test-Path "$stick\cyber.rw") -and (Test-Path (Join-Path $dosar 'cyber.rw'))) {
        Copy-Item (Join-Path $dosar 'cyber.rw') "$stick\cyber.rw"
    }
    $versiune = (Get-Content "$stick\VERSIUNE" -TotalCount 1)
    Scrie "sistemul nou e pe stick ($versiune); kronos\, firmware\ si cyber.rw au ramas neatinse"
}

# Intrarea de firmware a stick-ului, pentru `bootsequence` (pornire o singura data).
function Gaseste-IntrareStick {
    if ($IdStick) { return $IdStick }
    $text = (& bcdedit /enum firmware) -join "`n"
    $blocuri = $text -split "`n\s*`n"
    $usb = @()
    foreach ($b in $blocuri) {
        # bcdedit vorbeste limba Windows-ului (identifier/identificator, description/descriere): cautam doar forma valorilor.
        $id = [regex]::Match($b, '\{[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\}').Value
        $primul = [regex]::Match($b, '\{[^}]+\}').Value
        if (-not $id -or $primul -ne $id) { continue }   # blocul {fwbootmgr} incepe cu {fwbootmgr}, nu cu un GUID
        if ($b -match 'Windows Boot Manager' -or $b -notmatch 'USB|UEFI:|Removable') { continue }
        $desc = (($b -split "`n") | Where-Object { $_ -match 'USB|UEFI:|Removable' } | Select-Object -First 1).Trim()
        $usb += [pscustomobject]@{ Id = $id; Descriere = $desc }
    }
    if ($usb.Count -ne 1) {
        Scrie 'intrarile de firmware gasite (bcdedit /enum firmware):'
        $usb | ForEach-Object { Scrie "  $($_.Id)  $($_.Descriere)" }
        Opreste 'nu e clar care intrare e stick-ul; ruleaza din nou cu -IdStick {...} (o singura data, cu Adrian)'
    }
    Scrie "intrarea stick-ului: $($usb[0].Id) ($($usb[0].Descriere))"
    return $usb[0].Id
}

function Pregateste {
    $ora = (Get-Date).Hour
    if (-not $SiNoaptea -and ($ora -lt 8 -or $ora -ge 22)) { Opreste "e $((Get-Date).ToString('HH:mm')): bucla ruleaza doar ziua (08:00-22:00), nu in noptile de antrenare" }
    if (-not (Este-Administrator)) { Opreste 'trebuie pornit ca administrator (doar pentru pornirea o data de pe stick si repornire)' }

    $stick = Gaseste-Stick
    Scrie "stick-ul: $stick"
    if ($DosarRelease) { Copiaza-Release $DosarRelease $stick } else { Scrie 'fara -DosarRelease: sistemul de pe stick ramane cel de acum' }

    $auto = Join-Path $stick 'auto'
    New-Item -ItemType Directory -Force -Path $auto | Out-Null
    # Un marcaj ramas de la o rulare intrerupta ar face sistemul sa trateze lista noua ca intrerupta: il mutam deoparte.
    $marcaj = Join-Path $auto 'in-lucru.txt'
    if (Test-Path $marcaj) {
        $vechi = "in-lucru-vechi-$((Get-Date).ToString('yyyyMMdd-HHmm')).txt"
        Rename-Item $marcaj $vechi
        Scrie "ATENTIE: rularea trecuta n-a terminat (marcajul mutat in auto\$vechi); vezi raportul ei cu: bucla-automata.ps1 citeste"
    }
    if (Test-Path (Join-Path $auto 'teste.txt')) { Scrie 'ATENTIE: o lista veche nerulata (auto\teste.txt) se inlocuieste' }

    $lista = if ($Teste) { Get-Content -Raw $Teste } else { $ListaImplicita }
    foreach ($rand in ($lista -split "`r?`n")) {
        $r = ($rand -split '#')[0].Trim()
        if ($r -match '--scrie' -or $r -match '^gpu\s+i6\b' -or $r -match '^reboot\b') { Opreste "lista are o comanda care nu e voie in modul automat: $r" }
    }
    # ASCII cu sfarsit de rand LF: sistemul citeste textul simplu.
    [IO.File]::WriteAllText((Join-Path $auto 'teste.txt'), ($lista -replace "`r`n", "`n"), [Text.Encoding]::ASCII)
    Scrie "lista scrisa: $auto\teste.txt"

    $id = Gaseste-IntrareStick
    & bcdedit /set '{fwbootmgr}' bootsequence $id | Out-Host
    if ($LASTEXITCODE -ne 0) { Opreste 'bcdedit nu a putut seta pornirea o data de pe stick' }
    Scrie 'pornirea urmatoare: o singura data de pe stick; dupa ea, calculatorul revine singur in Windows'

    if ($FaraRepornire) { Scrie 'fara repornire (-FaraRepornire): reporneste cand vrei'; return }
    Scrie 'repornesc in 15 secunde (Ctrl+C ca sa opresti)'
    Start-Sleep -Seconds 15
    Restart-Computer -Force
}

function Citeste {
    $stick = Gaseste-Stick
    $auto = Join-Path $stick 'auto'
    if (-not (Test-Path $auto)) { Opreste "pe stick nu exista dosarul auto\ (nicio rulare)" }
    New-Item -ItemType Directory -Force -Path $DosarIesire | Out-Null
    $citite = Join-Path $DosarIesire 'citite.txt'
    $deja = @(); if (Test-Path $citite) { $deja = @(Get-Content $citite) }
    $noi = @(Get-ChildItem $auto -Filter 'raport-*.txt' | Where-Object { $deja -notcontains $_.Name } | Sort-Object Name)
    if ($noi.Count -eq 0) { Scrie 'niciun raport nou pe stick'; return }
    foreach ($r in $noi) {
        # Numele pentru hub: AAAA-LL-ZZ_HHMM_cyber-universe_auto.txt (din numele raportului: raport-AAAALLZZ-HHMM.txt).
        $m = [regex]::Match($r.Name, 'raport-(\d{4})(\d{2})(\d{2})-(\d{4})')
        $nume = if ($m.Success) { "$($m.Groups[1].Value)-$($m.Groups[2].Value)-$($m.Groups[3].Value)_$($m.Groups[4].Value)_cyber-universe_auto.txt" } else { "cyber-universe_auto_$($r.Name)" }
        Copy-Item $r.FullName (Join-Path $DosarIesire $nume)
        $rezultat = Select-String -Path $r.FullName -Pattern '^REZULTAT:|== INTRERUPT ==' | Select-Object -Last 1
        $text = if ($rezultat) { $rezultat.Line } else { 'fara REZULTAT (rularea nu a terminat?)' }
        Scrie "$nume  ->  $text"
        Add-Content $citite $r.Name
    }
    Scrie "rapoartele sunt in $DosarIesire (de pus in hub, la rezultate\)"
}

switch ($Actiune) {
    'pregateste' { Pregateste }
    'citeste' { Citeste }
}
