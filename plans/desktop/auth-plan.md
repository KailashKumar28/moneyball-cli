# Connecting accounts by authorization - plan

Decided by the owner 2026-11-14: onboarding never asks anyone to paste a
token. Moneyball has its own registered app with each provider; the user
authorizes their account and Moneyball gets, stores and refreshes the
token. This replaces D2 of [onboarding-plan.md](onboarding-plan.md) (a
pasted system-user token). Research behind it: the 2026-11-14 auth round
(sources inline) and openai/codex `codex-rs/login` (PKCE, a loopback
callback, auth.json).

## Decisions

| # | Decision | Taken |
|---|---|---|
| A1 | Meta connects through "Continue with Facebook" (Facebook Login for Business), giving a business-integration system-user token that does not expire | Owner, 2026-11-14 |
| A2 | Meta's app secret lives in a stateless auth broker on a Cloudflare Worker; it swaps the authorization code for the token and hands it to the app; it stores and logs nothing | Owner, 2026-11-14 |
| A3 | Moneyball's Meta app also asks for lead permissions (`leads_retrieval`, `pages_manage_ads`, `pages_show_list`, `pages_read_engagement`, `business_management`, with `ads_read`), because Moneyball will collect leads and send them to a CRM or hold them itself (see "Lead capture" below) | Owner, 2026-11-14 |
| A4 | A CRM connects by authorization when it offers one (default); a CRM with API keys only (LeadSquared, Freshsales) gets a guided key form with a live test | Owner, 2026-11-14 |
| A5 | Tokens stay in `~/.moneyball/auth.json`, 0600, in a 0700 folder: the CLI and the morning schedule must read them too, and the macOS data-protection keychain needs an app bundle a CLI is not (Apple TN3137); codex defaults to a file for the same reason | Research |
| A6 | Wave 2 stays off master until "Continue with Facebook" replaces the token placeholder | Owner, 2026-11-14 |

## Per provider

