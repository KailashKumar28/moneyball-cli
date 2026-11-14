import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import worker from "../src/index.js";

const SECRET = "app-secret-0123456789";
const TOKEN = "EAAtoken0123456789";
const CODE = "code-0123456789";
const STATE = "A".repeat(20) + "b_c-" + "z".repeat(19); // 43 chars, base64url
const ENV = {
  META_APP_ID: "1234",
  META_APP_SECRET: SECRET,
  META_GRAPH_VERSION: "v26.0",
  META_REDIRECT_URI: "https://broker.example/meta/callback",
};

const realFetch = globalThis.fetch;
const realConsole = { ...console };
let calls; // what the Worker sent to "Meta"
let logged; // everything written to the console

beforeEach(() => {
  calls = [];
  logged = [];
  for (const k of ["log", "info", "warn", "error", "debug", "trace"]) {
    console[k] = (...args) => logged.push(args.map(String).join(" "));
  }
});
afterEach(() => {
  globalThis.fetch = realFetch;
  Object.assign(console, realConsole);
  const all = logged.join("\n");
  for (const s of [SECRET, TOKEN, CODE]) assert.ok(!all.includes(s), "a secret reached the console");
});

function meta(status, body) {
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    if (status === "down") throw new TypeError("fetch failed");
    return new Response(typeof body === "string" ? body : JSON.stringify(body), { status });
  };
}

function get(query, { method = "GET", path = "/meta/callback" } = {}) {
  return worker.fetch(new Request(`https://broker.example${path}?${query}`, { method }), ENV);
}

const cb = (extra) => `state=${encodeURIComponent(`8123.${STATE}`)}&${extra}`;

// The hidden fields the page POSTs, and the page's form action. The exact
// tag (no enctype) means application/x-www-form-urlencoded.
function form(html) {
  const action = html.match(/<form method="post" action="([^"]+)">/)?.[1];
  const fields = Object.fromEntries([...html.matchAll(/name="(\w+)" value="([^"]*)"/g)].map((m) => [m[1], m[2]]));
  return { action, fields };
}

test("happy path: the token reaches the POST page and no URL", async () => {
  meta(200, { access_token: TOKEN, token_type: "bearer" });
  const res = await get(cb(`code=${CODE}`));
  const html = await res.text();

  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.equal(res.headers.get("referrer-policy"), "no-referrer");
  assert.equal(res.headers.get("location"), null);
  const { action, fields } = form(html);
  assert.equal(action, "http://127.0.0.1:8123/meta/done");
  // state is only the part after the port; exactly these two fields.
  assert.deepEqual(fields, { token: TOKEN, state: STATE });
  assert.equal(STATE.length, 43);
  assert.equal((html.match(/<input /g) ?? []).length, 2);
  for (const [, url] of html.matchAll(/(?:action|href|src)="([^"]*)"/g)) assert.ok(!url.includes(TOKEN));
  for (const [, v] of res.headers) assert.ok(!v.includes(TOKEN));

  const csp = res.headers.get("content-security-policy");
  const nonce = html.match(/<script nonce="([^"]+)">/)[1];
  assert.match(csp, new RegExp(`script-src 'nonce-${nonce}'`));
  assert.match(csp, /form-action http:\/\/127\.0\.0\.1:8123;/);
  assert.match(csp, /default-src 'none'/);
  assert.match(html, /<button type="submit">/); // the fallback with scripts off

  // The swap: POST to Meta's token endpoint, secret only in the body.
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://graph.facebook.com/v26.0/oauth/access_token");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(Object.fromEntries(calls[0].init.body), {
    client_id: "1234",
    client_secret: SECRET,
    redirect_uri: ENV.META_REDIRECT_URI,
    code: CODE,
  });
  assert.ok(!html.includes(SECRET));
});

