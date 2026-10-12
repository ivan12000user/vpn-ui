#!/usr/bin/env python3
"""Read-only Tailscale API diagnostics. Prints only status/counts, no identities or secrets."""
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://api.tailscale.com/api/v2/"
ID_PATH = Path("/etc/vpn-ui/ts-oauth-id")
SECRET_PATH = Path("/etc/vpn-ui/ts-oauth-secret")
TIMEOUT = 15


def fetch(path, token):
    request = urllib.request.Request(
        API + path, headers={"Authorization": "Bearer " + token}
    )
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            payload = response.read(5_000_000)
            try:
                obj = json.loads(payload)
            except (ValueError, UnicodeDecodeError):
                return response.status, None, "invalid-json"
            return response.status, obj, "ok"
    except urllib.error.HTTPError as error:
        # Classify response content without disclosing its text.
        raw = error.read(4096)
        content_type = error.headers.get("Content-Type", "").split(";", 1)[0]
        classification = "other"
        if content_type == "application/json":
            try:
                body = json.loads(raw)
                message = str(body.get("message", "")).lower()
                if "not found" in message:
                    classification = "json-not-found"
                elif "permission" in message or "scope" in message:
                    classification = "json-permission"
                else:
                    classification = "json-error"
            except (ValueError, AttributeError, UnicodeDecodeError):
                classification = "bad-json-error"
        elif content_type == "text/plain":
            classification = "text-error"
        elif content_type == "text/html":
            classification = "html-error"
        return error.code, None, classification
    except (OSError, TimeoutError) as error:
        return 0, None, type(error).__name__


def invite_summary(data):
    if isinstance(data, list):
        return "array", len(data)
    if isinstance(data, dict):
        for name in ("deviceInvites", "invites"):
            if isinstance(data.get(name), list):
                return name, len(data[name])
        return "unexpected-object", "unknown"
    return "unexpected-type", "unknown"


def user_summary(data):
    if isinstance(data, dict):
        users = data.get("users")
    else:
        users = data
    if not isinstance(users, list):
        return "unknown", "unknown", "unknown"
    kinds = [u.get("type") if isinstance(u, dict) else None for u in users]
    return len(users), kinds.count("member"), kinds.count("shared")


def main():
    client_id = ID_PATH.read_text().strip()
    client_secret = SECRET_PATH.read_text().strip()
    if not client_id or not client_secret:
        raise RuntimeError("OAuth files are empty")
    form = urllib.parse.urlencode({
        "grant_type": "client_credentials",
        "client_id": client_id,
        "client_secret": client_secret,
    }).encode()
    request = urllib.request.Request(API + "oauth/token", data=form)
    del client_secret, form
    with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
        oauth = json.load(response)
    token = oauth.get("access_token")
    if not isinstance(token, str) or not token:
        raise RuntimeError("No OAuth access token returned")

    print("===== OAUTH PERMISSIONS =====")
    scopes = oauth.get("scope")
    if isinstance(scopes, str):
        granted = set(scopes.split())
        for scope in ("users:read", "devices:core:read", "devices_invites:read"):
            print(scope, "granted:", scope in granted)
    else:
        print("Scope field not present (verify configured scopes in Tailscale Console)")

    print("===== USER DIRECTORY =====")
    for name, path in (
        ("all", "tailnet/-/users"),
        ("member", "tailnet/-/users?type=member"),
        ("shared", "tailnet/-/users?type=shared"),
    ):
        code, data, kind = fetch(path, token)
        total, members, shared = user_summary(data)
        print(
            f"Users {name}: HTTP={code}, count={total}, "
            f"member={members}, shared={shared}, response={kind}"
        )

    code, devices_doc, kind = fetch("tailnet/-/devices", token)
    print("===== DEVICE INVENTORY =====")
    if code != 200 or not isinstance(devices_doc, dict):
        print(f"Devices: HTTP={code}, response={kind}")
        return 1
    devices = devices_doc.get("devices") or []
    print("Devices:", len(devices))
    devices.sort(key=lambda d: (
        "pl-vmpico-new" not in str(d.get("hostname", "")),
        str(d.get("hostname", "")),
    ))
    print("===== INVITE ENDPOINT =====")
    for i, device in enumerate(devices[:2], 1):
        for key in ("nodeId", "id"):
            device_id = device.get(key)
            if device_id is None:
                print(f"Device {i} identifier {key}: absent")
                continue
            escaped = urllib.parse.quote(str(device_id), safe="")
            code, data, kind = fetch(f"device/{escaped}/device-invites", token)
            shape, count = invite_summary(data)
            print(
                f"Device {i}, {key}: HTTP={code}, response={kind}, "
                f"shape={shape}, open-invites={count}"
            )
    print("===== DONE (NO SECRET VALUES PRINTED) =====")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, urllib.error.HTTPError) as exc:
        print("DIAGNOSTIC ERROR:", type(exc).__name__)
        sys.exit(1)
