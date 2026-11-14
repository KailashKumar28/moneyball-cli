// A stand-in Meta Graph API on loopback. The e2e build reads
// MB_E2E_META_BASE (meta::graph_base), so onboarding and the refresh run
// core's real HTTP client, paging and error mapping with no network.
//
// Onboarding: /me, /debug_token, /me/adaccounts (two pages).
// The refresh: <act>/insights (daily rows, and the 7-day reach),
// ?ids= (ads with their creatives, ad sets), <act>/adimages, <ad>/leads,
// <act>/activities, and the creative images themselves (/img/...).
//
// Every call must carry TOKEN, as the spec pastes it; any other token is
// refused the way Meta does (code 190), so a reconnect can be tested.
// Dates follow the app's own: the windows it asks for, and "yesterday"
// for leads and the change log. GET /__log lists every request seen.
// Run on its own by check-mock-meta.mjs, against the terminal binary.
import { createServer } from "node:http";
import { PNG } from "pngjs";

export const TOKEN = "EAAMoneyballE2EStandInSystemUserToken0123456789";
export const USER = { id: "100090000000001", name: "Moneyball Test System User" };

// The spec types CLIENT as the client's name; only the first account's
// name contains it, so only that one arrives ticked.
export const CLIENT = "Acme Homes";
export const ACCOUNTS = [
  { id: "act_1001", name: "Acme Homes - Cityville", account_status: 1 },
  { id: "act_1002", name: "Fincity Loans", account_status: 1 },
  { id: "act_1003", name: "Old Brand Test", account_status: 2 },
];
const PAGE = 2; // accounts per page: the list always takes a second page

// What runs in each account. The disabled one has nothing.
const TREE = {
  act_1001: {
    campaign: ["2381001", "CV - Leads"],
    adset: ["2382001", "Lookalike 1% - Bengaluru"],
    ads: [["2383001", "Carousel - Lake view"], ["2383002", "Video - Walkthrough", "video"]],
  },
  act_1002: {
    campaign: ["2381002", "FL - Leads"],
    adset: ["2382002", "Interest - Home loans"],
    ads: [["2383003", "Static - Rate card"]],
  },
};

// Every ad and ad set by id, as ?ids= and /<ad>/leads look them up.
const ADS = [];
const NODES = {};
for (const [act, t] of Object.entries(TREE)) {
  NODES[t.adset[0]] = { kind: "adset", act, t };
  for (const [id, name, video] of t.ads) {
    const ad = { id, name, video: Boolean(video), act, t, k: ADS.length };
    ADS.push(ad);
    NODES[id] = { kind: "ad", ...ad };
  }
}

const iso = (d) => d.toISOString().slice(0, 10);
const addDays = (date, n) => iso(new Date(Date.parse(`${date}T00:00:00Z`) + n * 86_400_000));
/** Yesterday by the local clock, as the app counts it. */
function yesterday() {
  const d = new Date();
  d.setDate(d.getDate() - 1);
  return [d.getFullYear(), d.getMonth() + 1, d.getDate()].map((n) => String(n).padStart(2, "0")).join("-");
}

/** One ad's day: numbers from the ad and the day's distance to `until`, as Meta sends them (strings). */
function daily(ad, date, until) {
  const o = Math.round((Date.parse(until) - Date.parse(date)) / 86_400_000);
  const spend = 1200 + 300 * ad.k + ((o * 137) % 500);
  const impressions = Math.round(spend * 8.5);
  const reach = Math.round(impressions * 0.68);
  const clicks = Math.round(impressions * 0.012);
  const link = Math.round(clicks * 0.8);
  return {
    campaign_id: ad.t.campaign[0],
    campaign_name: ad.t.campaign[1],
    adset_id: ad.t.adset[0],
    adset_name: ad.t.adset[1],
    ad_id: ad.id,
    ad_name: ad.name,
    spend: spend.toFixed(2),
    impressions: String(impressions),
    reach: String(reach),
    frequency: (impressions / reach).toFixed(6),
    clicks: String(clicks),
    inline_link_clicks: String(link),
    inline_link_click_ctr: ((link / impressions) * 100).toFixed(6),
    cost_per_inline_link_click: (spend / link).toFixed(6),
    cpc: (spend / clicks).toFixed(6),
    ctr: ((clicks / impressions) * 100).toFixed(6),
    cpm: ((spend / impressions) * 1000).toFixed(6),
    actions: [
      { action_type: "link_click", value: String(link) },
      { action_type: "lead", value: String(1 + ((o + ad.k) % 4)) },
    ],
    date_start: date,
    date_stop: date,
  };
}

