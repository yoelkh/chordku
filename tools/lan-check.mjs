// Checks LAN mode the way a phone browser talks to it: over HTTP/2, where the host name arrives
// as `:authority` and there is no `Host` header. curl builds without HTTP/2 cannot test this.
//
//   node tools/lan-check.mjs 192.168.1.20:7465 <pairing code from the QR card>
//
// The certificate is not verified here (the local CA is usually not installed on the laptop's
// own Node); the TLS chain is covered separately in docs/LAN.md.
import http2 from "node:http2";

const [host, code] = process.argv.slice(2);
if (!host || !code) {
  console.error("usage: node tools/lan-check.mjs <ip:port> <pairing code>");
  process.exit(2);
}

const origin = `https://${host}`;
const client = http2.connect(origin, { rejectUnauthorized: false });
client.on("error", (e) => {
  console.error("connection failed:", e.message);
  process.exit(1);
});

const call = (headers, body) =>
  new Promise((resolve) => {
    const req = client.request(headers);
    let head;
    let text = "";
    req.on("response", (h) => (head = h));
    req.on("data", (d) => (text += d));
    req.on("end", () => resolve({ status: head?.[":status"], head, text }));
    req.end(body);
  });

let failed = 0;
const expect = (label, got, want) => {
  const ok = got === want;
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${label}: ${got}${ok ? "" : ` (expected ${want})`}`);
};

const json = { ":method": "POST", "content-type": "application/json" };

const anon = await call({ ":path": "/api/songs" });
expect("library without a device cookie", anon.status, 401);

const pair = await call({ ...json, ":path": "/api/pair", origin }, JSON.stringify({ code, name: "lan-check" }));
expect("pairing with the one-time code", pair.status, 200);
const cookie = (pair.head?.["set-cookie"] ?? [""])[0].split(";")[0];

const songs = await call({ ":path": "/api/songs", cookie });
expect("library with the device cookie", songs.status, 200);

const again = await call({ ...json, ":path": "/api/pair", origin }, JSON.stringify({ code, name: "lan-check" }));
expect("pairing code is single use", again.status === 200 ? 200 : "refused", "refused");

const admin = await call({ ":path": "/api/system", cookie });
expect("system API from the LAN", admin.status, 403);

const foreign = await call({ ...json, ":path": "/api/pair", origin: "https://evil.example" }, "{}");
expect("foreign Origin", foreign.status, 403);

client.close();
console.log(failed ? `${failed} check(s) failed` : "all checks passed; revoke the 'lan-check' device in Pengaturan > Akses dari HP");
process.exit(failed ? 1 : 0);
