# Stratum Web

## Page and chat layout

- Overview (`/`) and Longzhong (`/longzhong`) are independent routes. Do not place them in a shared scrolling track.
- Navbar tabs navigate between routes with a left/right view transition; they do not scroll to in-page anchors.
- Chat messages use the document scroll. Do not add an internal message scroller.
- Auto-follow is user-controlled: scrolling upward pauses it, content resize must preserve the reading
  position, and it resumes only after the user reaches the actual bottom or activates the scroll-to-bottom
  control.

## Longzhong chat layout constraints (hard)

- The main chat column on `/longzhong` must remain a single centered column. The only horizontal dimension that may be adjusted is the whitespace (gutter / margin) on the left and right sides of this column.
- Do not embed `ChatHistory` into the main layout flow as a permanent left or right rail. It must stay a togglable overlay / drawer.
- `SiteNavbar` and the bottom `PromptInput` are fixed, but their top/bottom offsets from the viewport must be expressed as the outermost `margin` on their fixed containers, not as internal padding or positioned offsets.
- On wide screens (`2xl`+), the history trigger is rendered as a detached pill to the left of the navbar shell; the drawer opens down-left from that trigger with a safe margin from the left edge.
- The Longzhong composer renders adjacent Agent and model dropdowns in its left tool area. A new
  conversation selects the first template by default; switching Agent starts a new uncreated
  conversation and resets the model to that template default. A pre-session model selection is sent
  with the creation request, while an existing-session selection applies to the next message.
- Approval UI may describe only facts carried by the approval event. Do not generate generic reasons,
  effects, risk claims, or reversibility guidance when the backend did not provide them.

## Frontend test policy

- Do not add, restore, or maintain frontend test files under `stratum-web`.

## Component ownership

- Stratum-owned components live in `app/components/stratum/`.
- Keep third-party components in `app/components/react-bits/`, `app/components/ui/`, or `app/components/ai-elements/`.