/** <act>/insights: per ad per day with time_increment, else per ad over the range (the 7-day reach). */
function insights(act, q) {
  const { since, until } = JSON.parse(q.get("time_range"));
  const ads = ADS.filter((a) => a.act === act);
  const days = [];
  for (let d = since; d <= until; d = addDays(d, 1)) days.push(d);
  if (q.get("time_increment")) return ads.flatMap((ad) => days.map((d) => daily(ad, d, until)));
  return ads.map((ad) => {
    const impressions = days.reduce((n, d) => n + Number(daily(ad, d, until).impressions), 0);
    const reach = Math.round(impressions * 0.55);
    return { ad_id: ad.id, reach: String(reach), impressions: String(impressions), frequency: (impressions / reach).toFixed(6), date_start: since, date_stop: until };
  });
}

/** An ad with its creative, or an ad set, as ?ids= returns it. */
function node(id, base) {
  const n = NODES[id];
  if (n.kind === "adset") {
    return {
      id,
      name: n.t.adset[1],
      effective_status: "ACTIVE",
      learning_stage_info: { status: "SUCCESS" },
      optimization_goal: "LEAD_GENERATION",
      daily_budget: "300000",
      targeting: {
        geo_locations: { cities: [{ key: "1035921", name: "Bengaluru", region: "Karnataka", country: "IN", radius: 25, distance_unit: "kilometer" }] },
        age_min: 28,
        age_max: 55,
      },
    };
  }
  const creative = { id: `9${id}`, title: n.name, body: `${n.name}. Homes from Rs. 75 lakh.`, call_to_action_type: "LEARN_MORE" };
  // A video carries a thumbnail; an image its hash, resolved by adimages.
  if (n.video) Object.assign(creative, { video_id: `7${id}`, thumbnail_url: `${base}/img/thumb-${id}.png` });
  else Object.assign(creative, { image_hash: `e2ehash${id}`, thumbnail_url: `${base}/img/small-${id}.png` });
  return {
    id,
    name: n.name,
    adset_id: n.t.adset[0],
    campaign_id: n.t.campaign[0],
    effective_status: "ACTIVE",
    created_time: `${addDays(yesterday(), -40)}T09:00:00+0000`,
    creative,
  };
}

/** Two leads per ad, yesterday. Fake people. */
const leads = (ad) =>
  [1, 2].map((i) => {
    const n = ad.k * 2 + i;
    return {
      id: `5${ad.id}${i}`,
      created_time: `${yesterday()}T1${i}:00:00+0000`,
      field_data: [
        { name: "full_name", values: [`Test Lead ${n}`] },
        { name: "phone_number", values: [`+91 90000 0000${n}`] },
        { name: "email", values: [`lead${n}@example.com`] },
      ],
    };
  });

/** Yesterday's one budget change in each account that runs ads. */
const activities = (act) =>
  TREE[act]
    ? [
        {
          event_type: "update_ad_set_budget",
          translated_event_type: "Ad set budget updated",
          event_time: `${yesterday()}T05:30:00+0000`,
          object_id: TREE[act].adset[0],
          object_name: TREE[act].adset[1],
          object_type: "CAMPAIGN",
          extra_data: JSON.stringify({
            old_value: { type: "payment_amount", currency: "INR", old_value: 250000 },
            new_value: { type: "payment_amount", currency: "INR", new_value: 300000 },
          }),
        },
      ]
    : [];

/** A flat square, its color from the name, standing in for a creative image. */
function image(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const png = new PNG({ width: 320, height: 320 });
  const rgb = [90 + (h % 120), 110 + ((h >> 8) % 100), 120 + ((h >> 16) % 90)];
  for (let i = 0; i < png.data.length; i += 4) png.data.set([...rgb, 255], i);
  return PNG.sync.write(png);
}

