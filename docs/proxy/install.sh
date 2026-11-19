#!/usr/bin/env bash
# Clod Helper - one-command proxy installer.
# Installs 3proxy 0.9.4 (HTTP + SOCKS5 with login/password) on Ubuntu 22.04/24.04 or Debian 12.
#
# Usage (as root):
#   curl -fsSL https://iimperium-dev.github.io/ClodHelper/proxy/install.sh | bash -s -- [LOGIN] [PASSWORD] [HTTP_PORT] [SOCKS_PORT]
#
# All arguments are optional. Missing values are taken from the previous run
# (/etc/3proxy/install.state) or generated. Running it again is safe.
# All output is plain ASCII on purpose: VGA text consoles cannot show anything else.
#
# Everything lives inside main(), which is called on the very last line,
# so a partially downloaded script does nothing.

PROXY_VERSION="0.9.4"
SRC_URL="https://github.com/3proxy/3proxy/archive/refs/tags/${PROXY_VERSION}.tar.gz"
BIN=/usr/local/bin/3proxy
CFG_DIR=/etc/3proxy
CFG="$CFG_DIR/3proxy.cfg"
STATE="$CFG_DIR/install.state"
UNIT=/etc/systemd/system/3proxy.service
SYSCTL_FILE=/etc/sysctl.d/99-proxy.conf
LIMITS_FILE=/etc/security/limits.d/99-nofile.conf
BBR_FILE=/etc/modules-load.d/bbr.conf
LOG=/var/log/clod-proxy-install.log
SVC_USER=proxy3
PACKAGES="build-essential wget ca-certificates curl ufw"
STEPS=6

