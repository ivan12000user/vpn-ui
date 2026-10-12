# Tailscale user activity and open device-share invites — read-only preview

This feature is developed on `feature/ts-shared-users-activity`. Do not
install it over production v0.8.0 without isolated testing and rollback.

## Data semantics

- `GET /api/v2/tailnet/{tailnet}/users` supplies user types (including
  `shared`), `currentlyConnected`, and `lastSeen` when available.
  This is not proof of exit-node traffic.
- `GET /api/v2/tailnet/{tailnet}/devices` lists tailnet devices.
- `GET /api/v2/device/{id}/device-invites` lists *open* (not yet accepted)
  device invitations. The API does **not** provide a complete record of
  accepted grants; an absent row does not mean no current access.
- Each external user can receive access to multiple devices. This endpoint
  does not establish the full mapping of already accepted shares. Do not
  invent those mappings in the UI.
- Access for tailnet members depends on grants/ACL policy, not invitations.

## Least-privileged authentication (preferred: OAuth client)

Create a Tailscale OAuth client under the tailnet's Trust credentials
with **exact** read-only scopes:

- `users:read`
- `devices:core:read`
- `devices_invites:read`

OAuth access tokens expire after 1 hour; the Rust app obtains a fresh token
and caches it for up to 45 minutes. The secret stays in a credential file.

Configuration for a **future isolated validation service**:

```ini
[Service]
LoadCredential=ts_oauth_secret:/etc/vpn-ui/ts_oauth_secret
Environment=VPN_UI_TS_TAILNET=-
Environment=VPN_UI_TS_OAUTH_CLIENT_ID=your-client-id
Environment=VPN_UI_TS_OAUTH_SECRET_FILE=/run/credentials/YOUR-TEST-SERVICE.service/ts_oauth_secret
```

Use tailnet `-` to refer to the tailnet owning the token; this avoids
confusing display name and legacy ID with the current Tailnet ID. Never
write a credential to a Git repository, debug log, shell history, argv,
or a plain environment variable.

For one-off testing, `VPN_UI_TS_API_TOKEN_FILE` may point to a
credential file, but personal API tokens have broader permission and
expire after 1–90 days. Do not make one for long-term unattended use.
OAuth settings take priority over the API-token fallback.

The current production `vpn-ui.service` and its VPN interfaces must not
be modified as part of credential provisioning.

## Security

- No privileged Tailscale mutations are performed; only GET to data APIs,
  and POST to the fixed HTTPS OAuth token exchange endpoint.
- Rust copies a small set of fields to JSON. Secret invite URLs, raw
  upstream responses, OAuth secrets and access tokens never go to clients.
- Cache 180 seconds; at most four simultaneous device queries and 64
  devices per snapshot. Incomplete responses are marked.
- New endpoint `/api/tailscale/shares` carries personal identifiers.
  It must be protected by existing nginx authentication; the Rust
  listener should stay bound to `127.0.0.1`.
- This directory is not a live traffic analyzer, permission evaluator,
  or replacement for Tailscale Admin Console.

## Invite request completeness

The JSON snapshot includes `invite_checks` and `invite_checks_ok` for device
queries. An empty invite list is only confirmed when **every device request
returns a valid list**. HTTP 404, 403 or other errors must be shown as
*unavailable/incomplete*, not as zero accepted grants or zero invitations.