test("an error from Meta is passed on in words, without the secret or the code", async () => {
  meta(400, { error: { message: `Invalid code ${CODE} for secret ${SECRET}.`, type: "OAuthException", code: 100 } });
  const res = await get(cb(`code=${CODE}`));
  const html = await res.text();
  const { action, fields } = form(html);

  assert.equal(action, "http://127.0.0.1:8123/meta/done");
  assert.deepEqual(fields, {
    error: "Meta did not accept the sign-in: Invalid code [hidden] for secret [hidden].",
    state: STATE,
  });
  assert.ok(!html.includes(SECRET) && !html.includes(CODE));
});

test("a non-JSON answer from Meta is said with its status", async () => {
  meta(502, "<html>Bad gateway</html>");
  const { fields } = form(await (await get(cb(`code=${CODE}`))).text());
  assert.equal(fields.error, "Meta answered with HTTP 502 and no details. Try again.");
});

test("the user declining is forwarded, and Meta is not called", async () => {
  meta(200, {});
  const res = await get(cb("error=access_denied&error_reason=user_denied&error_description=Permissions+error"));
  const { action, fields } = form(await res.text());

  assert.equal(calls.length, 0);
  assert.equal(action, "http://127.0.0.1:8123/meta/done");
  assert.deepEqual(fields, { error: "Facebook sign-in was cancelled, so nothing was connected.", state: STATE });
});

test("another error from the dialog is forwarded with Meta's words", async () => {
  meta(200, {});
  const { fields } = form(await (await get(cb("error=server_error&error_description=Try+later"))).text());
  assert.equal(fields.error, "Facebook did not finish signing in: Try later");
});

test("a missing code is an error, not a swap", async () => {
  meta(200, {});
  const { fields } = form(await (await get(cb(""))).text());
  assert.equal(calls.length, 0);
  assert.equal(fields.error, "Facebook did not send an authorization code. Try again.");
});

test("Meta unreachable is said in words", async () => {
  meta("down");
  const { fields } = form(await (await get(cb(`code=${CODE}`))).text());
  assert.equal(fields.error, "Could not reach Meta to finish connecting. Try again in a minute.");
});

test("a missing secret is named, not guessed", async () => {
  meta(200, { access_token: TOKEN });
  const res = await worker.fetch(
    new Request(`https://broker.example/meta/callback?${cb(`code=${CODE}`)}`),
    { ...ENV, META_APP_SECRET: undefined },
  );
  const { fields } = form(await res.text());
  assert.equal(calls.length, 0);
  assert.equal(fields.error, "The Moneyball broker is not set up: META_APP_SECRET missing.");
});

test("bad ports and bad states are rejected without a form or a swap", async () => {
  meta(200, { access_token: TOKEN });
  const bad = [
    "", "8123", `.${STATE}`, `80.${STATE}`, `1023.${STATE}`, `65536.${STATE}`, `08123.${STATE}`,
    `abc.${STATE}`, `8123.short`, `8123.${STATE.slice(1)}`, `8123.${STATE}a`, `8123.${STATE.slice(1)}+`,
    `8123.${STATE}.x`,
    `8123.${STATE}"><script>`,
  ];
  for (const state of bad) {
    const res = await get(`state=${encodeURIComponent(state)}&code=${CODE}`);
    const html = await res.text();
    assert.equal(res.status, 400, state);
    assert.equal(res.headers.get("cache-control"), "no-store");
    assert.match(res.headers.get("content-security-policy"), /form-action 'none'/);
    assert.ok(!html.includes("<form") && !html.includes("<script"), state);
  }
  assert.equal(calls.length, 0);
});

test("the edges of the port range are accepted", async () => {
  meta(200, { access_token: TOKEN });
  for (const port of [1024, 65535]) {
    const { action } = form(await (await get(`state=${port}.${STATE}&code=${CODE}`)).text());
    assert.equal(action, `http://127.0.0.1:${port}/meta/done`);
  }
});

test("everything else is a 404", async () => {
  meta(200, { access_token: TOKEN });
  for (const [path, method] of [["/", "GET"], ["/meta", "GET"], ["/meta/callback/", "GET"], ["/meta/callback", "POST"], ["/meta/callback", "HEAD"]]) {
    const res = await get(cb(`code=${CODE}`), { path, method });
    assert.equal(res.status, 404, `${method} ${path}`);
  }
  assert.equal(calls.length, 0);
});
