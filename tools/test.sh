#!/usr/bin/env bash
# Automated headless tests: every scenario from tests/scenarios.txt is built and booted in QEMU.
#   tools/test.sh                 all scenarios
#   tools/test.sh boot log        only the named scenarios
#   tools/test.sh --no-build ...  reuse existing ISOs
#   CYBER_NET=1 tools/test.sh    also the scenarios marked `internet` (real web sites)
#   tools/test.sh --stress        5-minute stability test (tests/stress.txt), not run on every push
#   tools/test.sh --shard K/N     only every N-th scenario starting with the K-th (CI runs the parts in parallel, P6-67)
# Exit code: 0 = all passed, 1 = at least one failed.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/qemu-common.sh"

BUILD_FIRST=1
SCENARIOS="$ROOT/tests/scenarios.txt"
TIMEOUT=60
ONLY=()
SHARD_K=1; SHARD_N=1
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
    arg="${args[i]}"
    case "$arg" in
        --shard) i=$((i+1)); arg="--shard=${args[i]:-}" ;;
    esac
    case "$arg" in
        --shard=*)
            v="${arg#--shard=}"
            if [[ ! "$v" =~ ^([0-9]+)/([0-9]+)$ ]] || (( BASH_REMATCH[1] < 1 || BASH_REMATCH[1] > BASH_REMATCH[2] )); then
                echo "--shard cere K/N cu 1 <= K <= N (primit: '$v')" >&2; exit 2
            fi
            SHARD_K="${BASH_REMATCH[1]}"; SHARD_N="${BASH_REMATCH[2]}" ;;
        --no-build) BUILD_FIRST=0 ;;
        --stress) SCENARIOS="$ROOT/tests/stress.txt"; TIMEOUT=420 ;;
        -*) echo "Opțiune necunoscută: $arg" >&2; exit 2 ;;
        *) ONLY+=("$arg") ;;
    esac
done

find_ovmf || exit 1
mkdir -p "$BUILD"
trim() { local s="$1"; s="${s#"${s%%[![:space:]]*}"}"; echo "${s%"${s##*[![:space:]]}"}"; }

passed=0; failed=0; skipped=0; failed_names=()
index=0
while IFS='|' read -r name features ending expects screenshot qextra; do
    name="$(trim "$name")"
    [[ -z "$name" || "$name" == \#* ]] && continue
    index=$((index+1))
    (( (index - 1) % SHARD_N == SHARD_K - 1 )) || continue
    if (( ${#ONLY[@]} )) && [[ ! " ${ONLY[*]} " == *" $name "* ]]; then continue; fi
    features="$(trim "$features")"; ending="$(trim "$ending")"
    expects="$(trim "$expects")"; screenshot="$(trim "${screenshot:-}")"; qextra="$(trim "${qextra:-}")"

    echo "== scenariu: $name"
    # `internet` in the QEMU column: the scenario talks to real web sites; it runs only with CYBER_NET=1 (CI).
    if [[ "$qextra" =~ (^|[[:space:]])internet($|[[:space:]]) ]]; then
        if [[ "${CYBER_NET:-}" != "1" ]]; then
            echo "[test] sarit: are nevoie de internet (CYBER_NET=1)"; skipped=$((skipped+1)); continue
        fi
        qextra="${qextra/internet/}"
    fi
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
        # tests/disk/<name>/ (if it exists) holds extra files for this disk, e.g. saved settings.
        extra="$ROOT/tests/disk/${BASH_REMATCH[2]%.img}"
        if [[ -d "$extra" ]]; then
            MTOOLS_SKIP_CHECK=1 mcopy -o -s -i "$DISK_IMG@@1M" "$extra"/* ::/
        fi
    fi
    export DISK_IMG
    # timeout=N: this scenario needs longer than the default (e.g. the whole simulated GPU chain, P6-56).
    stimeout="$TIMEOUT"
    if [[ "$qextra" =~ (^|[[:space:]])timeout=([0-9]+) ]]; then
        stimeout="${BASH_REMATCH[2]}"
        qextra="${qextra/timeout=${BASH_REMATCH[2]}/}"
    fi
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
    PY=(--iso "$iso" --log "$BUILD/serial-$name.log" --timeout "$stimeout" --expect-exit "$ending")
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
    helper=""
    if [[ -x "$ROOT/tools/before-$name.sh" ]]; then   # a helper the scenario talks to (e.g. a test DNS server)
        "$ROOT/tools/before-$name.sh" & helper=$!
        sleep 0.5
    fi
    python3 "$ROOT/tools/qemutest.py" "${PY[@]}" -- "${QARGS[@]}" && ok=1
    if [[ -n "$helper" ]]; then kill "$helper" 2>/dev/null; wait "$helper" 2>/dev/null; fi
    if (( ok )) && [[ -x "$ROOT/tools/after-$name.sh" ]]; then
        "$ROOT/tools/after-$name.sh" "$DISK_IMG" || ok=0
    fi
    if (( ok )); then
        passed=$((passed+1))
    else
        failed=$((failed+1)); failed_names+=("$name")
    fi
done < "$SCENARIOS"

(( SHARD_N > 1 )) && echo "== partea $SHARD_K/$SHARD_N din scenarii"
echo "== rezultat: $passed trecute, $failed eșuate${failed_names:+ (${failed_names[*]})}$( (( skipped )) && echo ", $skipped sarite (fara internet)")"
(( failed == 0 && passed > 0 ))
