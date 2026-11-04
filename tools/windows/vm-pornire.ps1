<#
  Masina virtuala Cyber Universe (P6-48, comanda C-68, F1) - scriptul pentru Windows de pe workstation.

  Ce face (pe scurt, pentru Adrian):
    .\vm-pornire.ps1 porneste     -> porneste Cyber Universe intr-o masina virtuala QEMU (discul cyber-universe-vm.qcow2)
    .\vm-pornire.ps1 opreste      -> opreste masina (QEMU inchis curat prin monitorul lui)
    .\vm-pornire.ps1 reporneste   -> opreste + porneste (jurnal serial nou)
    .\vm-pornire.ps1 stare        -> e pornita? ultimele randuri din jurnalul serial
    .\vm-pornire.ps1 supraveghe   -> ceasul de siguranta: porneste masina si o reporneste daca se opreste singura, scrie
                                     PANIC sau nu mai raspunde (cel mult 3 reporniri pe ora, apoi se opreste si noteaza)
    .\vm-pornire.ps1 disc-nou     -> pune discul dintr-un Release nou (cel vechi pastrat ca .vechi); doar cu masina oprita

  Reguli (C-68, C-70):
    - masina primeste cel mult -Nuclee procesoare (implicit 8) si -MemorieGB (implicit 16); restul ramane lui Kronos;
    - placa video NU e data masinii (nicio optiune de "passthrough" in acest script);
    - scriptul nu contine si nu cere parole, chei sau tokenuri; nu are nevoie de drept de administrator.

  Pornire din PowerShell (o singura data, daca Windows refuza scripturile):
    powershell -ExecutionPolicy Bypass -File .\vm-pornire.ps1 porneste

  Exemple:
    .\vm-pornire.ps1 porneste
    .\vm-pornire.ps1 porneste -Nuclee 4 -MemorieGB 8 -FaraEcran
    .\vm-pornire.ps1 porneste -Accel whpx        (mai rapid, cere "Windows Hypervisor Platform"; de incercat cu Adrian)
    .\vm-pornire.ps1 supraveghe -FaraEcran
    .\vm-pornire.ps1 disc-nou -DiscNou C:\cyber\os-230\cyber-universe-vm.qcow2
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('porneste', 'opreste', 'reporneste', 'stare', 'supraveghe', 'disc-nou')]
    [string]$Comanda = 'stare',

    # Dosarul masinii: discul, variabilele UEFI, jurnalele.
    [string]$Dosar = (Join-Path $HOME 'CyberUniverseVM'),

    # qemu-system-x86_64(.exe). Gol = il caut (C:\Program Files\qemu, apoi PATH).
    [string]$Qemu = '',

    # Firmware-ul UEFI (edk2-x86_64-code.fd). Gol = il caut langa QEMU.
    [string]$Firmware = '',
    [string]$FirmwareVars = '',

    [ValidateRange(1, 16)] [int]$Nuclee = 8,
    [ValidateRange(1, 32)] [int]$MemorieGB = 16,

    # tcg = emulare (merge oriunde, mai lent); whpx = accelerarea Windows (cu revenire la tcg daca nu merge).
    [ValidateSet('tcg', 'whpx')] [string]$Accel = 'tcg',

    # Fara fereastra (masina se vede doar prin jurnalul serial).
    [switch]$FaraEcran,

    # Portul monitorului QEMU (QMP), doar pe 127.0.0.1.
    [ValidateRange(1024, 65535)] [int]$PortQmp = 4444,

    # P6-49 (F2): consola remote a OS-ului (portul 2323 din masina) se vede pe Windows la 127.0.0.1:<PortRemote>.
    [ValidateRange(1024, 65535)] [int]$PortRemote = 2323,

    # supraveghe: secunde intre verificari; minute pana la oprire (0 = fara limita).
    [ValidateRange(2, 3600)] [int]$IntervalSecunde = 30,
    [ValidateRange(0, 100000)] [int]$MinuteMaxim = 0,

    # disc-nou: discul din Release. Gol = cyber-universe-vm.qcow2 de langa acest script.
    [string]$DiscNou = ''
)
Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

$NumeDisc = 'cyber-universe-vm.qcow2'
$Disc = Join-Path $Dosar $NumeDisc
$Jurnale = Join-Path $Dosar 'jurnal'
$FisierPid = Join-Path $Dosar 'masina.pid'
$FisierSerial = Join-Path $Dosar 'serial-curent.txt'   # numele jurnalului serial al pornirii curente
$PeWindows = ($env:OS -eq 'Windows_NT')

