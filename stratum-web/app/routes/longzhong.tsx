"use client"

import { useEffect, useRef } from "react"
import { useLocation, useNavigate } from "react-router"

import { ChatWorkspace } from "~/components/stratum/chat-workspace"
import { RouteTransition } from "~/components/stratum/route-transition"
import { useAgentConversation } from "~/hooks/use-agent-conversation"

export default function Longzhong() {
  const location = useLocation()
  const navigate = useNavigate()
  const conversation = useAgentConversation()
  const { selectAgent, composerConfiguration } = conversation
  const handledSearchRef = useRef<string | null>(null)

  useEffect(() => {
    if (location.search === "") {
      handledSearchRef.current = null
      return
    }
    if (handledSearchRef.current === location.search) return

    const parameters = new URLSearchParams(location.search)
    const agentId = parameters.get("agent")
    const templateName = parameters.get("template")
    const startNew = parameters.get("new") === "1"

    if (startNew) {
      handledSearchRef.current = location.search
      selectAgent(null)
      navigate("/longzhong", { replace: true })
      return
    }

    if (agentId) {
      handledSearchRef.current = location.search
      selectAgent(agentId)
      return
    }

    if (templateName) {
      if (composerConfiguration.metadataLoading) return
      handledSearchRef.current = location.search
      const template = composerConfiguration.agentTemplates.find(
        (candidate) => candidate.agent_name === templateName
      )
      if (template) composerConfiguration.selectTemplate(template)
      return
    }

    handledSearchRef.current = location.search
  }, [
    composerConfiguration.agentTemplates,
    composerConfiguration.metadataLoading,
    composerConfiguration.selectTemplate,
    location.search,
    navigate,
    selectAgent,
  ])

  return (
    <RouteTransition>
      <ChatWorkspace conversation={conversation} />
    </RouteTransition>
  )
}
