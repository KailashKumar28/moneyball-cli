// A stand-in for Moneyball's auth broker (broker/, a Cloudflare Worker) on
// loopback: the broker's side of plans/desktop/auth-plan.md "The Meta
// protocol", step 4, against the stand-in Meta (mock-meta.mjs). The app
// reaches it through MB_E2E_BROKER, read only by the e2e build.
//
// GET /meta/callback?code|error&state=<port>.<state>: the port must be
// 1024 to 65535; a code is swapped for the token with the app secret at
// the stand-in Meta's /<version>/oauth/access_token (POST, the same
// redirect_uri); the answer is a page that POSTs `token` (or `error`, in
// words) and `state` to http://127.0.0.1:<port>/meta/done by itself, so
// the token never sits in a URL. It keeps nothing.
import { createServer } from "node:http";
import { APP_ID, APP_SECRET } from "./mock-meta.mjs";

const VERSION = "v26.0";
const PORT = /^[1-9][0-9]{3,4}$/;
const STATE = /^[A-Za-z0-9_-]{43}$/; // base64url of the app's 32 random bytes

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

/** "<port>.<state>" as [port, state], or [] when either part is off. */
function split(raw) {
  const dot = raw.indexOf(".");
  const [port, state] = [raw.slice(0, dot), raw.slice(dot + 1)];
  const n = Number(port);
  if (dot < 0 || !PORT.test(port) || n < 1024 || n > 65535 || !STATE.test(state)) return [];
  return [n, state];
}

/** The person declined, or Meta said why not: in words. */
function declined(q) {
  if (q.get("error") === "access_denied") return "Facebook sign-in was cancelled, so nothing was connected.";
  return `Facebook did not finish signing in: ${q.get("error_description") || q.get("error")}`;
}

/** The code for the token at the stand-in Meta: { token } or { error }. */
async function exchange(meta, code, redirect_uri) {
  if (!code) return { error: "Facebook did not send an authorization code. Try again." };
  const res = await fetch(`${meta}/${VERSION}/oauth/access_token`, {
    method: "POST",
    body: new URLSearchParams({ client_id: APP_ID, client_secret: APP_SECRET, redirect_uri, code }),
  });
  const body = await res.json().catch(() => null);
  if (typeof body?.access_token === "string" && body.access_token) return { token: body.access_token };
  return { error: `Meta did not accept the sign-in: ${body?.error?.message || `HTTP ${res.status}`}` };
}

const page = (res, status, body) =>
  res
    .writeHead(status, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "referrer-policy": "no-referrer" })
    .end(`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Moneyball</title></head><body>${body}</body></html>`);

/** The page that hands `fields` to the app's loopback listener. */
function handBack(res, port, fields) {
  const inputs = Object.entries(fields)
    .map(([k, v]) => `<input type="hidden" name="${k}" value="${esc(v)}">`)
    .join("");
  page(
    res,
    200,
    `<form method="post" action="http://127.0.0.1:${port}/meta/done">${inputs}` +
      `<p>Returning to Moneyball...</p><button type="submit">Continue</button></form>` +
      `<script>document.forms[0].submit()</script>`,
  );
}

/** Start the stand-in broker on a free loopback port, in front of the stand-in Meta at `meta`. */
export function startMockBroker(meta) {
  let callback;
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, callback);
    if (req.method !== "GET" || url.pathname !== "/meta/callback") return page(res, 404, "<p>Not found.</p>");
    const q = url.searchParams;
    const [port, state] = split(q.get("state") ?? "");
    if (!port) return page(res, 400, "<p>This link did not come from Moneyball. Start again from the Moneyball app.</p>");
    const fields = q.has("error") ? { error: declined(q) } : await exchange(meta, q.get("code"), callback);
    handBack(res, port, { ...fields, state });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const url = `http://127.0.0.1:${server.address().port}`;
      callback = `${url}/meta/callback`;
      resolve({ url, close: () => server.close() });
    });
  });
}