function Scrie([string]$Text) {
    $rand = '{0}  {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Text
    Write-Host $rand
    if (Test-Path $Jurnale) { Add-Content -Path (Join-Path $Jurnale 'vm.txt') -Value $rand }
}

function Gaseste-Qemu {
    if ($Qemu) {
        if (-not (Test-Path $Qemu)) { throw "Nu gasesc QEMU la $Qemu" }
        return (Resolve-Path $Qemu).Path
    }
    $candidati = @()
    if ($PeWindows) {
        foreach ($baza in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, 'C:\qemu')) {
            if ($baza) { $candidati += (Join-Path $baza 'qemu\qemu-system-x86_64.exe'); $candidati += (Join-Path $baza 'qemu-system-x86_64.exe') }
        }
    }
    foreach ($c in $candidati) { if (Test-Path $c) { return $c } }
    $cmd = Get-Command 'qemu-system-x86_64' -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    throw 'Nu gasesc QEMU. Instaleaza-l de pe qemu.org (Windows) sau da calea cu -Qemu.'
}

function Gaseste-Firmware([string]$qemuExe) {
    if ($Firmware) {
        if (-not (Test-Path $Firmware)) { throw "Nu gasesc firmware-ul la $Firmware" }
        $v = $FirmwareVars
        if (-not $v) { $v = Join-Path (Split-Path $Firmware) 'edk2-i386-vars.fd' }
        if (-not (Test-Path $v)) { throw "Nu gasesc variabilele UEFI la $v (da -FirmwareVars)" }
        return @($Firmware, $v)
    }
    $dir = Split-Path $qemuExe
    $perechi = @(
        @((Join-Path $dir 'share\edk2-x86_64-code.fd'), (Join-Path $dir 'share\edk2-i386-vars.fd')),
        @((Join-Path $dir 'edk2-x86_64-code.fd'), (Join-Path $dir 'edk2-i386-vars.fd')),
        @('/usr/share/OVMF/OVMF_CODE_4M.fd', '/usr/share/OVMF/OVMF_VARS_4M.fd'),
        @('/usr/share/OVMF/OVMF_CODE.fd', '/usr/share/OVMF/OVMF_VARS.fd'),
        @('/usr/share/qemu/edk2-x86_64-code.fd', '/usr/share/qemu/edk2-i386-vars.fd'),
        @('/usr/share/edk2/x64/OVMF_CODE.4m.fd', '/usr/share/edk2/x64/OVMF_VARS.4m.fd')
    )
    foreach ($p in $perechi) { if ((Test-Path $p[0]) -and (Test-Path $p[1])) { return $p } }
    throw 'Nu gasesc firmware-ul UEFI (edk2-x86_64-code.fd) langa QEMU. Da calea cu -Firmware.'
}

function Proces-Masina {
    if (-not (Test-Path $FisierPid)) { return $null }
    $idProces = 0
    if (-not [int]::TryParse((Get-Content $FisierPid -TotalCount 1), [ref]$idProces)) { return $null }
    $p = Get-Process -Id $idProces -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -like 'qemu-system*') { return $p }
    return $null
}

# Sends one QMP command and returns the parsed answer (or $null when QEMU does not answer).
function Qmp([string]$Executa) {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $incercare = $client.BeginConnect('127.0.0.1', $PortQmp, $null, $null)
        if (-not $incercare.AsyncWaitHandle.WaitOne(3000)) { return $null }
        $client.EndConnect($incercare)
        $flux = $client.GetStream()
        $flux.ReadTimeout = 5000
        $cititor = New-Object System.IO.StreamReader($flux)
        $scriitor = New-Object System.IO.StreamWriter($flux)
        $scriitor.AutoFlush = $true
        [void]$cititor.ReadLine()                                  # the QMP greeting
        $scriitor.WriteLine('{"execute":"qmp_capabilities"}')
        [void](Citeste-Raspuns $cititor)
        $scriitor.WriteLine('{"execute":"' + $Executa + '"}')
        return (Citeste-Raspuns $cititor)
    } catch {
        return $null
    } finally {
        $client.Close()
    }
}

function Citeste-Raspuns($cititor) {
    for ($i = 0; $i -lt 50; $i++) {
        $rand = $cititor.ReadLine()
        if ($null -eq $rand) { return $null }
        $obiect = $rand | ConvertFrom-Json
        if ($obiect.PSObject.Properties.Name -contains 'return' -or $obiect.PSObject.Properties.Name -contains 'error') { return $obiect }
        # events ("event": ...) are skipped
    }
    return $null
}

function Stare-Qemu {
    $r = Qmp 'query-status'
    if ($null -eq $r -or -not ($r.PSObject.Properties.Name -contains 'return')) { return '' }
    return [string]$r.return.status
}

