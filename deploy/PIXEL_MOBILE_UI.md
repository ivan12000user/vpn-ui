# VPN UI v0.8.6 — responsive Pixel/Android optimization

The desktop sidebar retains its existing navigation and layout.
On phone-sized displays (up to 900 CSS px), the seven-link sticky
navigation is replaced with a one-row header and a native HTML
`details/summary` menu, so navigation works without JavaScript.
Keyboard focus is visible; touch targets are 44–46px tall; the dropdown
is scrollable and does not push the graph off the screen.

On the /history page, the protocol and device inputs are side-by-side
on suitable phone widths and the period/refresh controls fit in one row.
The SVG chart uses a 420×270 mobile viewBox (not a shrunk 900px SVG),
three readable timeline ticks, larger axis labels and larger success/
failure dots. This is presentation-only and does not change Ping or
SQLite collection logic.

Deploy `deploy/install-pixel-v086.sh` against the running v0.8.5.
It backs up the binary and reloads only vpn-ui.service. The 5-minute
history timer, database, WG, AWG, Tailscale and Nginx are left untouched.
The installer validates health, responsive page content and services,
and automatically rolls back the binary if verification fails.

To preview without changing production, build this branch and run the
existing localhost smoke tests; offline SVG regression covers both
desktop and Pixel-sized chart viewBoxes.
