# Moneyball auth broker

A stateless Cloudflare Worker that completes Meta's "Facebook Login for
Business" for the Moneyball desktop app. Meta's app secret cannot ship
inside an app, so the broker holds it: Meta redirects the browser here with
an authorization code, the broker swaps the code for the system-user token,
and answers with a page that POSTs the token to the app's loopback listener
on `127.0.0.1`. It stores nothing and logs nothing, and the code, the token
and the secret never sit in a URL, a log or an error.

## Protocol

[plans/desktop/auth-plan.md](../plans/desktop/auth-plan.md), "The Meta
protocol", step 4. The broker serves one route:

- `GET /meta/callback?code=..&state=<port>.<state>` (or
  `error=..&error_description=..`). The port must be 1024 to 65535 and the
  state 43 chars of base64url, or the answer is a 400 page with no form.
- The code is swapped by a POST to
  `https://graph.facebook.com/<META_GRAPH_VERSION>/oauth/access_token`
  with `client_id`, `client_secret`, `redirect_uri` and `code` in the form
  body.
- The answer is a page that POSTs (`application/x-www-form-urlencoded`)
  to `http://127.0.0.1:<port>/meta/done` by itself, with `state` (the part
  after the port) and either `token` or `error`. `error` is plain words,
  shown to the user as is. A **Continue** button submits it when scripts
  are off.
- Headers: `Cache-Control: no-store`, `Referrer-Policy: no-referrer` (the
  callback URL holds the code), and a CSP of `default-src 'none'` with a
  per-response nonce for the one inline script and style and
  `form-action http://127.0.0.1:<port>`.
- Everything else (other paths, other methods) is a 404.

## Config

| Name | Where | Value |
|---|---|---|
| `META_APP_ID` | `[vars]` in wrangler.toml | The Meta app's id (public) |
| `META_GRAPH_VERSION` | `[vars]` | `v26.0`, the same as the app |
| `META_REDIRECT_URI` | `[vars]` | `https://<worker host>/meta/callback`, exactly as registered with Meta and as the app sends it in the dialog |
| `META_APP_SECRET` | Worker secret only | `wrangler secret put META_APP_SECRET`; never in the repo |

A missing one is named in the `error` the app shows. Workers Logs stay off
(`[observability] enabled = false`): a log of the request would hold the code.

## Local dev and tests

```sh
cd broker
pnpm install
pnpm test        # node --test, with a stand-in Meta (stubbed global fetch)
pnpm check       # the tests, then wrangler deploy --dry-run
```

For `pnpm dev` (wrangler dev on `http://127.0.0.1:8787`), put
`META_APP_SECRET=<anything>` in `broker/.dev.vars` (git-ignored).
The only dependency is wrangler, for dev and deploy.

## Deploy (the owner)

1. Meta app: a Meta app of type Business in your business portfolio, with
   the products Facebook Login for Business and the Marketing API.
2. Login for Business configuration (in the app: Facebook Login for
   Business, Configurations, Create): token type system user, expiry never,
   asset types ad accounts and Pages, permissions `ads_read`,
   `leads_retrieval`, `pages_manage_ads`, `pages_show_list`,
   `pages_read_engagement`, `business_management`. Its id is the app's
   `CONFIG_ID` in core.
3. `cd broker && pnpm install && pnpm exec wrangler login`
4. In `wrangler.toml`, set `META_APP_ID` and `META_REDIRECT_URI`
   (`https://moneyball-broker.<your-subdomain>.workers.dev/meta/callback`;
   your workers.dev subdomain is on the Cloudflare dashboard under Workers).
5. `pnpm exec wrangler secret put META_APP_SECRET` and paste the app
   secret (App settings, Basic).
6. `pnpm exec wrangler deploy`. Check the printed URL matches
   `META_REDIRECT_URI`; if not, fix the var and deploy again.
7. In the Meta app, Facebook Login for Business, Settings: add the
   deployed `.../meta/callback` URL to Valid OAuth Redirect URIs. Leave
   "Require App Secret" off, so the app can call the Graph API directly.
8. Add every operator to the app as a tester (App roles); testers can
   grant every permission while the app is in development.
9. Before serving businesses other than your own: Business Verification
   and App Review for the permissions above; after 500 API calls in 15
   days, ask for the Marketing API's full tier.

`wrangler tail` streams live requests, callback URLs (with the single-use
code) included: do not share its output.
