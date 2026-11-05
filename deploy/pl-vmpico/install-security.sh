#!/usr/bin/env bash
# Restore only HTTPS Basic Auth and Fail2ban for the pre-existing vpn-ui service.
# No VPN interface, WireGuard peer, AWG key, or Tailscale configuration is touched.
set -Eeuo pipefail
umask 077

if [ "$#" -ne 1 ] || { [ "$1" != "--check" ] && [ "$1" != "--apply" ]; }; then
    echo "Usage: sudo bash deploy/pl-vmpico/install-security.sh --check|--apply" >&2
    exit 64
fi

MODE="$1"
ROOT="$(cd "$(dirname "$0")" && pwd)"
SITE=/etc/nginx/sites-available/vpn-ui-ts-5443.conf
LINK=/etc/nginx/sites-enabled/vpn-ui-ts-5443.conf
CERTDIR=/etc/nginx/ssl/vpn-ui-old
CERT="$CERTDIR/server.crt"
KEY="$CERTDIR/server.key"
AUTH=/etc/nginx/vpn-ui-old-admin.htpasswd
JAIL=/etc/fail2ban/jail.d/vpn-ui-auth.local
LOG=/var/log/nginx/vpn-ui-auth-error.log

[ "$(id -u)" -eq 0 ] || { echo "Run as root" >&2; exit 1; }
[ -s "$ROOT/nginx/vpn-ui-443.conf" ]
[ -s "$ROOT/fail2ban/vpn-ui-auth.local" ]

check() {
    echo "===== SECURITY CHECK (READ ONLY) ====="
    for f in "$SITE" "$CERT" "$KEY" "$AUTH" "$JAIL"; do
        [ -s "$f" ] || { echo "Missing: $f" >&2; exit 1; }
    done
    [ -L "$LINK" ] && [ "$(readlink -f "$LINK")" = "$SITE" ]
    grep -Fq 'listen 0.0.0.0:443 ssl;' "$SITE"
    grep -Fq 'listen [::]:443 ssl;' "$SITE"
    grep -Fq 'auth_basic "VPN UI";' "$SITE"
    nginx -t
    fail2ban-client -t
    systemctl is-active --quiet nginx
    systemctl is-active --quiet fail2ban
    systemctl is-active --quiet vpn-ui
    fail2ban-client status vpn-ui-auth
    [ "$(curl --noproxy '*' -ksS --connect-timeout 3 --max-time 8 \
        -o /dev/null -w '%{http_code}' https://127.0.0.1/healthz)" = 401 ]
    iptables -S INPUT | grep -F 'f2b-vpn-ui-auth'
    ip6tables -S INPUT | grep -F 'f2b-vpn-ui-auth'
    echo "HTTPS TCP 443: 401 without credentials"
    echo "Fail2ban IPv4 and IPv6 firewall hooks: present"
    echo "SECURITY CHECK: OK"
}

if [ "$MODE" = "--check" ]; then
    check
    exit 0
fi

echo "===== PREFLIGHT ====="
systemctl is-active --quiet nginx
systemctl is-active --quiet vpn-ui
for cmd in openssl curl iptables ip6tables ss nginx; do
    command -v "$cmd" >/dev/null
done

if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
    echo "Refusing to replace an existing non-symlink Nginx site" >&2
    exit 1
fi
if [ -L "$LINK" ] && [ "$(readlink -f "$LINK")" != "$SITE" ]; then
    echo "Refusing to replace another Nginx site symlink" >&2
    exit 1
fi
if [ ! -L "$LINK" ] &&
   ss -H -lnt | awk '$4 ~ /:443$/ { busy=1 } END { exit !busy }'; then
    echo "TCP 443 is in use but this site is not enabled; refusing" >&2
    exit 1
fi

if ! command -v fail2ban-client >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y fail2ban
fi
[ -f /etc/fail2ban/filter.d/nginx-http-auth.conf ]

BACKUP="$(mktemp -d /root/vpn-ui-security-backup-XXXXXXXX)"
snapshot() {
    if [ -e "$1" ]; then cp -a -- "$1" "$BACKUP/$2"
    else : > "$BACKUP/$2.absent"; fi
}
restore() {
    if [ -e "$BACKUP/$2.absent" ]; then rm -f -- "$1"
    else cp -a -- "$BACKUP/$2" "$1"; fi
}
snapshot "$SITE" site
snapshot "$AUTH" auth
snapshot "$JAIL" jail
snapshot "$CERT" cert
snapshot "$KEY" key
if [ -L "$LINK" ]; then : > "$BACKUP/link.existed"
else : > "$BACKUP/link.absent"; fi

