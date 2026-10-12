# Tailscale users and device-share invites — read-only preview

This feature is developed on `feature/ts-shared-users-activity` and must **not** be
installed over production v0.8.0 until CI, manual verification, and rollback checks.

## Sources and semantics

* `GET /api/v2/tailnet/{tailnet}/users`: user type (member/shared),
  `currentlyConnected` and `lastSeen`. This is *not* exit-node traffic.
* `GET /api/v2/tailnet/{tailnet}/devices`: owned device inventory.
* `GET /api/v2/device/{id}/device-invites`: per-device share invites.
  `acceptedBy.loginName` and `allowExitNode` are extracted when available.
  **Never return or log `inviteUrl`**, a bearer capability.
* One recipient may have accepted several device shares. The UI groups
  those *accepted invite records* by login name, without claiming that the
  current ACL/grants or share still allow access. Member access depends
  on policy, not on invite rows.
* An absent invite row is NOT proof of no access. The backend reports
  partial errors (including permissions issues) instead of silently reporting
  a complete empty list.

## Configuration, no secrets in systemd Environment=

The integration is disabled unless **both** values are present:

* `VPN_UI_TS_TAILNET`: your tailnet identifier/domain.
* `VPN_UI_TS_API_TOKEN_FILE`: readable *file path*, never the token value.

Example **for later, after tests**, using a systemd credential:

1. Obtain a Tailscale API token from the admin console.
   API tokens expire; configure key rotation. Prefer a scoped OAuth design
   for a future unattended long-term production integration.
2. Put the token in a root-owned `0600` file under `/etc/vpn-ui/`.
   Do not paste it into chats, GitHub, process command lines, environment
   values or curl diagnostics.
3. Add a dedicated drop-in to `vpn-ui.service`:

   ```ini
   [Service]
   LoadCredential=ts_admin_api_token:/etc/vpn-ui/ts_admin_api_token
   Environment=VPN_UI_TS_TAILNET=example.ts.net
   Environment=VPN_UI_TS_API_TOKEN_FILE=/run/credentials/vpn-ui.service/ts_admin_api_token
   ```

4. Only after a separate production-change review, reload systemd and
   restart the UI, *not* any VPN tunnel.

A credential restricted to GET endpoints is not generally enforced with a
normal personal API token; this implementation only sends GET requests.
For future scoped OAuth credentials, required endpoint scopes are
`users:read`, `devices:core:read`, `devices_invites:read`.

## Safety

* Backend HTTPS host fixed to `api.tailscale.com`; no user-provided URL.
* Results are cached for 180 seconds; requests for device invites run at
  concurrency 4, with a 64-device inventory cap and explicit warning.
* Request timeout is 8 seconds. Only supported fields are copied to API
  JSON; the raw upstream response is never returned.
* HTML uses DOM textContent, not untrusted innerHTML.
* New JSON endpoint `/api/tailscale/shares` uses no-store, but still
  requires the **existing authenticated Nginx perimeter** to avoid exposing
  account email addresses. Do not expose port 8090 publicly.
* Statuses: `currentlyConnected` is not equivalent to traffic through an
  exit node. Invite acceptance is not proof of ongoing access.
