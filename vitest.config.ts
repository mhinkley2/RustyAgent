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
      // Count every source file, not only the ones a test happens to import.
      //
      // Without this the denominator is import-driven, which inverts the
      // incentive for integration tests: `BoardPage.test.tsx` transitively
      // pulls in `RunPanel`, `ListView` and `StoryForm`, so writing it *added*
      // ~2,000 uncounted statements and dropped every threshold below its
      // floor. A test that caught the bug it was written for could not be
      // merged, and deleting it restored a green build. It was deleted.
      //
      // (In Vitest 4 this is `include`. The older `all: true` spelling is
      // accepted silently and does nothing, which is worth knowing before
      // trusting a number that did not move.)
      include: ["src/**/*.{ts,tsx}"],
      // A ratchet, not a target. Set just below what the suite actually
      // achieves so a regression fails the build; raise it as more of the app
      // comes under test. Deliberately untested areas are listed in `exclude`.
      //
      // These numbers dropped by roughly half when `include` was added, and
      // that is not a regression — nothing got worse, the denominator stopped
      // hiding the files nothing tests. 69% was 1474/2126 of the imported
      // files; 37% is 1643/4402 of the app. The second is the honest one, and
      // large untested modules (`RunPanel.tsx`, `ChatPage.tsx`) now dominate
      // it, which is the point.
      //
      // Each floor sits ~1.2pp under actual, which is deliberate and uniform.
      // A tighter one is not a stronger ratchet, it is a different trap: at
      // 0.4pp of slack the binding metric is branches, where ~39 new uncovered
      // branches trips the build — less than one mid-sized component. That
      // would punish "added a file, tests in a follow-up" exactly the way the
      // old denominator punished integration tests.
      //
      // Be honest about what a global percentage can catch. Deleting a large
      // test file trips it; deleting a mid-sized one (StoryCard.test.tsx, say)
      // moves the number by less than a point and passes. This is a floor
      // against drift, not a per-file guarantee.
      thresholds: {
        statements: 36,
        branches: 32,
        functions: 34,
        lines: 37,
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