function Argumente([string]$serial) {
    $fw = Gaseste-Firmware $script:QemuExe
    $vars = Join-Path $Dosar 'vars.fd'
    if (-not (Test-Path $vars)) { Copy-Item $fw[1] $vars }
    $a = @(
        '-name', 'Cyber Universe',
        '-M', 'q35',
        '-smp', "$Nuclee",
        '-m', "$($MemorieGB)G",
        '-drive', "if=pflash,unit=0,format=raw,readonly=on,file=$($fw[0])",
        '-drive', "if=pflash,unit=1,format=raw,file=$vars",
        # The VM disk is seen as a USB stick, so /usb, `actualizeaza` and the reports behave as on real hardware.
        '-device', 'qemu-xhci,id=xhci',
        '-drive', "if=none,id=disc,format=qcow2,file=$Disc",
        '-device', 'usb-storage,bus=xhci.0,drive=disc,bootindex=0',
        '-device', 'usb-kbd,bus=xhci.0',
        # Network: the e1000e card (driver in the OS), QEMU "user" network = internet through Windows.
        # The OS's remote console (port 2323, P6-49) is forwarded to 127.0.0.1 only: not visible from the network.
        '-nic', 'none',
        '-netdev', "user,id=n0,hostfwd=tcp:127.0.0.1:$($PortRemote)-:2323",
        '-device', 'e1000e,netdev=n0',
        '-serial', "file:$serial",
        '-qmp', "tcp:127.0.0.1:$($PortQmp),server=on,wait=off"
    )
    if ($Accel -eq 'whpx') { $a += @('-accel', 'whpx,kernel-irqchip=off', '-accel', 'tcg,thread=multi') }
    else { $a += @('-accel', 'tcg,thread=multi') }
    if ($FaraEcran) { $a += @('-display', 'none') }
    return $a
}

