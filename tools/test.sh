#!/usr/bin/env bash
# Automated headless tests: every scenario from tests/scenarios.txt is built and booted in QEMU.
#   tools/test.sh                 all scenarios
#   tools/test.sh boot log        only the named scenarios
#   tools/test.sh --no-build ...  reuse existing ISOs
#   tools/test.sh --stress        5-minute stability test (tests/stress.txt), not run on every push
# Exit code: 0 = all passed, 1 = at least one failed.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/qemu-common.sh"

BUILD_FIRST=1
SCENARIOS="$ROOT/tests/scenarios.txt"
TIMEOUT=60
ONLY=()
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD_FIRST=0 ;;
        --stress) SCENARIOS="$ROOT/tests/stress.txt"; TIMEOUT=420 ;;
        -*) echo "Opțiune necunoscută: $arg" >&2; exit 2 ;;
        *) ONLY+=("$arg") ;;
    esac
done

find_ovmf || exit 1
mkdir -p "$BUILD"
trim() { local s="$1"; s="${s#"${s%%[![:space:]]*}"}"; echo "${s%"${s##*[![:space:]]}"}"; }

passed=0; failed=0; failed_names=()
while IFS='|' read -r name features ending expects screenshot qextra; do
    name="$(trim "$name")"
    [[ -z "$name" || "$name" == \#* ]] && continue
    if (( ${#ONLY[@]} )) && [[ ! " ${ONLY[*]} " == *" $name "* ]]; then continue; fi
    features="$(trim "$features")"; ending="$(trim "$ending")"
    expects="$(trim "$expects")"; screenshot="$(trim "${screenshot:-}")"; qextra="$(trim "${qextra:-}")"

    echo "== scenariu: $name"
    build_args=(--test); [[ -n "$features" ]] && build_args+=(--features "$features")
    iso="$BUILD/cyber-universe-test.iso"
    [[ -n "$features" ]] && iso="$BUILD/cyber-universe-test-$(echo "$features" | tr ',' '-').iso"
    if (( BUILD_FIRST )); then
        if ! "$ROOT/tools/build.sh" "${build_args[@]}" > "$BUILD/build-$name.log" 2>&1; then
            echo "[test] build eșuat:"; tail -30 "$BUILD/build-$name.log"
            failed=$((failed+1)); failed_names+=("$name"); continue
        fi
    fi

    # disk=NAME: the test disk is a fresh copy (build/NAME) marked for writing (fat32::WRITE_MARKER),
    # attached without snapshot; tools/after-<scenario>.sh then checks it.
    DISK_IMG=""
    if [[ "$qextra" =~ (^|[[:space:]])disk=([^[:space:]]+) ]]; then
        DISK_IMG="$BUILD/${BASH_REMATCH[2]}"
        qextra="${qextra/disk=${BASH_REMATCH[2]}/}"
        cp "$BUILD/disk.img" "$DISK_IMG"
        echo "scriere permisa" > "$BUILD/cyber.rw"
        MTOOLS_SKIP_CHECK=1 mcopy -o -i "$DISK_IMG@@1M" "$BUILD/cyber.rw" ::/cyber.rw
    fi
    export DISK_IMG
    mapfile -t ARGS < <(qemu_args "$iso")
    QARGS=(); skip=0
    for a in "${ARGS[@]}"; do
        if (( skip )); then skip=0; continue; fi
        if [[ "$a" == "-cdrom" ]]; then skip=1; continue; fi
        QARGS+=("$a")
    done
    keys=""
    if [[ "$qextra" == *keys=* ]]; then keys="${qextra#*keys=}"; qextra="${qextra%%keys=*}"; fi
    mon=""
    if [[ "$qextra" == *mon=* ]]; then mon="${qextra#*mon=}"; qextra="${qextra%%mon=*}"; fi
    qextra="${qextra//\{BUILD\}/$BUILD}"   # {BUILD} = the build directory (for disk images)
    # shellcheck disable=SC2206
    [[ -n "$qextra" ]] && QARGS+=($qextra)
    PY=(--iso "$iso" --log "$BUILD/serial-$name.log" --timeout "$TIMEOUT" --expect-exit "$ending")
    IFS=$'\n' read -r -d '' -a lines < <(echo "$expects" | sed 's/;;/\n/g' && printf '\0')
    for l in "${lines[@]}"; do l="$(trim "$l")"; [[ -n "$l" ]] && PY+=(--expect "$l"); done
    if [[ "$screenshot" == *@* ]]; then   # file@line: take it when the kernel prints that line
        PY+=(--screenshot "$ROOT/${screenshot%%@*}" --screenshot-when "${screenshot#*@}")
    elif [[ -n "$screenshot" ]]; then
        PY+=(--screenshot "$ROOT/$screenshot")
    fi
    [[ -n "$keys" ]] && PY+=(--keys "$keys")
    [[ -n "$mon" ]] && PY+=(--monitor "$mon")

    ok=0
    python3 "$ROOT/tools/qemutest.py" "${PY[@]}" -- "${QARGS[@]}" && ok=1
    if (( ok )) && [[ -x "$ROOT/tools/after-$name.sh" ]]; then
        "$ROOT/tools/after-$name.sh" "$DISK_IMG" || ok=0
    fi
    if (( ok )); then
        passed=$((passed+1))
    else
        failed=$((failed+1)); failed_names+=("$name")
    fi
done < "$SCENARIOS"

echo "== rezultat: $passed trecute, $failed eșuate${failed_names:+ (${failed_names[*]})}"
(( failed == 0 && passed > 0 ))