| Provider | How | Server needed | Token | Notes |
|---|---|---|---|---|
| Meta | Login for Business in the system browser, code to the broker, token to the app | The broker, only at connect time | System user, never expires ([Meta](https://developers.facebook.com/docs/facebook-login/facebook-login-for-business)) | App secret cannot ship in an app; "Require App Secret" stays off so the app can call the Graph API directly |
| Google Ads | Desktop OAuth client, loopback + PKCE (codex's shape) | No | Refresh token until revoked or 6 months unused; 7 days while the consent screen is in Testing ([Google](https://developers.google.com/identity/protocols/oauth2)) | Comes with the Google Ads data source, which does not exist yet |
| Salesforce | Public client + PKCE, no secret | No | Rotating refresh tokens | Comes with that CRM's connector |
| Zoho CRM | Mobile/desktop client + PKCE | No (loopback to be confirmed) | Refresh token until revoked | Comes with that CRM's connector |
| HubSpot, Kylas | Code exchange needs the client secret | The broker, at connect and at each refresh | Short access tokens, refresh via the broker | Comes with those connectors |
| LeadSquared, Freshsales | API keys only, no authorization flow | No | Static keys | Guided key form (A4) |
| Sell.do, LeadZump | Unknown (no public auth docs) | - | - | Ask the vendors |

Onboarding needs only Meta now. The others land with their data sources.

## The Meta protocol (app, Meta, broker)

1. The app (core `oauth`) binds a listener on `127.0.0.1` at a random port
   and makes `state` from 32 random bytes (base64url). Nothing is written
   yet.
2. It opens the system browser (never a webview) at Meta's dialog:
   `https://www.facebook.com/<version>/dialog/oauth?client_id=<APP_ID>&config_id=<CONFIG_ID>&response_type=code&override_default_response_type=true&redirect_uri=<BROKER>/meta/callback&state=<port>.<state>`
3. The user picks their business and the ad accounts, Pages and
   permissions; Meta redirects to `<BROKER>/meta/callback?code=..&state=..`
   (or `error=..`).
4. The broker checks the port (1024 to 65535), swaps the code with the
   secret (`/<version>/oauth/access_token`, same `redirect_uri`), and
   answers with a page that POSTs `token` and `state` (or `error`) to
   `http://127.0.0.1:<port>/meta/done` by itself, so the token never sits
   in a URL or the browser history. It keeps nothing.
5. The app accepts only a POST whose `state` matches, stores the token
   (`secrets::store_meta_token`), reads it back with `meta::check_token`
   (name, accounts, scopes, expiry), answers the browser with "Connected.
   You can close this tab.", and closes the listener.
6. Cancel closes the listener; it also closes itself after 5 minutes. An
   `error` (the user declined) is said in words and nothing is stored.

Pinned between the app and the broker (2026-11-14): `state` is 43
characters of base64url (32 random bytes) and Meta carries it as
`<port>.<state>`; the broker posts back only the part after the port;
`redirect_uri` is `<BROKER>/meta/callback`, identical in the dialog and the
code swap; the swap may POST to `/<version>/oauth/access_token`; `error` is
already words, shown as is; Graph API version `v26.0` on both sides.

`APP_ID`, `CONFIG_ID` and `BROKER` are public constants in core; under
core's `e2e` feature each can be overridden (like `meta::graph_base`) so the
e2e drives a stand-in Meta dialog and a stand-in broker. In the e2e build
the "open the browser" step fetches the dialog URL itself and follows the
redirects, standing in for the person.

Reconnect (onboarding Step 6) is the same flow; error 190 from any Graph
call starts it (no subcode: expired or revoked; 458: app removed; 460 or
463: session or expiry).

## What is built where

- **core `oauth`**: the loopback listener (state check, POST only, cancel,
  timeout, a small success page), PKCE helpers for the providers that use
  them (Google, Salesforce, Zoho; not Meta), and `meta::connect(handle)`
  that runs steps 1 to 5 and returns `MetaConnected`. Network stays in the
  allowed modules; no secret in an error, a log or a return value.
- **broker/** (new, in this repo): one Cloudflare Worker, `/meta/callback`
  only for now; `wrangler.toml`; the app secret as a Worker secret; tests
  with a stand-in Meta token endpoint. Later `/hubspot` and `/kylas`.
- **desktop**: the Connect Meta page becomes one button, **Continue with
  Facebook**, a waiting state with **Cancel**, then the `MetaConnected`
  lines and **Next**. The token field and the pasted-id check go.
- **e2e**: the stand-in Meta (`e2e/mock-meta.mjs`) also serves the dialog
  and the token endpoint; a stand-in broker runs beside it.

## The owner's setup outside the code

1. A Meta app of type Business in the owner's business portfolio, with
   Facebook Login for Business and the Marketing API.
2. A Login for Business configuration: system-user token, never expires,
   the permissions of A3, asset types ad accounts and Pages. Its id is
   `CONFIG_ID`.
3. The broker deployed (`wrangler deploy`), its URL added as the valid
   OAuth redirect URI; the app secret set with `wrangler secret put`.
4. Every operator added to the app as a tester (testers can grant any
   permission while the app is in development).
5. Before serving businesses other than the owner's: Business
   Verification and App Review for the permissions of A3; after 500 API
   calls in 15 days, ask for the Marketing API's full tier.

## Lead capture (A3): a separate plan

The owner's direction: Moneyball collects Meta leads itself and either
sends them to the client's CRM or holds them and works them as an
AI-first CRM. That is a new product line (lead storage, routing, a
write path to CRMs, consent and retention), not onboarding. It gets its own
plan. Two things it changes now: the Meta app asks for the lead permissions
at connect time (A3), and the app's line "It only reads: it never changes
your ads or your CRM" must be revisited once Moneyball writes leads to a
CRM.

## Risks to test first

1. With one real client: does the Login for Business picker show ad
   accounts a client shared into the operator's Business Manager?
2. Rate limits before the Marketing API's full tier: is a daily 28-day pull
   across every account inside them?
3. Whether Meta's PKCE flow (documented "still in testing") or device login
   could later remove the broker for Meta.
4. Sell.do and LeadZump: ask the vendors what they offer.

## Relation to docs/CLOUD_PLAN.md

The cloud plan's future `moneyball-server` (axum, Postgres) is unchanged.
The broker is not that server: it holds no data and no state, only the app
secret, and the desktop app keeps working against the Graph API directly.