ROLLBACK=1
on_exit() {
    rc=$?
    trap - EXIT
    if [ "$ROLLBACK" -eq 1 ]; then
        echo "FAILED: restoring previous Nginx/Fail2ban configuration" >&2
        restore "$SITE" site
        restore "$AUTH" auth
        restore "$JAIL" jail
        restore "$CERT" cert
        restore "$KEY" key
        if [ -e "$BACKUP/link.absent" ]; then rm -f "$LINK"; fi
        nginx -t && systemctl restart nginx || true
        if systemctl is-active --quiet fail2ban; then fail2ban-client reload || true; fi
    fi
    exit "$rc"
}
trap on_exit EXIT

echo "===== TLS CERTIFICATE ====="
install -d -o root -g root -m 0700 "$CERTDIR"
if [ ! -e "$CERT" ] && [ ! -e "$KEY" ]; then
    openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
      -subj '/CN=pl-vmpico' \
      -addext 'subjectAltName=IP:193.247.82.202,IP:100.88.88.100,IP:10.66.66.1,IP:10.88.88.1,IP:127.0.0.1' \
      -keyout "$KEY" -out "$CERT" >/dev/null 2>&1
    chmod 0600 "$KEY"
    chmod 0644 "$CERT"
    echo "Created a self-signed TLS certificate"
else
    [ -s "$CERT" ] && [ -s "$KEY" ] || {
        echo "Incomplete TLS certificate/key pair; refusing to overwrite" >&2; exit 1;
    }
    echo "Kept existing TLS certificate and key"
fi

PASSWORD_CHANGED=0
echo "===== BASIC AUTH ====="
if [ ! -s "$AUTH" ] || [ "$(printenv VPN_UI_RESET_PASSWORD || true)" = 1 ]; then
    IFS= read -r -s -p 'New admin password (8+ characters): ' PASS </dev/tty
    echo
    IFS= read -r -s -p 'Repeat password: ' CONFIRM </dev/tty
    echo
    size="$(printf %s "$PASS" | wc -c)"
    [ "$size" -ge 8 ] && [ "$PASS" = "$CONFIRM" ] || {
        echo "Password too short or does not match" >&2; exit 1;
    }
    unset CONFIRM
    HASH="$(printf %s "$PASS" | openssl passwd -6 -stdin)"
    TMP="$(mktemp /etc/nginx/.vpn-ui-auth.XXXXXXXX)"
    printf 'admin:%s\n' "$HASH" > "$TMP"
    chown root:www-data "$TMP"
    chmod 0640 "$TMP"
    mv -f "$TMP" "$AUTH"
    unset HASH
    PASSWORD_CHANGED=1
    echo "Password hash updated; plaintext not stored"
else
    echo "Kept existing htpasswd"
fi

echo "===== INSTALL NGINX / FAIL2BAN ====="
if [ ! -e "$LOG" ]; then install -o root -g adm -m 0640 /dev/null "$LOG"; fi
install -o root -g root -m 0644 "$ROOT/nginx/vpn-ui-443.conf" "$SITE"
if [ ! -L "$LINK" ]; then ln -s "$SITE" "$LINK"; fi
install -o root -g root -m 0644 "$ROOT/fail2ban/vpn-ui-auth.local" "$JAIL"

nginx -t
fail2ban-client -t
systemctl restart nginx
if systemctl is-active --quiet fail2ban; then fail2ban-client reload
else systemctl enable --now fail2ban; fi
systemctl enable fail2ban >/dev/null

echo "===== VERIFY ====="
check
if [ "$PASSWORD_CHANGED" -eq 1 ]; then
    code="$(curl --noproxy '*' -ksS --connect-timeout 3 --max-time 8 \
        -u "admin:$PASS" -o /dev/null -w '%{http_code}' \
        https://127.0.0.1/healthz)"
    [ "$code" = 200 ] || { echo "New login returned HTTP $code" >&2; exit 1; }
    unset PASS
    echo "New password accepted: HTTP 200"
fi

ROLLBACK=0
trap - EXIT
echo "SECURITY RESTORE: OK"
echo "URL: https://193.247.82.202/"
echo "Private backup (never upload): $BACKUP"
