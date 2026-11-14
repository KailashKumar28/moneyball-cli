// Moneyball's auth broker: finishes Meta's Facebook Login for Business for
// the desktop app (plans/desktop/auth-plan.md, "The Meta protocol", step 4).
// Stateless. It never logs or stores the code, the token or the app secret.

const PORT = /^[1-9][0-9]{3,4}$/; // then 1024..65535 by value
const STATE = /^[A-Za-z0-9_-]{43}$/; // base64url of the app's 32 random bytes

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method !== "GET" || url.pathname !== "/meta/callback") {
      return new Response("Not found\n", {
        status: 404,
        headers: { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" },
      });
    }
    return metaCallback(url.searchParams, env);
  },
};

async function metaCallback(params, env) {
  const [port, state] = splitState(params.get("state") ?? "");
  if (!port) {
    return page(400, () => "<p>This link did not come from Moneyball. Start again from the Moneyball app.</p>");
  }
  const fields = params.has("error") ? { error: declined(params) } : await exchange(params.get("code"), env);
  return handBack(port, { ...fields, state });
}

// "<port>.<state>" -> [port, state], or [] when either part is off.
function splitState(raw) {
  const dot = raw.indexOf(".");
  const port = raw.slice(0, dot);
  const state = raw.slice(dot + 1);
  const n = Number(port);
  if (dot < 0 || !PORT.test(port) || n < 1024 || n > 65535 || !STATE.test(state)) return [];
  return [Number(port), state];
}

function declined(params) {
  if (params.get("error") === "access_denied") {
    return "Facebook sign-in was cancelled, so nothing was connected.";
  }
  return `Facebook did not finish signing in: ${params.get("error_description") || params.get("error")}`;
}

// The authorization code for the token, at Meta. Returns { token } or { error }.
async function exchange(code, env) {
  const need = ["META_APP_ID", "META_APP_SECRET", "META_GRAPH_VERSION", "META_REDIRECT_URI"];
  const missing = need.filter((k) => !env[k]);
  if (missing.length) return { error: `The Moneyball broker is not set up: ${missing.join(", ")} missing.` };
  if (!code) return { error: "Facebook did not send an authorization code. Try again." };

  let res;
  let body;
  try {
    // POST, so the secret and the code never sit in a URL, even Meta's.
    res = await fetch(`https://graph.facebook.com/${env.META_GRAPH_VERSION}/oauth/access_token`, {
      method: "POST",
      body: new URLSearchParams({
        client_id: env.META_APP_ID,
        client_secret: env.META_APP_SECRET,
        redirect_uri: env.META_REDIRECT_URI,
        code,
      }),
      signal: AbortSignal.timeout(15000),
    });
    body = await res.json();
  } catch {
    return {
      error: res
        ? `Meta answered with HTTP ${res.status} and no details. Try again.`
        : "Could not reach Meta to finish connecting. Try again in a minute.",
    };
  }
  if (typeof body?.access_token === "string" && body.access_token) return { token: body.access_token };
  const why = body?.error?.message || `HTTP ${res.status}`;
  return { error: `Meta did not accept the sign-in: ${hide(why, [env.META_APP_SECRET, code])}` };
}

function hide(text, secrets) {
  return secrets.reduce((t, s) => t.replaceAll(s, "[hidden]"), String(text));
}

// A page that POSTs the fields to the app's loopback listener by itself,
// so the token never sits in a URL or the browser history.
function handBack(port, fields) {
  const action = `http://127.0.0.1:${port}/meta/done`;
  const inputs = Object.entries(fields)
    .map(([k, v]) => `<input type="hidden" name="${k}" value="${esc(v)}">`)
    .join("");
  return page(
    200,
    (nonce) =>
      `<form method="post" action="${action}">${inputs}` +
      "<p>Returning to Moneyball...</p><button type=\"submit\">Continue</button></form>" +
      `<script nonce="${nonce}">document.forms[0].submit()</script>`,
    `http://127.0.0.1:${port}`,
  );
}

function page(status, body, formAction = "'none'") {
  const nonce = crypto.randomUUID();
  const html =
    '<!doctype html><html lang="en"><head><meta charset="utf-8">' +
    '<meta name="viewport" content="width=device-width, initial-scale=1"><title>Moneyball</title>' +
    `<style nonce="${nonce}">:root{color-scheme:light dark}` +
    "body{font:16px/1.5 -apple-system,system-ui,sans-serif;max-width:32rem;margin:20vh auto;padding:0 16px}</style>" +
    `</head><body>${body(nonce)}</body></html>`;
  return new Response(html, {
    status,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
      "Referrer-Policy": "no-referrer", // the callback URL holds the code
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy":
        `default-src 'none'; script-src 'nonce-${nonce}'; style-src 'nonce-${nonce}'; ` +
        `form-action ${formAction}; base-uri 'none'; frame-ancestors 'none'`,
    },
  });
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}
