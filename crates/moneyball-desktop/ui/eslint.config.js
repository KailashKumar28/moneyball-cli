import hooks from "eslint-plugin-react-hooks";
import ts from "typescript-eslint";

export default ts.config(
  { ignores: ["dist", "node_modules", "test-results", "playwright-report"] },
  ...ts.configs.recommended,
  hooks.configs.flat.recommended,
);
