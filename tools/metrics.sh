#!/usr/bin/env bash
# Boot metrics (P1-12): builds the normal kernel, boots it RUNS times in QEMU and averages
#   - total time from QEMU start to the kernel's "metrici:" line (firmware + Limine + kernel),
#   - the kernel's own start-up time (kmain -> ready, measured with the TSC),
#   - memory used by the kernel, and the kernel file size.
#   tools/metrics.sh            5 runs, prints one Markdown table row for docs/METRICS.md
#   tools/metrics.sh --runs 3
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/qemu-common.sh"

RUNS=5
[[ "${1:-}" == "--runs" ]] && RUNS="$2"

find_ovmf
"$ROOT/tools/build.sh" > "$BUILD/build-metrics.log" 2>&1 || { tail -30 "$BUILD/build-metrics.log"; exit 1; }
ISO="$BUILD/cyber-universe.iso"
KERNEL="$BUILD/target-normal/x86_64-unknown-none/release/cyber_kernel"
mapfile -t ARGS < <(qemu_args "$ISO")
VERSION="$(grep -m1 '^version' "$ROOT/kernel/Cargo.toml" | cut -d'"' -f2) ($(git -C "$ROOT" rev-parse --short HEAD))"

python3 - "$RUNS" "$KERNEL" "$VERSION" "${ARGS[@]}" <<'PY'
import os, re, subprocess, sys, time
runs, kernel, version, args = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4:]
pat = re.compile(r"metrici: pornire ([\d.]+) ms; memorie nucleu (\d+) KiB \(imagine (\d+) KiB \+ cadre (\d+) KiB")
total, kern, mem = [], [], []
for i in range(runs):
    t0 = time.time()
    p = subprocess.Popen(["qemu-system-x86_64", *args, "-display", "none", "-serial", "stdio"],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
    os.set_blocking(p.stdout.fileno(), False)
    out, m = b"", None
    while time.time() - t0 < 60 and not m:
        try:
            c = p.stdout.read(4096)
        except BlockingIOError:
            c = None
        if c:
            out += c
            m = pat.search(out.decode("utf-8", "replace"))
        else:
            time.sleep(0.01)
    elapsed = time.time() - t0
    p.kill(); p.wait()
    if not m:
        print(f"EROARE: rularea {i+1} nu a scris linia 'metrici:'", file=sys.stderr)
        print(out.decode("utf-8", "replace")[-2000:], file=sys.stderr)
        sys.exit(1)
    total.append(elapsed * 1000); kern.append(float(m.group(1))); mem.append(int(m.group(2)))
    print(f"rularea {i+1}: total {elapsed*1000:.0f} ms, nucleu {m.group(1)} ms, memorie {m.group(2)} KiB", file=sys.stderr)
avg = lambda v: sum(v) / len(v)
size = os.path.getsize(kernel) // 1024
accel = "KVM" if "-enable-kvm" in args else "TCG (fără accelerare)"
print(f"| {time.strftime('%Y-%m-%d')} | {version} | {accel} | {avg(total):.0f} ms | {avg(kern):.1f} ms | {avg(mem):.0f} KiB | {size} KiB |")
PY
