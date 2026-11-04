#!/usr/bin/env bash
# Test of the virtual machine (P6-48, C-68 F1): the VM disk boots as a USB stick, mounts /usb with write access and
# gets an address by DHCP; tools/windows/vm-pornire.ps1 (run with PowerShell 7 = pwsh, present on the GitHub
# machines) starts, reports, stops and - as the safety watch - restarts a VM that died.
# Run after `tools/build.sh --usb` and `tools/vm-image.sh`.
#   tools/vm-test.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/tools/qemu-common.sh"
DISK="$BUILD/cyber-universe-vm.qcow2"
PS1="$ROOT/tools/windows/vm-pornire.ps1"
[[ -f "$DISK" ]] || { echo "[vm-test] EROARE: lipseste $DISK (tools/vm-image.sh)"; exit 1; }
WORK="$(mktemp -d)"
cleanup() {
    [[ -f "$WORK/masina.pid" ]] && kill "$(cat "$WORK/masina.pid")" 2>/dev/null || true
    [[ -n "${QPID:-}" ]] && kill "$QPID" 2>/dev/null || true
    [[ -n "${WPID:-}" ]] && kill "$WPID" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT
cp "$DISK" "$WORK/cyber-universe-vm.qcow2"
QEMU="$(command -v qemu-system-x86_64)"
EXPECT=("montat la /usb (scriere permisa)" "net: DHCP: adresa 10.0.2.15" "procesoare")
PORT=$((20000 + RANDOM % 20000))

# Waits until the serial log named by $1 (a file holding the path, or the path) contains all EXPECT lines.
wait_boot() {
    local log="$1" t
    for ((t = 0; t < 180; t++)); do
        local f="$log"
        [[ -f "$WORK/serial-curent.txt" && "$log" == "curent" ]] && f="$(head -n1 "$WORK/serial-curent.txt")"
        if [[ -f "$f" ]]; then
            local ok=1
            for e in "${EXPECT[@]}"; do grep -qF "$e" "$f" || ok=0; done
            if (( ok )); then echo "[vm-test] pornire ok in ~${t} s: ${EXPECT[*]}"; return 0; fi
            if grep -q "PANIC:" "$f"; then echo "[vm-test] EROARE: PANIC"; tail -20 "$f"; return 1; fi
        fi
        sleep 1
    done
    echo "[vm-test] EROARE: pornirea nu a ajuns la retea + /usb in 180 s"; [[ -f "${f:-}" ]] && tail -30 "$f"
    return 1
}

if ! command -v pwsh >/dev/null; then
    if [[ -n "${CI:-}" ]]; then echo "[vm-test] EROARE: in CI trebuie pwsh (PowerShell 7)"; exit 1; fi
    echo "[vm-test] ATENTIE: pwsh lipseste aici: testez doar discul (aceleasi argumente QEMU ca scriptul), nu si scriptul"
    find_ovmf
    cp "$OVMF_VARS_TEMPLATE" "$WORK/vars.fd"
    "$QEMU" -M q35 -smp 4 -m 2G \
        -drive "if=pflash,unit=0,format=raw,readonly=on,file=$OVMF_CODE" -drive "if=pflash,unit=1,format=raw,file=$WORK/vars.fd" \
        -device qemu-xhci,id=xhci -drive "if=none,id=disc,format=qcow2,file=$WORK/cyber-universe-vm.qcow2" \
        -device usb-storage,bus=xhci.0,drive=disc,bootindex=0 -device usb-kbd,bus=xhci.0 \
        -nic none -netdev user,id=n0 -device e1000e,netdev=n0 -serial "file:$WORK/serial.txt" -display none \
        -accel tcg,thread=multi >"$WORK/qemu.txt" 2>&1 &
    QPID=$!
    wait_boot "$WORK/serial.txt"
    echo "[vm-test] TRECE (doar discul)"
    exit 0
fi

ps() { pwsh -NoProfile -NonInteractive -File "$PS1" "$@" -Dosar "$WORK" -Qemu "$QEMU" -FaraEcran -Nuclee 4 -MemorieGB 2 -PortQmp "$PORT"; }

echo "[vm-test] 1. porneste"
ps porneste
wait_boot curent
echo "[vm-test] 2. stare"
OUT="$(ps stare)"; echo "$OUT"
grep -q "stare: pornita (proces .*QEMU: running)" <<<"$OUT" || { echo "[vm-test] EROARE: stare nu spune pornita/running"; exit 1; }
echo "[vm-test] 3. a doua pornire nu porneste alt QEMU"
OUT="$(ps porneste)"; grep -q "deja pornita" <<<"$OUT" || { echo "[vm-test] EROARE: a doua pornire: $OUT"; exit 1; }
echo "[vm-test] 4. opreste"
P="$(cat "$WORK/masina.pid")"
ps opreste
sleep 1
if kill -0 "$P" 2>/dev/null; then echo "[vm-test] EROARE: QEMU inca merge dupa opreste"; exit 1; fi
ps stare | grep -q "stare: oprita" || { echo "[vm-test] EROARE: stare dupa oprire"; exit 1; }
grep -q -- "-smp 4" "$WORK/jurnal/ultima-comanda.txt" && ! grep -qi "vfio" "$WORK/jurnal/ultima-comanda.txt" \
    || { echo "[vm-test] EROARE: comanda QEMU (nuclee / fara placa video)"; exit 1; }

echo "[vm-test] 5. supraveghe: QEMU oprit din afara -> repornit"
pwsh -NoProfile -NonInteractive -File "$PS1" supraveghe -Dosar "$WORK" -Qemu "$QEMU" -FaraEcran -Nuclee 4 -MemorieGB 2 \
    -PortQmp "$PORT" -IntervalSecunde 3 -MinuteMaxim 3 >"$WORK/supraveghe.txt" 2>&1 &
WPID=$!
sleep 5
wait_boot curent
FIRST="$(head -n1 "$WORK/serial-curent.txt")"
kill -9 "$(cat "$WORK/masina.pid")"
for ((t = 0; t < 30; t++)); do
    grep -q "repornire: QEMU s-a oprit" "$WORK/jurnal/supraveghere.txt" 2>/dev/null && break
    sleep 1
done
grep -q "repornire: QEMU s-a oprit" "$WORK/jurnal/supraveghere.txt" || { echo "[vm-test] EROARE: nicio repornire"; cat "$WORK/supraveghe.txt"; exit 1; }
sleep 2
[[ "$(head -n1 "$WORK/serial-curent.txt")" != "$FIRST" ]] || sleep 2
wait_boot curent
echo "[vm-test] repornit: $(cat "$WORK/jurnal/supraveghere.txt")"
kill "$WPID" 2>/dev/null || true; wait "$WPID" 2>/dev/null || true; WPID=""
ps opreste
echo "[vm-test] TRECE"
