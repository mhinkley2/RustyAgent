import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Vitest config lives here rather than inlined in vite.config.ts so the `test`
// key is typed against `vitest/config` (Vite's own `defineConfig` does not know
// about it). Tailwind is deliberately omitted — tests never render real CSS.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
    clearMocks: true,
    restoreMocks: true,
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "lcov"],
      // A ratchet, not a target. Set just below what the first wave of tests
      // achieved so a regression fails the build; raise it as more of the app
      // comes under test. Deliberately untested areas are listed in `exclude`.
      thresholds: {
        statements: 65,
        branches: 55,
        functions: 65,
        lines: 65,
      },
      // Keep the percentage honest: exclude what we deliberately do not test.
      // See the "Non-goals" section of the coverage plan.
      exclude: [
        "**/node_modules/**",
        "dist/**",
        "src-tauri/**",
        "src/test/**",
        "**/*.test.{ts,tsx}",
        "**/*.config.{ts,js}",
        // App wiring — covered implicitly by any test that renders a route.
        "src/main.tsx",
        "src/App.tsx",
        "src/vite-env.d.ts",
        // Type-only modules. `tsc --noEmit` is their test.
        // runs.ts is the exception: it holds runtime helpers, so it stays in.
        "src/types/board.ts",
        "src/types/agent.ts",
        "src/types/mcp.ts",
        "src/types/human.ts",
        "src/types/permissions.ts",
        "src/types/logs.ts",
        "src/types/custom_tools.ts",
        "src/types/settings.ts",
        // Presentational primitives: prop-to-className mappings with no
        // branching worth protecting. Toast.tsx is excluded from this rule
        // because its pending-queue is real logic.
        "src/components/ui/Button.tsx",
        "src/components/ui/Skeleton.tsx",
        "src/components/ui/EmptyState.tsx",
        "src/components/ui/AlertBanner.tsx",
        "src/components/ui/StatusBadge.tsx",
        "src/components/ui/EntityCard.tsx",
        "src/components/ui/Tooltip.tsx",
        "src/components/forms/FormField.tsx",
        "src/components/forms/FormSelect.tsx",
        "src/components/forms/TextInput.tsx",
        "src/components/forms/Toggle.tsx",
        "src/components/forms/KeyValueInput.tsx",
        "src/components/board/PageHeader.tsx",
        // Barrels.
        "**/index.ts",
      ],
    },
  },
});