function Citeaza([string]$s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

function Porneste {
    $p = Proces-Masina
    if ($p) { Scrie "Masina e deja pornita (QEMU, proces $($p.Id))."; return }
    if (-not (Test-Path $Disc)) {
        $langa = Join-Path $PSScriptRoot $NumeDisc
        if (Test-Path $langa) { Copy-Item $langa $Disc; Scrie "Discul copiat din $langa" }
        else { throw "Lipseste discul $Disc. Pune langa script fisierul $NumeDisc din Release (sau ruleaza disc-nou)." }
    }
    $script:QemuExe = Gaseste-Qemu
    $serial = Join-Path $Jurnale ('serial-{0}.txt' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
    $a = Argumente $serial
    $err = Join-Path $Jurnale 'qemu-erori.txt'
    $out = Join-Path $Jurnale 'qemu-iesire.txt'
    $linie = ($a | ForEach-Object { Citeaza $_ }) -join ' '
    Set-Content -Path (Join-Path $Jurnale 'ultima-comanda.txt') -Value ("$(Citeaza $script:QemuExe) $linie")
    $proc = Start-Process -FilePath $script:QemuExe -ArgumentList $linie -PassThru -RedirectStandardError $err -RedirectStandardOutput $out
    Set-Content -Path $FisierPid -Value $proc.Id
    Set-Content -Path $FisierSerial -Value $serial
    Start-Sleep -Seconds 2
    if ($proc.HasExited) {
        $mesaj = ''
        if (Test-Path $err) { $mesaj = (Get-Content $err -Raw) }
        Remove-Item $FisierPid -ErrorAction SilentlyContinue
        throw "QEMU s-a oprit imediat. $mesaj"
    }
    Scrie "Masina pornita: proces $($proc.Id), $Nuclee nuclee, $MemorieGB GB, accelerare $Accel, jurnal serial $serial"
}

function Opreste {
    $p = Proces-Masina
    if (-not $p) { Scrie 'Masina nu e pornita.'; Remove-Item $FisierPid -ErrorAction SilentlyContinue; return }
    [void](Qmp 'quit')
    if (-not $p.WaitForExit(15000)) {
        Scrie 'QEMU nu a raspuns la oprire in 15 s: il opresc fortat.'
        Stop-Process -Id $p.Id -Force
        [void]$p.WaitForExit(5000)
    }
    Remove-Item $FisierPid -ErrorAction SilentlyContinue
    Scrie 'Masina oprita.'
}

function Serial-Curent {
    if (-not (Test-Path $FisierSerial)) { return '' }
    return [string](Get-Content $FisierSerial -TotalCount 1)
}

function Stare {
    $p = Proces-Masina
    if (-not $p) { Write-Host 'stare: oprita' }
    else {
        $s = Stare-Qemu
        if (-not $s) { $s = 'nu raspunde' }
        Write-Host "stare: pornita (proces $($p.Id), QEMU: $s)"
    }
    $serial = Serial-Curent
    if ($serial -and (Test-Path $serial)) {
        Write-Host "jurnal serial: $serial"
        Get-Content $serial -Tail 8 | ForEach-Object { Write-Host "  $_" }
    }
}

function Contine-Panic([string]$serial, [ref]$pozitie) {
    if (-not $serial -or -not (Test-Path $serial)) { return $false }
    $lungime = (Get-Item $serial).Length
    if ($lungime -lt $pozitie.Value) { $pozitie.Value = 0 }
    if ($lungime -eq $pozitie.Value) { return $false }
    $flux = [System.IO.File]::Open($serial, 'Open', 'Read', 'ReadWrite')
    try {
        [void]$flux.Seek($pozitie.Value, 'Begin')
        $cititor = New-Object System.IO.StreamReader($flux)
        $nou = $cititor.ReadToEnd()
    } finally { $flux.Close() }
    $pozitie.Value = $lungime
    return ($nou -match 'PANIC:')
}

function Supraveghe {
    $notaSupraveghere = Join-Path $Jurnale 'supraveghere.txt'
    $reporniri = New-Object System.Collections.ArrayList
    $start = Get-Date
    Porneste
    $poz = 0
    $faraRaspuns = 0
    Scrie "Ceasul de siguranta pornit (verificare la $IntervalSecunde s$(if ($MinuteMaxim) { ", oprire dupa $MinuteMaxim min" }))."
    while ($true) {
        Start-Sleep -Seconds $IntervalSecunde
        if ($MinuteMaxim -gt 0 -and ((Get-Date) - $start).TotalMinutes -ge $MinuteMaxim) {
            Scrie "Au trecut $MinuteMaxim minute: opresc masina."
            Opreste
            return
        }
        $motiv = ''
        $p = Proces-Masina
        if (-not $p) { $motiv = 'QEMU s-a oprit' }
        else {
            $s = Stare-Qemu
            if (-not $s) {
                $faraRaspuns++
                if ($faraRaspuns -ge 3) { $motiv = 'QEMU nu raspunde de 3 verificari' }
            } else {
                $faraRaspuns = 0
                if ($s -in @('shutdown', 'guest-panicked', 'internal-error', 'io-error')) { $motiv = "QEMU in starea $s" }
            }
            if (-not $motiv -and (Contine-Panic (Serial-Curent) ([ref]$poz))) { $motiv = 'PANIC in jurnalul serial' }
        }
        if (-not $motiv) { continue }
        $acum = Get-Date
        $recente = @($reporniri | Where-Object { ($acum - $_).TotalMinutes -lt 60 })
        if ($recente.Count -ge 3) {
            $rand = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')  OPRIT: $motiv, dar au fost deja 3 reporniri in ultima ora. Masina ramane oprita; trebuie verificat jurnalul $(Serial-Curent)."
            Add-Content -Path $notaSupraveghere -Value $rand
            Scrie $rand
            Opreste
            exit 1
        }
        Add-Content -Path $notaSupraveghere -Value "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')  repornire: $motiv (jurnal $(Serial-Curent))"
        Scrie "Repornesc masina: $motiv"
        Opreste
        Porneste
        [void]$reporniri.Add($acum)
        $poz = 0
        $faraRaspuns = 0
    }
}

function Disc-Nou {
    if (Proces-Masina) { throw 'Masina e pornita: ruleaza intai .\vm-pornire.ps1 opreste' }
    $sursa = $DiscNou
    if (-not $sursa) { $sursa = Join-Path $PSScriptRoot $NumeDisc }
    if (-not (Test-Path $sursa)) { throw "Nu gasesc discul nou $sursa" }
    if ((Resolve-Path $sursa).Path -eq $Disc) { throw 'Discul nou e chiar discul masinii.' }
    if (Test-Path $Disc) { Copy-Item $Disc "$Disc.vechi" -Force; Scrie "Discul vechi pastrat ca $Disc.vechi" }
    Copy-Item $sursa $Disc -Force
    Scrie "Disc nou pus: $sursa"
}

New-Item -ItemType Directory -Force -Path $Dosar, $Jurnale | Out-Null
$script:QemuExe = ''
switch ($Comanda) {
    'porneste'   { Porneste }
    'opreste'    { Opreste }
    'reporneste' { Opreste; Porneste }
    'stare'      { Stare }
    'supraveghe' { Supraveghe }
    'disc-nou'   { Disc-Nou }
}