say() { printf '%s\n' "$*"; }
log() { printf '%s %s\n' "$(date '+%F %T')" "$*" 2>/dev/null >>"$LOG"; }
warn() { printf 'WARNING: %s\n' "$*"; log "WARNING: $*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
die_log() {
  log "ERROR: $*"
  printf 'ERROR: %s\n' "$*" >&2
  printf 'Details are in the log file: %s\n' "$LOG" >&2
  printf 'Last lines of the log:\n' >&2
  tail -n 15 "$LOG" 2>/dev/null | LC_ALL=C tr -c '\n\40-\176' '?' >&2
  exit 1
}
step() { printf '[%s/%s] %s\n' "$1" "$STEPS" "$2"; log "=== step $1: $2"; }

# Run a command, send all its output to the log file.
run() { log "+ $*"; "$@" </dev/null >>"$LOG" 2>&1; }

APT_OPTS="-o DPkg::Lock::Timeout=600"
# A previous apt/dpkg run was cut off (closed console, Ctrl+C, reboot during the
# first automatic update) -> apt refuses everything with "dpkg was interrupted".
# Finish it. dpkg has no lock timeout of its own, so wait while another apt holds the lock.
apt_repair() {
  local i
  for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
    if run dpkg --configure -a; then
      # shellcheck disable=SC2086
      run apt-get $APT_OPTS -f install -y || true
      return 0
    fi
    [ "$i" = 1 ] && say "    waiting for another apt/dpkg process to finish..."
    sleep 10
  done
  warn "dpkg --configure -a failed, see $LOG"
}

usage() {
  say "Usage: curl -fsSL https://iimperium-dev.github.io/ClodHelper/proxy/install.sh | bash -s -- [LOGIN] [PASSWORD] [HTTP_PORT] [SOCKS_PORT]"
  say "All four values are optional. Missing ones are generated (or reused from the last run)."
  say "  LOGIN      3-32 chars: small latin letters and digits, starts with a letter"
  say "  PASSWORD   8-64 chars: latin letters and digits"
  say "  PORTS      numbers 1025-65535, HTTP and SOCKS5 port must be different"
}

# ---------- random values ----------
rand_str() { # rand_str CHARSET LENGTH
  local set=$1 n=$2 s=""
  while [ "${#s}" -lt "$n" ]; do
    s="$s$(head -c 1024 /dev/urandom | LC_ALL=C tr -dc "$set")"
  done
  printf '%s' "${s:0:$n}"
}
rand_port() {
  local r
  r=$(od -An -N4 -tu4 /dev/urandom | tr -dc '0-9')
  printf '%s' $(( ${r:-12345} % 40001 + 20000 ))
}
port_busy() { timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$1" >/dev/null 2>&1; }

# ---------- validation ----------
valid_login() { [[ $1 =~ ^[a-z][a-z0-9]{2,31}$ ]]; }
valid_pass() { [[ $1 =~ ^[A-Za-z0-9]{8,64}$ ]]; }
valid_port() { [[ $1 =~ ^[0-9]{1,5}$ ]] && [ $((10#$1)) -ge 1025 ] && [ $((10#$1)) -le 65535 ]; }

# SSH ports from sshd config (22 is always included).
ssh_ports() {
  local p
  {
    echo 22
    cat /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf 2>/dev/null |
      sed -n 's/^[[:space:]]*[Pp][Oo][Rr][Tt][[:space:]]\{1,\}\([0-9]\{1,5\}\).*/\1/p'
  } | while read -r p; do echo $((10#$p)); done | sort -un
}
is_ssh_port() { ssh_ports | grep -qx "$1"; }

# ---------- self-test helpers ----------
# fetch_ip [curl options...] -> prints our public IPv4 as seen from the internet
fetch_ip() {
  local url out
  for url in https://api.ipify.org https://ipv4.icanhazip.com https://checkip.amazonaws.com; do
    out=$(env -u no_proxy -u NO_PROXY curl -s -m 15 "$@" "$url" 2>/dev/null | tr -d '[:space:]')
    if [[ $out =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
      printf '%s' "$out"
      return 0
    fi
  done
  return 1
}

service_logs() {
  if command -v journalctl >/dev/null 2>&1; then
    journalctl -u 3proxy -n 20 --no-pager 2>&1
  else
    echo "(journalctl not found)"
  fi
}

main() {
  # Script comes through a pipe: nothing below may read stdin.
  exec </dev/null
  umask 022
  export DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=a LC_ALL=C

  case "${1:-}" in -h|--help) usage; exit 0 ;; esac
  [ "$(id -u)" = 0 ] || die "Run as root. Type: sudo -i   then run the same command again."
  [ $# -le 4 ] || die "Too many arguments. Use: LOGIN PASSWORD HTTP_PORT SOCKS_PORT (all optional)."

  local i ok p a_login=${1:-} a_pass=${2:-} a_http=${3:-} a_socks=${4:-}

  # --- check explicit arguments ---
  if [ -n "$a_login" ] && ! valid_login "$a_login"; then
    die "Bad login '$a_login'. Use 3-32 small latin letters/digits, first one a letter (example: pxuser)."
  fi
  if [ -n "$a_pass" ] && ! valid_pass "$a_pass"; then
    die "Bad password. Use 8-64 latin letters and digits only (no spaces or symbols)."
  fi
  if [ -n "$a_http" ]; then
    valid_port "$a_http" || die "Bad HTTP port '$a_http'. Use a number from 1025 to 65535."
    a_http=$((10#$a_http))
  fi
  if [ -n "$a_socks" ]; then
    valid_port "$a_socks" || die "Bad SOCKS5 port '$a_socks'. Use a number from 1025 to 65535."
    a_socks=$((10#$a_socks))
  fi
  if [ -n "$a_http" ] && [ "$a_http" = "$a_socks" ]; then
    die "HTTP port and SOCKS5 port must be different."
  fi

  # --- OS check ---
  [ -r /etc/os-release ] || die "Unsupported system: /etc/os-release not found. Use Ubuntu 22.04/24.04 or Debian 12."
  local os_id os_ver os_name
  # shellcheck disable=SC1091
  os_id=$(. /etc/os-release && echo "${ID:-}")
  # shellcheck disable=SC1091
  os_ver=$(. /etc/os-release && echo "${VERSION_ID:-}")
  os_name="$os_id $os_ver"
  command -v apt-get >/dev/null 2>&1 || die "Unsupported system ($os_name). Use Ubuntu 22.04/24.04 or Debian 12."
  case "$os_id:$os_ver" in
    ubuntu:22.04|ubuntu:24.04|debian:12) ;;
    ubuntu:*|debian:*) ;; # checked below, after the log is open
    *) die "Unsupported system ($os_name). Use Ubuntu 22.04/24.04 or Debian 12." ;;
  esac

  if [ "${CLOD_NO_SYSTEMD:-0}" != 1 ] && [ ! -d /run/systemd/system ]; then
    die "systemd is not running on this machine. A normal VPS with Ubuntu 22.04/24.04 is needed."
  fi

  # --- previous run (state file) ---
  local o_login="" o_pass="" o_http="" o_socks="" k v
  if [ -f "$STATE" ]; then
    while IFS='=' read -r k v; do
      case "$k" in
        LOGIN) valid_login "$v" && o_login=$v ;;
        PASSWORD) valid_pass "$v" && o_pass=$v ;;
        HTTP_PORT) valid_port "$v" && o_http=$((10#$v)) ;;
        SOCKS_PORT) valid_port "$v" && o_socks=$((10#$v)) ;;
      esac
    done <"$STATE"
  fi

  # --- final values: explicit argument > saved value > generated ---
  local LOGIN PASS HTTP_PORT SOCKS_PORT reused=""
  LOGIN=$a_login
  if [ -z "$LOGIN" ]; then
    if [ -n "$o_login" ]; then LOGIN=$o_login; reused=1; else LOGIN="px$(rand_str 'a-z' 4)"; fi
  fi
  PASS=$a_pass
  if [ -z "$PASS" ]; then
    if [ -n "$o_pass" ]; then PASS=$o_pass; reused=1; else PASS=$(rand_str 'A-HJ-NP-Za-km-np-z2-9' 16); fi
  fi
  HTTP_PORT=$a_http
  SOCKS_PORT=$a_socks
  if [ -z "$HTTP_PORT" ] && [ -n "$o_http" ] && [ "$o_http" != "$SOCKS_PORT" ] && ! is_ssh_port "$o_http"; then
    HTTP_PORT=$o_http; reused=1
  fi
  if [ -z "$SOCKS_PORT" ] && [ -n "$o_socks" ] && [ "$o_socks" != "$HTTP_PORT" ] && ! is_ssh_port "$o_socks"; then
    SOCKS_PORT=$o_socks; reused=1
  fi
  local tries=0
  while [ -z "$HTTP_PORT" ] || [ -z "$SOCKS_PORT" ]; do
    tries=$((tries + 1))
    [ "$tries" -le 200 ] || die "Could not find free ports. Pass them as arguments."
    p=$(rand_port)
    if [ "$p" = "$HTTP_PORT" ] || [ "$p" = "$SOCKS_PORT" ] || is_ssh_port "$p" || port_busy "$p"; then
      continue
    fi
    if [ -z "$HTTP_PORT" ]; then HTTP_PORT=$p; else SOCKS_PORT=$p; fi
  done
  for p in "$HTTP_PORT" "$SOCKS_PORT"; do
    is_ssh_port "$p" && die "Port $p is used by SSH. Choose another port."
  done

  : >>"$LOG" 2>/dev/null || die "Cannot write log file $LOG"
  chmod 600 "$LOG" 2>/dev/null
  log "===== Clod proxy installer started ($os_name) ====="

  say "=================================================="
  say "  Clod Helper proxy installer (3proxy $PROXY_VERSION)"
  say "  System: $os_name"
  say "=================================================="
  case "$os_id:$os_ver" in
    ubuntu:22.04|ubuntu:24.04|debian:12) ;;
    *) warn "$os_name is not tested. Trying anyway." ;;
  esac
  [ -n "$reused" ] && say "Using saved settings from the previous run ($STATE)."
  log "settings: login=$LOGIN http=$HTTP_PORT socks=$SOCKS_PORT (password not logged)"

  # ---------- 1. packages ----------
  step 1 "Installing packages (can take a few minutes)..."
  local pkg missing=""
  for pkg in $PACKAGES; do
    dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed" || missing="$missing $pkg"
  done
  if [ -n "$missing" ]; then
    apt_repair
    ok=""
    for i in 1 2 3 4 5; do
      # shellcheck disable=SC2086
      if run apt-get $APT_OPTS update; then ok=1; break; fi
      say "    apt is busy or a mirror failed, retrying in 20 s ($i/5)..."
      # some servers have broken IPv6: mirrors answer only over IPv4
      APT_OPTS="-o DPkg::Lock::Timeout=600 -o Acquire::ForceIPv4=true"
      sleep 20
    done
    [ -n "$ok" ] || warn "apt-get update failed, trying to install anyway."
    ok=""
    for i in 1 2 3; do
      # shellcheck disable=SC2086
      if run apt-get $APT_OPTS -o Dpkg::Options::=--force-confdef \
        -o Dpkg::Options::=--force-confold install -y $missing; then
        ok=1; break
      fi
      say "    install failed, retrying in 30 s ($i/3)..."
      sleep 30
      apt_repair
      # shellcheck disable=SC2086
      run apt-get $APT_OPTS update
    done
    [ -n "$ok" ] || die_log "Could not install packages:$missing"
  else
    say "    already installed."
  fi

  # ---------- 2. build 3proxy ----------
  step 2 "Building 3proxy $PROXY_VERSION..."
  if [ -x "$BIN" ]; then
    say "    $BIN already exists, skipping build."
  else
    local tmp src
    tmp=$(mktemp -d /tmp/clod-3proxy.XXXXXX) || die_log "mktemp failed"
    ok=""
    for i in 1 2 3; do
      rm -f "$tmp/src.tar.gz"
      if run curl -fSL --connect-timeout 20 -m 600 -o "$tmp/src.tar.gz" "$SRC_URL" &&
        tar -tzf "$tmp/src.tar.gz" >/dev/null 2>&1; then
        ok=1; break
      fi
      say "    download failed, retrying ($i/3)..."
      sleep 5
    done
    [ -n "$ok" ] || { rm -rf "$tmp"; die_log "Could not download 3proxy from GitHub."; }
    run tar -xzf "$tmp/src.tar.gz" -C "$tmp" || { rm -rf "$tmp"; die_log "Could not unpack 3proxy."; }
    src="$tmp/3proxy-$PROXY_VERSION"
    say "    compiling (1-5 minutes)..."
    run make -C "$src" -f Makefile.Linux
    if [ ! -x "$src/bin/3proxy" ]; then
      rm -rf "$tmp"
      die_log "3proxy build failed."
    fi
    install -m 0755 "$src/bin/3proxy" "$BIN.new" && mv -f "$BIN.new" "$BIN" ||
      { rm -rf "$tmp"; die_log "Could not install $BIN"; }
    rm -rf "$tmp"
    say "    done."
  fi

  # ---------- 3. config + service ----------
  step 3 "Writing proxy config and service..."
  getent group "$SVC_USER" >/dev/null 2>&1 || run groupadd --system "$SVC_USER" ||
    die_log "Could not create group $SVC_USER"
  id -u "$SVC_USER" >/dev/null 2>&1 ||
    run useradd --system -g "$SVC_USER" -d /nonexistent -M -s /usr/sbin/nologin "$SVC_USER" ||
    die_log "Could not create user $SVC_USER"

  mkdir -p "$CFG_DIR" && chown "root:$SVC_USER" "$CFG_DIR" && chmod 750 "$CFG_DIR" ||
    die_log "Could not create $CFG_DIR"
  local tmpf
  tmpf=$(mktemp "$CFG_DIR/.3proxy.cfg.XXXXXX") || die_log "Could not write config"
  cat >"$tmpf" <<EOF
nserver 1.1.1.1
nserver 8.8.8.8
nscache 65536
maxconn 5000
timeouts 1 5 30 60 180 1800 15 60
users $LOGIN:CL:$PASS
auth strong
allow $LOGIN
proxy -p$HTTP_PORT -a
socks -p$SOCKS_PORT
EOF
  chown "root:$SVC_USER" "$tmpf" && chmod 640 "$tmpf" && mv -f "$tmpf" "$CFG" ||
    die_log "Could not write $CFG"

  tmpf=$(mktemp "$CFG_DIR/.install.state.XXXXXX") || die_log "Could not write state file"
  cat >"$tmpf" <<EOF
LOGIN=$LOGIN
PASSWORD=$PASS
HTTP_PORT=$HTTP_PORT
SOCKS_PORT=$SOCKS_PORT
EOF
  chmod 600 "$tmpf" && mv -f "$tmpf" "$STATE" || die_log "Could not write $STATE"

  cat >"$UNIT" <<EOF
[Unit]
Description=3proxy HTTP + SOCKS5 proxy (Clod Helper)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=0

[Service]
Type=simple
User=$SVC_USER
Group=$SVC_USER
ExecStart=$BIN $CFG
Restart=always
RestartSec=2
LimitNOFILE=1048576
TasksMax=infinity

[Install]
WantedBy=multi-user.target
EOF
  chmod 644 "$UNIT"

  if [ "${CLOD_NO_SYSTEMD:-0}" = 1 ]; then
    # Test mode for containers without systemd. Not used on a real server.
    pkill -x 3proxy >/dev/null 2>&1
    sleep 1
    setsid runuser -u "$SVC_USER" -- "$BIN" "$CFG" </dev/null >>"$LOG" 2>&1 &
  else
    run systemctl daemon-reload || warn "systemctl daemon-reload failed."
    run systemctl enable 3proxy || warn "Could not enable 3proxy autostart."
    run systemctl restart 3proxy || warn "3proxy service did not start (see below)."
  fi

  # ---------- 4. network tuning ----------
  step 4 "Tuning network (BBR, limits)..."
  cat >"$SYSCTL_FILE" <<'EOF'
# Clod Helper proxy tuning
net.core.default_qdisc = fq
net.ipv4.tcp_congestion_control = bbr
net.core.somaxconn = 65535
net.core.netdev_max_backlog = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.core.rmem_max = 67108864
net.core.wmem_max = 67108864
net.ipv4.tcp_rmem = 4096 131072 67108864
net.ipv4.tcp_wmem = 4096 65536 67108864
net.ipv4.tcp_mtu_probing = 1
net.ipv4.tcp_fastopen = 3
net.ipv4.tcp_slow_start_after_idle = 0
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 15
net.ipv4.tcp_keepalive_time = 600
net.ipv4.tcp_keepalive_intvl = 30
net.ipv4.tcp_keepalive_probes = 5
net.ipv4.ip_local_port_range = 10000 65535
net.ipv4.tcp_max_tw_buckets = 2000000
fs.file-max = 2097152
fs.nr_open = 2097152
EOF
  echo tcp_bbr >"$BBR_FILE"
  cat >"$LIMITS_FILE" <<'EOF'
*    soft nofile 1048576
*    hard nofile 1048576
root soft nofile 1048576
root hard nofile 1048576
EOF
  run modprobe tcp_bbr || log "modprobe tcp_bbr failed (not fatal)"
  run sysctl -e --system || log "sysctl --system reported errors (not fatal)"

  # ---------- 5. firewall ----------
  step 5 "Opening ports in firewall (ufw)..."
  if command -v ufw >/dev/null 2>&1; then
    local fw_ok=1
    for p in $(ssh_ports); do
      run ufw allow "$p/tcp" || fw_ok=""
    done
    run ufw allow "$HTTP_PORT/tcp" || fw_ok=""
    run ufw allow "$SOCKS_PORT/tcp" || fw_ok=""
    for p in $o_http $o_socks; do
      if [ "$p" != "$HTTP_PORT" ] && [ "$p" != "$SOCKS_PORT" ] && ! is_ssh_port "$p"; then
        run ufw delete allow "$p/tcp" || log "could not delete old rule $p/tcp (not fatal)"
      fi
    done
    run ufw --force enable || fw_ok=""
    if [ -n "$fw_ok" ]; then
      say "    SSH (22) and ports $HTTP_PORT, $SOCKS_PORT are open."
    else
      warn "ufw firewall setup had errors. If the proxy is not reachable, open TCP ports $HTTP_PORT and $SOCKS_PORT manually."
    fi
  else
    warn "ufw not found, firewall not configured."
  fi

  # ---------- 6. self-test ----------
  step 6 "Testing the proxy..."
  for i in $(seq 1 20); do
    port_busy "$HTTP_PORT" && port_busy "$SOCKS_PORT" && break
    sleep 0.5
  done
  local http_ip="" socks_ip="" ip http_res socks_res
  http_ip=$(fetch_ip -x "http://$LOGIN:$PASS@127.0.0.1:$HTTP_PORT") || http_ip=""
  socks_ip=$(fetch_ip -x "socks5h://$LOGIN:$PASS@127.0.0.1:$SOCKS_PORT") || socks_ip=""
  ip=${http_ip:-$socks_ip}
  [ -n "$ip" ] || ip=$(fetch_ip -4 --noproxy '*') || ip=""
  if [ -n "$http_ip" ]; then http_res=OK; else http_res=FAILED; fi
  if [ -n "$socks_ip" ]; then socks_res=OK; else socks_res=FAILED; fi
  log "self-test: http=$http_res socks=$socks_res ip=${ip:-unknown}"
  local show_ip=${ip:-YOUR_SERVER_IP}

  say ""
  say "=================================================="
  if [ "$http_res" = OK ] && [ "$socks_res" = OK ]; then
    say "  DONE! Proxy is working.  Server IP: $show_ip"
  else
    say "  FAILED! Proxy test did not pass."
  fi
  say "  HTTP   : $http_res     SOCKS5 : $socks_res"
  say ""
  say "  Login    : $LOGIN"
  say "  Password : $PASS"
  say "  HTTP port: $HTTP_PORT    SOCKS5 port: $SOCKS_PORT"
  say ""
  say "  Clod Helper string (HTTP):"
  say "  $show_ip:$HTTP_PORT:$LOGIN:$PASS"
  say "  SOCKS5:"
  say "  socks5://$LOGIN:$PASS@$show_ip:$SOCKS_PORT"
  say "=================================================="

  if [ "$http_res" = OK ] && [ "$socks_res" = OK ]; then
    say "  To show these lines again, run the same command again."
    log "===== finished OK ====="
    return 0
  fi

  say ""
  say "Last 20 lines of the 3proxy service log:"
  service_logs | tail -n 20 | LC_ALL=C tr -c '\n\40-\176' '?'
  say ""
  if [ -z "$ip" ]; then
    say "HINT: this server could not reach the internet at all. Check the VPS network/DNS."
  else
    say "HINT: run 'systemctl status 3proxy' and check the ports are not used by other programs."
    say "      Running this installer again often fixes it."
  fi
  say "Install log: $LOG"
  log "===== finished with FAILED self-test ====="
  exit 1
}

main "$@"
