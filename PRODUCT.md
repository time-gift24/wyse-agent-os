# Product

## Register

product

## Platform

web

## Users

Primary audience: end-users who interact with deployed agents through the Longzhong chat. They arrive with a real task — strategic, operational, or analytical — and want an agent that handles it with depth, not a chatbot that deflects. They are not developers; they should never need to understand the runtime, the event protocol, or the tool registry to get their work done.

Secondary audience: developers and operators who configure agent templates, compose workflows, and deploy agents behind the scenes. They need visibility into execution, approval flows, and recovery — but their workflow is configuration and orchestration, not chat. A future visual orchestration page (Dify-style workflow composition) will serve this audience directly.

The web frontend serves both from one product. The chat surface leads for end-users; the orchestration surface (planned) leads for developers. Neither audience is forced into the other's workflow.

## Product Purpose

Stratum is a Rust-first agent runtime for composing agents, tools, and reliable execution paths. The web frontend, 运筹 Stratum, is the human surface of that runtime: end-users converse with agents through the Longzhong chat, and developers compose and deploy those agents through a future orchestration page.

Success looks like trust through transparency. The user trusts the agent to handle a real task end-to-end, with the ability to see what is happening whenever they want to. Thinking, tool execution, and intermediate steps are collapsed by default — the user trusts the agent to proceed — but any step can be expanded on demand to inspect reasoning, tool calls, and results in detail.

## Positioning

Strategic depth — composable agents for complex, high-stakes work. Not a chatbot, not a cloud console, not a developer tool. An agent product that earns trust through observable, controllable execution, where every interaction leaves the user with a better plan, not just a completed task.

## Brand Personality

Restrained, trustworthy, engineering-workbench. The product carries the quiet confidence of serious infrastructure — not loud, not playful, not personality-forward. The agent is a capable colleague, not a character.

The brand name 运筹 (strategize, lay plans) and the chat name 隆中对 (Longzhong Plan, a famous Chinese strategic dialogue) carry a literary and strategic dimension. How central this dimension is to the visual and verbal identity remains an open tension — the product is exploring whether the strategic heritage is a brand pillar or an ambient accent. The positioning already commits to the strategic register; the literary treatment is being discovered through design.

Motion is purposeful and interaction-driven. GSAP-powered animations respond to clicks, state changes, and transitions — not ambient decoration. The product moves when movement communicates something; it stays still when stillness is the right answer.

## Anti-references

Generic SaaS chatbot UIs — ChatGPT-clones, Intercom-style support chat, reskinned "AI assistant" widgets. The product must not read as a chatbot with a skin on top. The Longzhong chat is a workspace for real work, not a support channel.

Visual anti-references (detailed in `stratum-web/DESIGN.md`): no emoji, no Inter, no generic serif, no pure black, no purple/blue neon, no glow, no high-saturation gradients, no card-in-card, no three-equal-cards, no side-stripe borders.

Positive reference for the future orchestration page: Dify's visual workflow editor — a node-based composition surface where agent workflows are built by connecting typed components.

## Design Principles

1. **Progressive transparency.** Default to trust — collapse thinking, tool execution, and intermediate steps. Let the user expand any step on demand. The user trusts because they *can* see, not because they *must* watch.

2. **Strategic depth over task completion.** Every interaction should leave the user with a better understanding of their problem, not just a completed task. The agent surfaces insight, trade-offs, and next steps — not just output.

3. **Engineering rigor in service of end-users.** The Rust foundation delivers reliability, observability, and control, but the UI must never feel like a developer tool. Restrained, not dense; capable, not overwhelming. The runtime's strength is invisible; the conversation is the product.

4. **Motion with purpose.** GSAP-driven interactions respond to clicks, state changes, and transitions. Movement communicates state, not decoration. Reduced-motion is respected; when motion is removed, the state change still reads clearly.

5. **One product, two surfaces, no forced context-switching.** End-users converse (chat); developers compose (orchestration). The product serves both from one brand, but neither audience is forced through the other's workflow. Both surfaces share a visual language but not a layout.

## Accessibility & Inclusion

Practical commitment: body text contrast ≥ 4.5:1, full keyboard navigation, and reduced-motion alternatives for every animation. Glass surfaces are not part of the target design; any retained third-party glass component must provide an opaque fallback, including for reduced-transparency preferences, before it is used in product UI. These are product requirements, not a claim that every current component already satisfies them. No formal WCAG audit target is set yet; revisit when the product matures beyond the current chat surface.
