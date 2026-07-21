## ADDED Requirements

### Requirement: Lovable product color foundation
The Stratum web frontend SHALL use Lovable's cream, charcoal, off-white, muted gray, border, and focus colors as the semantic foundation for product surfaces and controls in both supported themes.

#### Scenario: Light theme product roles
- **WHEN** a user views the overview or Longzhong route in the light theme
- **THEN** the page SHALL use `#f7f4ed` for the primary canvas, `#1c1c1c` for primary ink and actions, `#fcfbf8` for action ink or high-contrast highlights, `#5f5f5d` for muted copy, and `#eceae4` for passive borders

#### Scenario: Dark theme product roles
- **WHEN** a user views either route in the dark theme
- **THEN** the page SHALL invert the Lovable cream and charcoal roles and derive surfaces, muted text, and borders from their opacity scale without reintroducing the previous blue-gray or violet-gray product surfaces

#### Scenario: Keyboard focus
- **WHEN** a keyboard user focuses an interactive control
- **THEN** the control SHALL expose a visible focus treatment based on Lovable's `rgba(59, 130, 246, 0.5)` focus color with sufficient contrast against its surface

### Requirement: Color layers remain separate
The frontend MUST distinguish Lovable product colors, semantic state colors, and reserved Stratum effect colors so that each color communicates a single role.

#### Scenario: Ordinary product UI
- **WHEN** the frontend renders navigation, the composer, history, ordinary cards, page backgrounds, or body text
- **THEN** those elements SHALL use Lovable product tokens rather than Stratum effect colors

#### Scenario: Semantic state
- **WHEN** the frontend renders a success, warning, danger, approval, or error state
- **THEN** it SHALL use an accessible semantic color that remains distinguishable from the Lovable product palette

#### Scenario: Reserved effect palette
- **WHEN** this change is implemented
- **THEN** the existing Stratum multicolor palette SHALL remain available for the brand mark and future opt-in effects but SHALL NOT introduce new decorative effects in this change

### Requirement: Lovable typography metrics
The frontend SHALL use Lovable's type sizes, 400 and 600 weight hierarchy, line heights, letter spacing, and responsive display scale while retaining the existing Noto Sans and Nunito Sans font assets.

#### Scenario: Desktop display hierarchy
- **WHEN** the overview hero is rendered on a desktop viewport
- **THEN** its display text SHALL use 60px size, 600 weight, 1.00-1.10 line height, and approximately -1.5px letter spacing

#### Scenario: Responsive display hierarchy
- **WHEN** the overview hero crosses tablet and mobile breakpoints
- **THEN** its display text SHALL scale from 60px to 48px and then 36px while preserving the Lovable proportional tracking and readable line height

#### Scenario: Product text roles
- **WHEN** the frontend renders section headings, subheadings, card titles, large body, body, actions, captions, or compact controls
- **THEN** it SHALL use the documented Lovable role metrics of 48px, 36px, 20px, 18px, 16px, and 14px respectively, with 400 or 600 weight and the documented line heights

#### Scenario: Multilingual typography
- **WHEN** the interface switches between Chinese and English
- **THEN** both locales SHALL preserve the same semantic hierarchy without clipped glyphs, unintended wrapping of desktop actions, or body text below 16px

### Requirement: Unified surface and shape language
The overview and Longzhong routes SHALL use the same border-led depth model and explicit radius roles for equivalent UI elements.

#### Scenario: Shared navigation language
- **WHEN** a user navigates between the overview and Longzhong routes
- **THEN** the navbar SHALL retain the same Lovable surface, border, typography, and shape treatment even if its responsive width changes

#### Scenario: Container depth
- **WHEN** the frontend renders an ordinary content group, composer, history drawer, or overlay
- **THEN** ordinary groups SHALL remain flat, standard containers SHALL use passive borders without heavy drop shadows, and only overlays SHALL receive restrained elevation

#### Scenario: Radius roles
- **WHEN** a component is styled
- **THEN** controls SHALL use approximately 6px radii, standard containers SHALL use 12px radii, large overlays MAY use 16px radii, and full pills SHALL be limited to circular actions, toggles, or behaviorally justified affordances

### Requirement: Purposeful and accessible motion
Stratum-owned frontend motion SHALL communicate interaction or state change, remain brief, and provide a reduced-motion result.

#### Scenario: Ordinary transition
- **WHEN** a user hovers, focuses, selects, opens, closes, or navigates through a Stratum-owned interface element
- **THEN** the visual feedback SHALL normally complete within 150-250ms and animate transform or opacity where practical

#### Scenario: Reduced motion
- **WHEN** the user enables `prefers-reduced-motion: reduce`
- **THEN** route, drawer, selection, and workspace transitions SHALL resolve immediately or nearly immediately without hiding content or obscuring the state change

#### Scenario: Decorative effects
- **WHEN** the unified visual system is applied
- **THEN** the frontend SHALL NOT add ambient, perpetual, scroll-driven, or decorative animation as part of this change

### Requirement: Existing product behavior is preserved
The visual migration MUST preserve the current information architecture, runtime-backed data behavior, accessibility interactions, and hard Longzhong layout constraints.

#### Scenario: Longzhong layout
- **WHEN** the Longzhong route is rendered at any supported viewport
- **THEN** it SHALL retain a single centered conversation column, document scrolling, a fixed composer, and history as a togglable overlay rather than a permanent rail

#### Scenario: Runtime content
- **WHEN** conversation messages, reasoning, tools, approvals, or history are rendered
- **THEN** the visual migration SHALL continue to display only backend or locally persisted facts and SHALL NOT fabricate tool state, approval explanations, conversation results, or product preview data

#### Scenario: Protected component adaptation
- **WHEN** a reusable component under `app/components/ui`, `app/components/react-bits`, or `app/components/ai-elements` needs visual adaptation
- **THEN** the implementation SHALL use semantic tokens, props, a Stratum-owned wrapper, or usage-site styling without changing the protected component's internal implementation

### Requirement: Authoritative design documentation
The repository SHALL retain the generated Lovable DESIGN.md as reference and SHALL maintain Stratum's own DESIGN.md as the authoritative source for product-specific visual and behavioral constraints.

#### Scenario: Lovable reference installation
- **WHEN** `npx getdesign@latest add lovable` is run from `stratum-web` while `stratum-web/DESIGN.md` exists
- **THEN** the generated Lovable document SHALL be retained separately and SHALL NOT overwrite the authoritative Stratum design document

#### Scenario: Future frontend work
- **WHEN** an agent or developer modifies Stratum frontend UI after this change
- **THEN** they SHALL follow `stratum-web/DESIGN.md` for accepted Lovable rules, Stratum-specific constraints, and the separation between product and effect palettes