const graphError = (code, message, type = "OAuthException") => ({ error: { message, type, code, fbtrace_id: "AE2EStandIn" } });
const UNSUPPORTED = (id) =>
  graphError(100, `Unsupported get request. Object with ID '${id}' does not exist, cannot be loaded due to missing permissions, or does not support this operation.`, "GraphMethodException");

/** The reply to one GET: [status, body]. */
function answer(path, q, base) {
  const list = (data) => [200, { data, paging: { cursors: { before: "QVFIUmE", after: "QVFIUmZ" } } }];
  if (path === "/me") return [200, { id: USER.id, name: USER.name }];
  if (path === "/debug_token") {
    if (q.get("input_token") && q.get("input_token") !== TOKEN)
      return [200, { data: { is_valid: false, scopes: [], error: { code: 190, message: "Invalid OAuth access token." } } }];
    const data = { app_id: "1000000000001", type: "SYSTEM_USER", application: "Moneyball", data_access_expires_at: 0, expires_at: 0, is_valid: true, issued_at: 1767225600, scopes: ["ads_read"], user_id: USER.id };
    return [200, { data }];
  }
  if (path === "/me/adaccounts") {
    const at = Number(q.get("after") ?? 0);
    const want = (q.get("fields") ?? "id").split(",");
    const data = ACCOUNTS.slice(at, at + PAGE).map((a) => Object.fromEntries(want.map((f) => [f, a[f]])));
    const body = { data, paging: { cursors: { before: String(at), after: String(at + PAGE) } } };
    if (at + PAGE < ACCOUNTS.length) {
      const next = new URL(`${base}/me/adaccounts`);
      for (const [k, v] of q) if (k !== "after") next.searchParams.set(k, v);
      next.searchParams.set("after", String(at + PAGE));
      body.paging.next = next.href;
    }
    if (q.has("summary")) body.summary = { total_count: ACCOUNTS.length };
    return [200, body];
  }
  if (path === "/" && q.has("ids")) {
    const ids = q.get("ids").split(",");
    const unknown = ids.find((id) => !NODES[id]);
    if (unknown) return [400, UNSUPPORTED(unknown)];
    return [200, Object.fromEntries(ids.map((id) => [id, node(id, base)]))];
  }
  const [, id, edge] = path.match(/^\/([^/]+)\/([^/]+)$/) ?? [];
  const isAct = ACCOUNTS.some((a) => a.id === id);
  if (isAct && edge === "insights") return list(insights(id, q));
  if (isAct && edge === "activities") return list(activities(id));
  if (isAct && edge === "adimages") {
    const hashes = JSON.parse(q.get("hashes") ?? "[]");
    return list(hashes.map((hash) => ({ hash, url: `${base}/img/full-${hash}.png` })));
  }
  if (NODES[id]?.kind === "ad" && edge === "leads") return list(leads(NODES[id]));
  return [400, UNSUPPORTED(id ?? path)];
}

/**
 * Start the stand-in on a free loopback port. `slowMs` holds each daily
 * insights pull that long, so the app's live refresh can be seen running.
 */
export function startMockMeta({ slowMs = 0 } = {}) {
  const log = [];
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, `http://${req.headers.host}`);
    const json = (status, body) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    if (url.pathname === "/__log") return json(200, log);
    log.push(`${req.method} ${url.pathname}`);
    // Read only, as the app promises: anything but a GET is refused.
    if (req.method !== "GET") return json(400, graphError(100, "The stand-in Meta answers GET only.", "GraphMethodException"));
    // A creative image on Meta's CDN: no token.
    if (url.pathname.startsWith("/img/")) return res.writeHead(200, { "content-type": "image/png" }).end(image(url.pathname));
    if (url.searchParams.get("access_token") !== TOKEN)
      return json(400, graphError(190, "Invalid OAuth access token - Cannot parse access token"));
    if (slowMs && url.pathname.endsWith("/insights") && url.searchParams.has("time_increment")) await new Promise((r) => setTimeout(r, slowMs));
    const [status, body] = answer(url.pathname, url.searchParams, url.origin);
    json(status, body);
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const url = `http://127.0.0.1:${server.address().port}`;
      resolve({ url, log, close: () => server.close() });
    });
  });
}
