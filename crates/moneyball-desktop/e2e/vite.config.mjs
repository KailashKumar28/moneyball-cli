// The e2e build of the interface: ui/'s own vite config, written to
// e2e/dist, with the WebdriverIO page helper imported ahead of the entry
// file. The helper is added here, at build time, so ui/ holds no test
// code and a normal build cannot contain it.
import { fileURLToPath } from "node:url";
import base from "../ui/vite.config.ts";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const helper = here("./node_modules/@wdio/tauri-plugin/dist-js/index.js");

export default {
  ...base,
  root: here("../ui"),
  build: { ...base.build, outDir: here("./dist"), emptyOutDir: true },
  plugins: [
    ...base.plugins,
    {
      name: "moneyball-e2e-helper",
      enforce: "pre",
      transform(code, id) {
        if (id.endsWith("/ui/src/main.tsx")) return `import ${JSON.stringify(helper)};\n${code}`;
      },
    },
  ],
};
