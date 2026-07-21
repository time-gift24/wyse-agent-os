## Context

Stratum Web has two current product surfaces: the overview route and the Longzhong chat. They share navigation and global tokens, but the overview reads as a soft multicolor brand page while the chat adds a pill-shaped, shadowed toolbar and a separate set of panel treatments. The current CSS also retains multicolor radial washes, a fixed grain overlay, deep shadows, and mixed radius roles that conflict with the project's documented restrained workbench direction.

The change adopts Lovable as the immediate visual foundation. Lovable supplies the product color roles, opacity-based neutral hierarchy, border-led depth, typography metrics, and radius discipline. Existing Noto Sans and Nunito Sans assets remain in use. Stratum's multicolor palette remains available only for the brand mark and future purposeful effects, not for current product surfaces or controls.

The implementation must preserve the current route structure, fixed navbar and composer behavior, centered Longzhong column, overlay history, document scrolling, progressive disclosure, approval facts, localization, and API-backed conversation state. Reusable third-party component internals are protected by project policy.

## Goals / Non-Goals

**Goals:**

- Make the overview and Longzhong routes read as one product through shared semantic color, typography, surface, shape, and motion rules.
- Use Lovable's light palette directly and derive a coherent dark palette from the same cream and charcoal roles.
- Use Lovable's type scale, weights, line heights, letter spacing, and responsive display sizes while retaining current font files.
- Replace decorative depth with shallow, border-led containment and a documented radius system.
- Keep semantic state colors accessible and separate them from brand or decorative color.
- Preserve current product behavior and adapt protected components through tokens and usage sites.
- Capture the downloaded Lovable DESIGN.md as reference while keeping `stratum-web/DESIGN.md` authoritative.

**Non-Goals:**

- Replacing Noto Sans or Nunito Sans with Camera Plain or another new font.
- Adding new visual effects, ambient animation, gradient decoration, or scroll choreography.
- Redesigning route information architecture, navigation labels, backend protocols, or conversation behavior.
- Adding permanent chat rails, an event column, fabricated runtime content, or static fake product previews.
- Editing internals under `app/components/ui`, `app/components/react-bits`, or `app/components/ai-elements` without separate approval.

## Decisions

### Use Lovable as the product color layer

The light theme will map semantic tokens to Lovable roles:

- canvas and default surface: Cream `#f7f4ed`
- primary ink and primary action: Charcoal `#1c1c1c`
- action ink and high-contrast highlight: Off-White `#fcfbf8`
- muted copy: Muted Gray `#5f5f5d`
- passive border and divider: Light Cream `#eceae4`
- interactive border: `rgba(28, 28, 28, 0.4)`
- keyboard focus: `rgba(59, 130, 246, 0.5)`

Secondary tonal roles will be derived from Charcoal opacity instead of unrelated gray hues. The dark theme will invert the same cream and charcoal relationship: Charcoal becomes the canvas, Off-White or Cream becomes primary ink, and cream alpha values create surfaces, muted text, and borders. The dark theme will not retain the current blue-gray and violet-gray surface palette.

Semantic success, warning, and danger colors remain distinct because they communicate product state. They must meet contrast requirements and must not become decorative accents.

Alternative considered: retain Baltic Blue as the primary action color. Rejected because the user selected full Lovable product coloring for the first pass. Baltic Blue and the existing multicolor tokens move to the reserved effects layer.

### Separate product, semantic, and effects color namespaces

Tokens will be organized into three conceptual layers:

1. Product tokens map Lovable colors to canvas, surface, ink, border, focus, and action roles.
2. Semantic tokens represent success, warning, danger, and informational states.
3. Effect tokens preserve the current Stratum multicolor palette for the mark and future opt-in visual effects.

Effect tokens must not be used for page backgrounds, navigation, composer surfaces, ordinary cards, body text, or primary controls in this change. Existing multicolor global radial backgrounds and the fixed grain overlay will be removed from the product foundation. The Stratum mark may retain its existing multicolor treatment.

Alternative considered: delete the old palette. Rejected because it remains a useful brand asset for later purposeful effects.

### Adopt Lovable typography metrics without changing font assets

The role scale will use the following values:

| Role | Size | Weight | Line height | Letter spacing |
| --- | --- | --- | --- | --- |
| Hero display | 60px desktop, 48px tablet, 36px mobile | 600 | 1.00-1.10 | -1.5px desktop, scaled proportionally |
| Section heading | 48px | 600 | 1.00 | -1.2px |
| Subheading | 36px | 600 | 1.10 | -0.9px |
| Card title | 20px | 400 | 1.25 | normal |
| Large body | 18px | 400 | 1.38 | normal |
| Body and standard action | 16px | 400 | 1.50 | normal |
| Caption and compact control | 14px | 400 | 1.50 | normal |

