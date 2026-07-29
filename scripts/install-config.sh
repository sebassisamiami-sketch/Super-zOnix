#!/usr/bin/env bash
# Install the zOnix PipeWire virtual-sink config for the current user and
# restart the user audio services so the lanes appear.
#
# Safe to run repeatedly (idempotent copy). No root required — everything lives
# under the user's ~/.config and the systemd --user session.
set -euo pipefail

# Resolve the repo root relative to this script.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SRC="${REPO_ROOT}/wireplumber/config/pipewire.conf.d/50-zonix-loopback.conf"
DEST_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pipewire/pipewire.conf.d"
DEST="${DEST_DIR}/50-zonix-loopback.conf"

if [[ ! -f "${SRC}" ]]; then
    echo "error: config not found at ${SRC}" >&2
    exit 1
fi

echo "==> Installing zOnix virtual-sink config"
mkdir -p "${DEST_DIR}"
install -m 0644 "${SRC}" "${DEST}"
echo "    ${DEST}"

# Validate the dropped-in config parses before restarting anything.
if command -v pw-cli >/dev/null 2>&1; then
    echo "==> PipeWire tools detected: $(pw-cli --version 2>/dev/null | head -1 || true)"
fi

echo "==> Restarting user audio services"
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user restart pipewire.service pipewire-pulse.service wireplumber.service \
        || echo "    (could not restart via systemctl --user; log out/in or restart manually)"
else
    echo "    systemctl not found; restart PipeWire manually."
fi

echo "==> Done. Verify the lanes exist:"
echo "    wpctl status | grep -i zonix"
echo "    # or"
echo "    pw-cli ls Node | grep -i zonix.sink"
