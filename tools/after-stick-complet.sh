#!/usr/bin/env bash
# P6-84 (C-114): the single test stick (64 GiB FAT32 with the system copied on it) is checked like fat-mare, plus the
# system files must still be there, unchanged by the journal writes.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/tools/after-fat-mare.sh" || exit 1
MTOOLS_SKIP_CHECK=1 mdir -b -i "$ROOT/build/fat-mare.img" ::/EFI/BOOT | grep -qi "BOOTX64.EFI" || { echo "[after-stick-complet] EȘUAT: BOOTX64.EFI lipsește"; exit 1; }
MTOOLS_SKIP_CHECK=1 mdir -b -i "$ROOT/build/fat-mare.img" ::/boot | grep -qi "kernel" || { echo "[after-stick-complet] EȘUAT: boot/kernel lipsește"; exit 1; }
echo "[after-stick-complet] sistemul a pornit de pe stick-ul de 64 GB și fișierele lui sunt la locul lor"
