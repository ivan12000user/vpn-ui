# pl-vmpico — HTTPS and login protection (VPN UI 0.8.7)

This directory is the **non-secret restoration recipe for the OLD VPS**.
It intentionally does not change the new VPS or the main VPN UI deployment.

## Deployed state (verified 2026-11-05)

- Public entry point: `https://193.247.82.202/` on TCP 443 (IPv4 and IPv6).
- Nginx proxies to `http://127.0.0.1:8090` with mandatory HTTP Basic Auth.
- Fail2ban jail `vpn-ui-auth`: 5 bad passwords in 600 seconds = 3600-second IP ban; only TCP 443, IPv4 and IPv6.
- WireGuard `wg0`: UDP 443 and 18 existing peers; AWG 3.1 `awg1`: UDP 8444; Tailscale also runs independently.
- The history timer and VPN configuration are **out of scope** for this security-only restore.
- The original Nginx site filename still contains `5443` for historical compatibility. Its **listener is 443**, not 5443.

## Tracked files

- `nginx/vpn-ui-443.conf`: Nginx HTTPS, Basic Auth, dedicated auth error log, loopback proxy.
- `fail2ban/vpn-ui-auth.local`: standalone jail using the Ubuntu-provided `nginx-http-auth` filter.
- `install-security.sh`: controlled, idempotent security restore and read-only verification.

**Never commit** `*.htpasswd`, TLS `*.key`, generated `*.crt` with private-key archives, WireGuard/AWG configs or peer keys, client profiles, token files, complete server backups, or auth logs. The repository is public.

## Read-only verification on pl-vmpico

First clone/fetch the specific branch, then run as root:

```bash
git clone --branch deploy/pl-vmpico-v087-no-awg2 \
  https://github.com/ivan12000user/vpn-ui.git /root/vpn-ui-security-recovery
cd /root/vpn-ui-security-recovery
bash -n deploy/pl-vmpico/install-security.sh
bash deploy/pl-vmpico/install-security.sh --check
```

If the checkout already exists, update it instead of cloning again. `--check` does not write configuration or restart services. It checks Nginx, Fail2ban, the Basic Auth 401 response and IPv4/IPv6 firewall hooks.

## Restore security (explicit opt-in)

Prerequisites: root access, the running `vpn-ui` systemd service bound to `127.0.0.1:8090`, a working Nginx installation, `openssl`, `curl`, `iptables`, `ip6tables`, `ss`, and `sudo` as appropriate. On a new host, prepare those prerequisites first; this is **not** a complete VPN installer.

```bash
cd /root/vpn-ui-security-recovery
bash deploy/pl-vmpico/install-security.sh --apply
```

The script:

1. Refuses conflicting Nginx site symlinks or an already-occupied TCP 443 when this site is not enabled.
2. Backs up existing Nginx configuration, htpasswd, Fail2ban jail, and TLS material under a **root-only directory**.
3. Keeps an existing valid certificate/private-key pair; otherwise creates a self-signed certificate with SANs for this VPS's known addresses.
4. Keeps an existing htpasswd; if absent, asks for a new password of at least eight characters, entered twice without terminal echo. No plaintext password is written to a file or printed.
5. Installs the templates, checks their syntax, restarts **Nginx only**, and reloads/enables Fail2ban.
6. Checks the 401 response without credentials and confirms IPv4/IPv6 Fail2ban hooks; attempts to roll back changed files if a validation fails.

Nginx's brief restart can interrupt other websites on the same host. WireGuard, AWG3.1, Tailscale, and their peer definitions are never restarted or modified by this script.

To **rotate the UI password intentionally**, rerun the script with:

```bash
VPN_UI_RESET_PASSWORD=1 bash deploy/pl-vmpico/install-security.sh --apply
```

Use an interactive terminal. A random password longer than eight characters is more secure than the allowed minimum.

## Operating commands

```bash
fail2ban-client status vpn-ui-auth
fail2ban-client get vpn-ui-auth maxretry
fail2ban-client get vpn-ui-auth findtime
fail2ban-client get vpn-ui-auth bantime
# Unban only a specific verified administrative IP:
fail2ban-client set vpn-ui-auth unbanip 203.0.113.10
```

The address in the last line is a **documentation-only** example; replace it before use.

The HTTPS certificate is currently **self-signed**, and browsers will show a trust warning until the operator deploys a properly trusted certificate. Fail2ban limits repeated password guesses but is not a replacement for strong passwords, trusted TLS, or restricting public administrative access.

### Disaster recovery boundaries

The public Git repository contains **templates and code only**. To restore the exact previous state, keep separate encrypted/offline backups of VPN configurations, private keys, client inventory, passwords, and TLS assets. Do not upload these backups to GitHub. Production recovery should be tested on a staging host before relying on this script.
