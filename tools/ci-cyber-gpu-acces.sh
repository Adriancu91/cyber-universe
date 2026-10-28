#!/usr/bin/env bash
# B-005 in CI: `cyber-gpu` is private and the workflow's own github.token only sees `cyber-universe`.
# With a read-only token for cyber-gpu stored as the repository secret CYBER_GPU_TOKEN, git (and cargo through
# the git CLI) fetches https://github.com/Adriancu91/cyber-gpu with that token. Nothing in Cargo.toml changes.
# Without the secret the job stops here with one clear message instead of 81 identical build failures.
#   CYBER_GPU_TOKEN=... tools/ci-cyber-gpu-acces.sh
set -euo pipefail

URL="https://github.com/Adriancu91/cyber-gpu"

if [[ -z "${CYBER_GPU_TOKEN:-}" ]]; then
    echo "::error::Lipseste secretul CYBER_GPU_TOKEN: cyber-gpu e privat si CI-ul nu-l poate descarca (B-005, docs/BLOCKERS.md)."
    exit 1
fi

git config --global --unset-all "url.https://x-access-token:${CYBER_GPU_TOKEN}@github.com/Adriancu91/cyber-gpu.insteadOf" 2>/dev/null || true
git config --global --add "url.https://x-access-token:${CYBER_GPU_TOKEN}@github.com/Adriancu91/cyber-gpu.insteadOf" "$URL"
mkdir -p "$HOME/.cargo"
if ! grep -q 'git-fetch-with-cli' "$HOME/.cargo/config.toml" 2>/dev/null; then
    printf '\n[net]\ngit-fetch-with-cli = true\n' >> "$HOME/.cargo/config.toml"
fi

# C-37: actions/checkout may leave an `http.https://github.com/.extraheader` with github.token in the repository's
# local config; git would send that header instead of the token from the URL. Check from $HOME (outside the
# repository) and with the extra headers reset (an empty value clears the list).
if (cd "$HOME" && GIT_TERMINAL_PROMPT=0 git -c http.extraheader= -c "http.https://github.com/.extraheader=" \
        ls-remote -q "$URL" HEAD >/dev/null 2>&1); then
    echo "cyber-gpu: acces cu CYBER_GPU_TOKEN (doar citire) - ok"
else
    echo "::error::CYBER_GPU_TOKEN exista, dar GitHub refuza cyber-gpu (token expirat sau fara drept de citire pe cyber-gpu)."
    exit 1
fi
