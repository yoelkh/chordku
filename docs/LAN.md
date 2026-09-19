# LAN mode (using ChordKu from a phone)

Off by default. When enabled in **Pengaturan > Akses dari HP**, a second listener opens on
`0.0.0.0:<port+1>` (7465 by default), HTTPS only.

## Why HTTPS

Browsers expose `getUserMedia` (the microphone) only in a secure context. `http://192.168.x.y` is
not one, so without TLS the in-page recorder cannot work on a phone. As a fallback that needs no
TLS, the Rekam tab offers the phone's own recorder through `<input type="file" capture>`.

## Certificates

- A private CA is generated on first use and kept in `<data>/tls/`. It is never regenerated, so a
  phone that trusted it once keeps trusting it.
- A leaf certificate for `localhost`, `127.0.0.1` and the laptop's current private IPv4 addresses
  is issued from the CA, valid 365 days. Every 30 s the addresses are checked; if they changed
  (another Wi-Fi, DHCP), a new leaf is issued and hot-reloaded.
- Without installing the CA the browser shows a warning once ("Lanjutan" > "Lanjutkan"); the
  microphone works after that. Installing `https://<ip>:7465/ca.crt` as a CA certificate on
  Android removes the warning.

## Pairing and authorisation

1. The laptop shows a QR code with a one-time link, valid ten minutes.
2. The phone opens it; the server stores only the SHA-256 of a random 256-bit device token and
   sets it as an `HttpOnly; Secure; SameSite=Strict` cookie.
3. Every `/api/*` request from the LAN needs that cookie, except `/api/pair` and `/api/session`.
4. `/api/system/*` (models, yt-dlp, LAN administration) is refused on the LAN listener.
5. Only private-range peers are served, and `Host` must be one of the laptop's own addresses.
6. Devices can be revoked from the same settings card.

## Verified

- TLS 1.3, chain validates against the local CA (Python `ssl`), SANs contain the LAN address.
- 401 without cookie, 403 for `/api/system` and for a foreign `Host`, pairing code single use.
- The same over **HTTP/2**, which is what phone browsers use: there is no `Host` header, the
  name arrives as `:authority`, and the Host allow-list reads either. A curl build without
  HTTP/2 cannot exercise this path, so the repository carries its own client:
  `node tools/lan-check.mjs <ip>:7465 <pairing code>` (401 without cookie, pairing, single-use
  code, 403 for the system API and for a foreign `Origin`).

## Manual phone checklist

- [ ] Windows Firewall prompt: allow for *Private* networks
- [ ] Scan QR, accept the certificate warning, library appears
- [ ] Rekam: record 20 s from a speaker, chart appears
- [ ] Open a song with stems: "2 stem (ringan)" is preselected, playback starts, vocals mute works
- [ ] Revoke the device on the laptop; the phone returns to the pairing screen
