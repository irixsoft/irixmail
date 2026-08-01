#!/usr/bin/env sh
set -eu

REPO="${IRIXMAIL_REPO:-irixsoft/irixmail}"
VERSION="${IRIXMAIL_VERSION:-latest}"
PREFIX="/usr/local/bin"
CONFIG_DIR="/etc/irixmail"
DATA_DIR="/var/lib/irixmail"
LOG_DIR="/var/log/irixmail"
SERVICE_USER="irixmail"

WORKDIR=""
cleanup() {
  [ -n "$WORKDIR" ] && rm -rf "$WORKDIR"
}
trap cleanup EXIT INT TERM

fail() {
  echo "error: $1" >&2
  exit 1
}

need_root() {
  [ "$(id -u)" -eq 0 ] || fail "please run this installer as root (e.g. with sudo)"
}

detect_target() {
  case "$(uname -m)" in
    x86_64 | amd64) echo "x86_64-unknown-linux-musl" ;;
    aarch64 | arm64) echo "aarch64-unknown-linux-musl" ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
}

download_binary() {
  target="$1"
  base="https://github.com/${REPO}/releases"
  if [ "$VERSION" = "latest" ]; then
    url="${base}/latest/download/irixmail-${target}"
  else
    url="${base}/download/${VERSION}/irixmail-${target}"
  fi

  echo "Downloading ${url}"
  curl -fSL "$url" -o "${WORKDIR}/irixmail" || fail "could not download the binary"

  echo "Verifying checksum"
  curl -fSL "${url}.sha256" -o "${WORKDIR}/irixmail.sha256" \
    || fail "could not download the checksum; refusing to install an unverified binary"
  expected="$(cut -d ' ' -f1 "${WORKDIR}/irixmail.sha256")"
  actual="$(sha256sum "${WORKDIR}/irixmail" | cut -d ' ' -f1)"
  [ -n "$expected" ] || fail "the published checksum was empty"
  [ "$expected" = "$actual" ] || fail "checksum verification failed"

  install -m 0755 "${WORKDIR}/irixmail" "${PREFIX}/irixmail"
}

create_user() {
  if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
  fi
}

create_dirs() {
  mkdir -p "$CONFIG_DIR" "$DATA_DIR" "$LOG_DIR"
  chown -R "${SERVICE_USER}:${SERVICE_USER}" "$DATA_DIR" "$LOG_DIR"
  chmod 0750 "$DATA_DIR" "$LOG_DIR"
  chmod 0755 "$CONFIG_DIR"
}

install_unit() {
  cat > /etc/systemd/system/irixmail.service <<'UNIT'
[Unit]
Description=IRIXMAIL mail server
Documentation=https://irixsoft.com
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=irixmail
Group=irixmail
ExecStart=/usr/local/bin/irixmail run
Restart=on-failure
RestartSec=5
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/irixmail /var/log/irixmail /etc/irixmail
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT

  cat > /etc/systemd/system/irixmail-update.service <<'UNIT'
[Unit]
Description=IRIXMAIL update
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/irixmail update
UNIT

  cat > /etc/systemd/system/irixmail-update.timer <<'UNIT'
[Unit]
Description=Daily IRIXMAIL update check

[Timer]
OnCalendar=daily
RandomizedDelaySec=1h
Persistent=true

[Install]
WantedBy=timers.target
UNIT
  systemctl daemon-reload || true
}

main() {
  need_root
  WORKDIR="$(mktemp -d)" || fail "could not create a temporary working directory"
  target="$(detect_target)"
  download_binary "$target"
  create_user
  create_dirs
  install_unit

  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    echo "Launching interactive setup..."
    "${PREFIX}/irixmail" setup < /dev/tty
    chown -R "${SERVICE_USER}:${SERVICE_USER}" "$DATA_DIR" "$LOG_DIR"
    if [ -f "${CONFIG_DIR}/config.toml" ]; then
      chown "${SERVICE_USER}:${SERVICE_USER}" "${CONFIG_DIR}/config.toml"
    fi
    echo
    echo "Done. Start the server on boot with:"
    echo "  systemctl enable --now irixmail"
  else
    echo
    echo "Installed. No terminal available, so run the setup yourself:"
    echo "  sudo irixmail setup"
    echo "Then start the server on boot with:"
    echo "  systemctl enable --now irixmail"
  fi
}

main "$@"
