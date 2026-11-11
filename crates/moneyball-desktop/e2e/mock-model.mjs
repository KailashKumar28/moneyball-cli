// A stand-in model on loopback, speaking OpenAI Chat Completions over SSE.
// The demo workspace's provider points here, so the advisor's real HTTP
// client, stream parser and tool loop all run with no network.
//
// One turn is two requests: the first is answered with a call to the
// `brief` tool, the second (it now carries the tool's output) with text.
// A message containing "slow" gets a long, slow answer to press Stop on.
// GET /calls returns how many completions were asked for.
import { createServer } from "node:http";

export const ANSWER = "Namma Mane is your cheapest product today. **Keep it running** and watch Cityville.";
const SLOW_WORDS = 400;
const SLOW_GAP_MS = 100;

const chunk = (delta, finish = null) =>
  `data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function answer(res, messages) {
  const lastUser = messages.findLastIndex((m) => m.role === "user");
  const said = JSON.stringify(messages[lastUser]?.content ?? "");
  const toolRan = messages.slice(lastUser + 1).some((m) => m.role === "tool");

  res.writeHead(200, { "content-type": "text/event-stream" });
  if (said.includes("slow")) {
    for (let i = 0; i < SLOW_WORDS && !res.destroyed; i++) {
      res.write(chunk({ content: `word${i} ` }));
      await sleep(SLOW_GAP_MS);
    }
  } else if (!toolRan) {
    const call = { index: 0, id: "call_e2e_1", type: "function", function: { name: "brief", arguments: "{}" } };
    res.write(chunk({ tool_calls: [call] }));
    res.write(chunk({}, "tool_calls"));
  } else {
    for (const word of ANSWER.split(/(?<= )/)) {
      res.write(chunk({ content: word }));
      await sleep(30);
    }
    res.write(chunk({}, "stop"));
  }
  res.end("data: [DONE]\n\n");
}

/** Start the mock on a free loopback port. */
export function startMockModel() {
  let calls = 0;
  const server = createServer((req, res) => {
    if (req.method === "GET" && req.url === "/calls") {
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ calls }));
      return;
    }
    if (req.method !== "POST" || !req.url.endsWith("/chat/completions")) {
      res.writeHead(404).end();
      return;
    }
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      calls += 1;
      answer(res, JSON.parse(body).messages ?? []).catch(() => res.destroy());
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const url = `http://127.0.0.1:${server.address().port}`;
      resolve({ url, close: () => server.close() });
    });
  });
}
