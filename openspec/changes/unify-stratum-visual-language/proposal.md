## Why

The overview and Longzhong chat currently share product structure but present different surface, color, shape, typography, and motion languages. A single Lovable-based foundation will make the frontend feel like one trustworthy product while preserving Stratum's existing workflows and leaving its multicolor palette available for later, purposeful effects.

## What Changes

- Adopt the Lovable color system across light and dark product surfaces, controls, borders, text, and primary actions.
- Adopt Lovable's typography sizes, weights, line heights, letter spacing, and responsive scale while retaining the existing Noto Sans and Nunito Sans font assets.
- Normalize navigation, composer, drawer, buttons, and content surfaces around a shallow border-led depth model and explicit radius roles.
- Reduce decorative background treatments and standardize product motion so transitions communicate state without competing for attention.
- Separate the visual system into a Lovable product layer, accessible semantic-state colors, and a dormant Stratum effects palette reserved for later visual effects.
- Preserve route structure, chat behavior, progressive disclosure, document scrolling, overlay history, localization, theming, and backend-driven runtime data.
- Add the Lovable DESIGN.md as a non-authoritative reference and update the project design documentation with the adopted rules.

## Capabilities

### New Capabilities

- `frontend-visual-system`: Defines the unified Lovable-based color, typography, surface, shape, responsive, accessibility, and motion requirements for the Stratum web frontend.

### Modified Capabilities

None.

## Impact

- Affects `stratum-web/DESIGN.md`, global frontend tokens and styles, the overview route, and Stratum-owned navigation, chat workspace, history, and route-transition components.
- Third-party and reusable component internals under `app/components/ui`, `app/components/react-bits`, and `app/components/ai-elements` remain unchanged; adaptation occurs through semantic tokens, props, and usage-site wrappers or classes.
- No backend API, event protocol, data model, route slug, or localization key contract changes are planned.
- The getdesign CLI is used only to capture a reference document and does not become a runtime dependency.
