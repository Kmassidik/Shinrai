#!/usr/bin/env bash
# curl -fsSL https://raw.githubusercontent.com/Kmassidik/Shinrai/main/install.sh | bash
set -euo pipefail

echo ""
echo "Welcome to shinrai."
echo ""
echo "Cloudflare Zero Trust from the CLI — no dashboard required."
echo "This install puts the shinrai binary on your PATH."
echo ""

REPO_URL="${SHINRAI_REPO_URL:-https://github.com/Kmassidik/Shinrai.git}"
INSTALL_DIR="${SHINRAI_INSTALL_DIR:-$HOME/.local/bin}"
SRC_DIR="${SHINRAI_SRC_DIR:-$HOME/.cache/shinrai/src}"

mkdir -p "$INSTALL_DIR"
mkdir -p "$(dirname "$SRC_DIR")"

if ! command -v nix >/dev/null 2>&1; then
  echo "error: nix is required (this project is Nix-first)." >&2
  echo "  install: https://determinate.systems/nix" >&2
  exit 1
fi

if [[ -d "$SRC_DIR/.git" ]]; then
  echo "Updating source in $SRC_DIR ..."
  git -C "$SRC_DIR" pull --ff-only
else
  echo "Cloning $REPO_URL → $SRC_DIR ..."
  # Private repos: ensure git auth (SSH remote or gh credential) works first.
  git clone "$REPO_URL" "$SRC_DIR"
fi

echo "Building with nix develop + cargo ..."
(
  cd "$SRC_DIR"
  nix develop --command cargo build --release
)

BIN="$SRC_DIR/target/release/shinrai"
install -m 755 "$BIN" "$INSTALL_DIR/shinrai"

echo ""
echo "Installed: $INSTALL_DIR/shinrai"
if ! echo ":$PATH:" | grep -q ":$INSTALL_DIR:"; then
  echo "Add to PATH, e.g.:"
  echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
echo ""
"$INSTALL_DIR/shinrai" hello
echo "Next:"
echo "  shinrai install   # cloudflared if missing"
echo "  shinrai up        # init + start — public host live on the internet"
echo ""
