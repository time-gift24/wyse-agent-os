# Agent and Model Composer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the composer select a default Agent template automatically, show its default Provider and model on the left, and make Agent changes start a new conversation configuration.

**Architecture:** Keep Agent and model configuration state in `useAgentConversation`. The hook selects the first template once metadata is available and exposes an Agent-change action that resets the active session before applying the new template. `ModelConfigMenu` becomes a flat configuration menu and moves into the left composer tools area, where its trigger renders a parsed Provider and model label.

**Tech Stack:** React 19, TypeScript, React Router, Base UI dropdown primitives, Tailwind CSS, existing i18next translations.

---

### Task 1: Add model-label helpers

**Files:**
- Modify: `wyse-web/app/lib/model-config.ts`
- Test: none - `wyse-web/AGENTS.md` prohibits adding or maintaining frontend test files.

**Step 1: Add a pure display helper**

Add a helper that splits a model ID at its first colon and returns a Provider
label and model label. Normalize the Provider only for display (for example,
`deepseek` to `DeepSeek`); leave the original model ID unchanged for API use.
If the ID lacks a colon, return the same value as its model label and omit the
Provider label.

**Step 2: Confirm the helper has safe fallbacks**

Check manually that `deepseek:deepseek-v4-flash` yields `DeepSeek` and
`deepseek-v4-flash`, while `local-model` yields no Provider and `local-model`.

### Task 2: Default and reset Agent-template selection in conversation state

**Files:**
- Modify: `wyse-web/app/hooks/use-agent-conversation.ts`

**Step 1: Select the default template after metadata resolves**

In the successful metadata load branch, if no Agent session or template has
been selected, store the first API-supplied template as `selectedTemplate`.
Do not replace an explicit user selection or a recovered existing Agent.

**Step 2: Add an Agent-template transition action**

Expose a callback in `ComposerConfiguration` that accepts a template. It must:

1. advance the selection generation;
2. clear the selected Agent session through the existing `selectAgent(null)`
   path;
3. clear pending and accepted model configurations;
4. store the target template.

This ordering ensures event recovery from the old session cannot update the new
composer state and prevents model configuration from leaking between Agents.

**Step 3: Preserve existing-session model behavior**

Keep `selectModel` and `setThinkingLevel` restricted to an existing Agent.
They must continue to apply on the following sent message only, and remain
unavailable while a turn is running.

### Task 3: Flatten the composer configuration menu

**Files:**
- Modify: `wyse-web/app/components/stratum/model-config-menu.tsx`
- Modify: `wyse-web/app/locales/en.json`
- Modify: `wyse-web/app/locales/zh.json`

**Step 1: Replace the nested new-Agent menu**

Render an Agent-labelled radio group directly in the root dropdown content.
Selecting an item invokes the hook's Agent-template transition action. Do not
use a submenu for Agent selection.

**Step 2: Keep default model visible after Agent selection**

Use the selected template's `model_config` to render the trigger immediately.
The root trigger is the persistent provider and model label, not the Agent
name. The model displayed after an Agent change therefore always comes from
that Agent's default configuration.

**Step 3: Retain session-only controls without nesting Agent selection**

For an existing Agent session, place the same flat Agent group above the
existing model and optional thinking controls. Selecting an Agent ends the
current session in the UI and returns the user to a new uncreated conversation.

**Step 4: Add concise labels**

Add only the necessary i18n labels for the Agent selection group and the
provider/model trigger. Keep Chinese and English equivalents aligned and avoid
new product terminology.

### Task 4: Move the control to the left composer tools area

**Files:**
- Modify: `wyse-web/app/components/stratum/chat-workspace.tsx`

**Step 1: Render `ModelConfigMenu` inside `PromptInputTools`**

Place it before transient reconnect and cancel actions, so the current Provider
and model have a stable position on the left side of the composer.

**Step 2: Remove the duplicate right-side configuration control**

Keep the right side limited to the submit button and its submit-state feedback.
Do not change reusable prompt-input primitives.

**Step 3: Verify new conversation positioning remains coherent**

The composer may remain vertically centered before the first message and dock
after it. With the default template already selected, its left configuration
label must be visible in both positions.

### Task 5: Verify the user flow

**Files:**
- Verify: `wyse-web/app/hooks/use-agent-conversation.ts`
- Verify: `wyse-web/app/components/stratum/model-config-menu.tsx`
- Verify: `wyse-web/app/components/stratum/chat-workspace.tsx`

**Step 1: Run static verification**

Run from `wyse-web`:

```bash
PATH="/Users/wanyaozhong/.nvm/versions/node/v24.7.0/bin:$PATH" pnpm run typecheck
PATH="/Users/wanyaozhong/.nvm/versions/node/v24.7.0/bin:$PATH" pnpm run test
PATH="/Users/wanyaozhong/.nvm/versions/node/v24.7.0/bin:$PATH" pnpm run build
```

Expected: all commands exit successfully. The build may emit the existing
large-chunk warning only.

**Step 2: Run the local stack and inspect in a browser**

Open `http://localhost:5173/longzhong` with the API available. Confirm:

1. `default-agent` is chosen without a preliminary menu action;
2. the left composer control shows `DeepSeek · deepseek-v4-flash`;
3. selecting an Agent with a different default updates that display instantly;
4. selecting an Agent while viewing a prior conversation clears that active
   session and leaves its history entry intact;
5. no `MenuGroupContext` console error occurs.

**Step 3: Check the diff and commit implementation**

```bash
git diff --check
git add wyse-web/app/lib/model-config.ts \
  wyse-web/app/hooks/use-agent-conversation.ts \
  wyse-web/app/components/stratum/model-config-menu.tsx \
  wyse-web/app/components/stratum/chat-workspace.tsx \
  wyse-web/app/locales/en.json \
  wyse-web/app/locales/zh.json \
  docs/plans/2026-07-13-agent-model-composer-implementation.md
git commit -m "feat(web): simplify agent model selection"
```

Do not stage the pre-existing unrelated changes in `config.docker.toml`,
`wyse-web/app/app.css`, or `wyse-web/app/components/stratum/site-navbar.tsx`.
