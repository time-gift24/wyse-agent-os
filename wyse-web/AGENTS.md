# Wyse Web

## Chat layout guardrail

- Do not change the existing chat canvas layout in `app/components/chat-workspace.tsx`.
- Preserve `data-slot="chat-main"`, its height and centered placement, the message scroller, and the composer position.
- History-rail changes must remain isolated and must not alter the chat canvas layout.
