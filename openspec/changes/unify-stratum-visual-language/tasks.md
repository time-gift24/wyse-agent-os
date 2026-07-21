## 1. Isolated Setup and Design Sources

- [ ] 1.1 Create a non-main `codex/unify-stratum-visual-language` branch in an isolated worktree and verify the source checkout remains unchanged.
- [ ] 1.2 Run `npx getdesign@latest add lovable` from `stratum-web` and verify the generated Lovable DESIGN.md is stored separately without overwriting `stratum-web/DESIGN.md`.
- [ ] 1.3 Update `stratum-web/DESIGN.md` with the accepted Lovable product palette, derived dark roles, type metrics, radius rules, motion limits, and the reserved Stratum effects layer.

## 2. Global Tokens and Typography

- [ ] 2.1 Refactor `app/app.css` semantic product tokens to use Lovable cream, charcoal, off-white, muted gray, passive border, interactive border, action, and focus roles in light mode.
- [ ] 2.2 Define the dark theme by inverting the Lovable cream and charcoal roles and deriving surfaces, muted text, and borders from their opacity scale.
- [ ] 2.3 Preserve semantic success, warning, and danger tokens separately and retain the existing multicolor Stratum values under effect-only roles.
- [ ] 2.4 Add shared typography roles for the Lovable 60/48/36/20/18/16/14px scale, 400/600 weights, documented line heights, letter spacing, and responsive hero sizing while retaining Noto Sans and Nunito Sans.
- [ ] 2.5 Remove the global multicolor radial canvas treatment, fixed grain overlay, and heavy ordinary-surface shadows from the base product layer.

## 3. Shared Product Shell and Overview

- [ ] 3.1 Restyle the shared navbar so overview and Longzhong use the same Lovable surface, border, typography, action, and radius language while preserving route-specific width constraints and detached history behavior.
- [ ] 3.2 Apply the Lovable typography hierarchy and product colors to the overview route without changing its route, content hierarchy, localization contract, or primary navigation intent.
- [ ] 3.3 Shorten Stratum-owned route and navigation feedback to the documented 150-250ms range and preserve an immediate reduced-motion result.

## 4. Longzhong Product Surfaces

- [ ] 4.1 Adapt the composer through `ChatWorkspace` tokens, props, wrappers, and usage-site styles so it uses Lovable type, color, border, and 12px container rules without modifying protected AI element internals.
- [ ] 4.2 Restyle the history trigger and overlay with the shared Lovable system, a maximum 16px overlay radius, restrained elevation, focus restoration, and brief open/close feedback.
- [ ] 4.3 Reconcile message, reasoning, tool, approval, error, and scroll-to-bottom presentation with the Lovable product layer while keeping semantic states distinguishable and runtime facts unchanged.
- [ ] 4.4 Verify the centered single-column layout, document scrolling, composer visibility, overlay history, touch targets, truncation, and 16px conversation body text at mobile breakpoints.

## 5. Verification and Documentation

- [ ] 5.1 Format changed frontend files and run `pnpm typecheck` and `pnpm build` from `stratum-web` without adding frontend test files.
- [ ] 5.2 Capture and inspect overview and Longzhong screenshots in light and dark themes at desktop and mobile widths.
- [ ] 5.3 Verify Chinese and English layout, keyboard focus visibility, body and placeholder contrast, reduced motion, action-label wrapping, and theme persistence.
- [ ] 5.4 Confirm protected component directories have no internal modifications and that route slugs, API behavior, event projection, approvals, history behavior, and localization keys remain compatible.
- [ ] 5.5 Archive concise durable implementation conventions in `stratum-web/AGENTS.md` and remind maintainers to review that archive before the pull request is merged.
