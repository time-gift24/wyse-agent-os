# Wyse Web

## Page and chat layout

- Overview (`/`) and Longzhong (`/longzhong`) are independent routes. Do not place them in a shared scrolling track.
- Navbar tabs navigate between routes with a left/right view transition; they do not scroll to in-page anchors.
- Chat messages use the document scroll. Do not add an internal message scroller.
- `ChatHistory` stays detached until the floating layout is designed; PromptInput will also move to a fixed floating position in a later layout pass.

## Frontend test policy

- Do not add, restore, or maintain frontend test files under `wyse-web`.

## Component ownership

- Stratum-owned components live in `app/components/stratum/`.
- Keep third-party components in `app/components/react-bits/`, `app/components/ui/`, or `app/components/ai-elements/`.