The system will use only 400 and 600 as normal hierarchy weights. Noto Sans remains available for display and CJK glyphs, while Nunito Sans remains available for Latin body and UI text. Shared semantic utilities will prevent each route from inventing local type sizes.

Alternative considered: import Camera Plain to reproduce Lovable exactly. Rejected because the user requested Lovable spacing and sizes, not a font-family replacement, and a new font would add licensing, loading, and multilingual consistency concerns.

### Use a shallow depth and explicit shape system

Product containment will follow Lovable's shallow model:

- ordinary content groups remain flat and rely on spacing
- controls and compact menus use a 6px radius
- standard cards, composer surfaces, and image containers use a 12px radius
- large overlay containers may use a 16px radius
- full pills are limited to circular icon actions, toggles, or affordances whose shape communicates their behavior
- passive containers use `#eceae4` or its dark equivalent and no drop shadow
- overlays may use a restrained focus/elevation shadow

The navbar will use the same surface, border, typography, and radius treatment on both routes. Its responsive width may still change to honor the Longzhong centered-column constraint, but its material language must not change between transparent overview chrome and a shadowed chat pill.

Alternative considered: preserve the chat pill as a signature component. Rejected because it is the most visible source of route-to-route discontinuity and conflicts with the existing navigation design requirements.

### Standardize motion around product state

Stratum-owned UI transitions will target 150-250ms for ordinary hover, focus, open, close, selection, and route feedback. GSAP remains the preferred library for Stratum-owned animations already requiring orchestration. CSS transitions are sufficient for simple state feedback. Existing Motion usage inside protected third-party components is not part of this change.

Animations must use transform and opacity where possible and must provide an instant or near-instant reduced-motion outcome. The route change must preserve directional understanding without a long page-load sequence. No new ambient or perpetual effects will be introduced.

Alternative considered: preserve all current 350-550ms sequences. Rejected because the product register prioritizes task flow and short state feedback.

### Adapt through tokens and Stratum-owned usage sites

Global semantic tokens and Stratum-owned components will carry the visual migration. Protected reusable component internals will remain unchanged. When a protected component exposes an unsuitable default, its Stratum-owned caller will adapt it through props, CSS variables, selectors scoped to the usage site, or a wrapper.

The one-time `npx getdesign@latest add lovable` command will be run from `stratum-web`. Because an authoritative `stratum-web/DESIGN.md` already exists, the generated `stratum-web/lovable/DESIGN.md` will be retained as reference rather than replacing project rules. The authoritative design document will summarize the accepted Lovable rules plus Stratum-specific product constraints.

Alternative considered: overwrite `stratum-web/DESIGN.md` with the generated template. Rejected because that would discard product-specific layout, behavior, accessibility, and runtime-data constraints.

## Risks / Trade-offs

- [Warm cream can make a technical product feel editorial] -> Use the palette consistently but keep product density, clear borders, restrained radii, and concrete UI copy.
- [Lovable's light-first reference does not fully specify dark product surfaces] -> Derive dark tokens from the same cream and charcoal roles, verify both modes visually, and document the resulting semantic values.
- [Global typography changes can overflow Chinese navigation and compact controls] -> Verify Chinese and English at desktop and mobile widths, allow truncation only where existing behavior permits it, and keep controls at or above their required touch sizes.
- [Usage-site overrides can become difficult to maintain] -> Prefer semantic CSS variables and small Stratum-owned wrappers; avoid long arbitrary-selector chains unless the protected component has no suitable API.
- [Removing decorative background layers may initially feel less branded] -> Treat the clean Lovable foundation as the first milestone; add Stratum effects only in a later scoped change with a stated communication purpose.
- [Changing both themes at once increases visual regression surface] -> Capture before and after screenshots for both routes, themes, and mobile layouts and apply the migration in token-first stages.

## Migration Plan

1. Create an isolated implementation worktree and non-main branch as required by project policy.
2. Run the getdesign command from `stratum-web` and retain the generated Lovable document as reference.
3. Update `stratum-web/DESIGN.md` with the authoritative three-layer color model, typography metrics, shape roles, and motion limits.
4. Replace global semantic tokens and base backgrounds first, including light and dark values.
5. Migrate the shared navbar and route transition, then the overview route, then Longzhong composer and history surfaces.
6. Verify responsive behavior, both locales, both themes, focus contrast, reduced motion, type checking, and production build.
7. Roll back by reverting the token and Stratum-owned component changes; the reference DESIGN.md is inert and can remain without affecting runtime behavior.

## Open Questions

None. The selected direction is Lovable product colors and typography metrics, current font assets, and Stratum colors reserved for later effects.
