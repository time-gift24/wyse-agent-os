import { useState } from "react"
import {
  ChevronDownIcon,
  ChevronRightIcon,
  Clock3Icon,
  PlusIcon,
  SendIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"

import GlassSurface from "~/components/GlassSurface"
import { StratumMark } from "~/components/stratum-mark"
import { Bubble, BubbleContent } from "~/components/ui/bubble"
import { Button } from "~/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "~/components/ui/card"
import {
  Message,
  MessageAvatar,
  MessageContent,
  MessageFooter,
  MessageHeader,
} from "~/components/ui/message"
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "~/components/ui/message-scroller"
import { Textarea } from "~/components/ui/textarea"

const historyItems = [
  { id: "current", titleKey: "chat.history.current", timeKey: "chat.time.now" },
  {
    id: "tool-policy",
    titleKey: "chat.history.toolPolicy",
    timeKey: "chat.time.yesterday",
  },
  {
    id: "runtime-plan",
    titleKey: "chat.history.runtimePlan",
    timeKey: "chat.time.lastWeek",
  },
] as const

const messages = [
  {
    id: "assistant-intro",
    role: "assistant",
    bodyKey: "chat.messages.assistantIntro",
    timeKey: "chat.time.now",
  },
  {
    id: "user-question",
    role: "user",
    bodyKey: "chat.messages.userQuestion",
    timeKey: "chat.time.now",
  },
  {
    id: "assistant-answer",
    role: "assistant",
    bodyKey: "chat.messages.assistantAnswer",
    timeKey: "chat.time.now",
  },
] as const

export function ChatWorkspace() {
  const { t } = useTranslation()
  const [isHistoryOpen, setIsHistoryOpen] = useState(false)

  return (
    <section
      id="longzhong"
      className="min-h-[100dvh] scroll-mt-20 px-4 pt-4 pb-8 md:px-8 md:pb-10"
    >
      <div className="relative mx-auto w-full max-w-5xl">
        <Card
          size="sm"
          className="relative mb-6 w-full bg-transparent ring-0 2xl:absolute 2xl:top-0 2xl:right-[calc(100%+1.5rem)] 2xl:mb-0 2xl:w-70"
        >
          <div className="absolute inset-0 -z-10">
            <GlassSurface
              width="100%"
              height="100%"
              borderRadius={8}
              borderWidth={0.06}
              brightness={68}
              opacity={0.94}
              blur={10}
              displace={0}
              backgroundOpacity={0.45}
              saturation={1.15}
              distortionScale={-40}
              redOffset={0}
              greenOffset={2}
              blueOffset={4}
              mixBlendMode="normal"
            />
          </div>

          <CardHeader className="grid-cols-[minmax(0,1fr)_auto] grid-rows-[auto] items-center gap-2">
            <button
              type="button"
              aria-controls="chat-history"
              aria-expanded={isHistoryOpen}
              className="-m-1 flex min-w-0 items-center gap-1 rounded-sm p-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
              onClick={() => setIsHistoryOpen((open) => !open)}
            >
              <CardTitle className="truncate">
                {t("chat.history.title")}
              </CardTitle>
              {isHistoryOpen ? (
                <ChevronDownIcon
                  aria-hidden="true"
                  className="size-4 shrink-0"
                />
              ) : (
                <ChevronRightIcon
                  aria-hidden="true"
                  className="size-4 shrink-0"
                />
              )}
            </button>
            <CardAction className="col-start-2 row-span-1 row-start-1 self-center">
              <Button
                variant="outline"
                size="icon-sm"
                aria-label={t("chat.history.new")}
                title={t("chat.history.new")}
              >
                <PlusIcon aria-hidden="true" />
              </Button>
            </CardAction>
          </CardHeader>
          {isHistoryOpen ? (
            <CardContent id="chat-history" className="flex flex-col gap-1.5">
              {historyItems.map((item, index) => (
                <Button
                  key={item.id}
                  variant={index === 0 ? "secondary" : "ghost"}
                  size="lg"
                  className="h-auto w-full justify-start py-2 text-left"
                >
                  <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
                    <span className="w-full truncate">{t(item.titleKey)}</span>
                    <span className="flex items-center gap-1 text-[0.625rem] text-muted-foreground">
                      <Clock3Icon aria-hidden="true" />
                      {t(item.timeKey)}
                    </span>
                  </span>
                </Button>
              ))}
            </CardContent>
          ) : null}
        </Card>

        <div
          data-slot="chat-main"
          className="flex h-[80dvh] min-h-[36rem] min-w-0 flex-col"
        >
          <MessageScrollerProvider autoScroll>
            <MessageScroller className="flex-1">
              <MessageScrollerViewport>
                <MessageScrollerContent className="w-full px-1 py-6 md:px-6">
                  {messages.map((message) => {
                    const isUser = message.role === "user"

                    return (
                      <MessageScrollerItem
                        key={message.id}
                        messageId={message.id}
                        scrollAnchor={isUser}
                      >
                        <Message align={isUser ? "end" : "start"}>
                          {isUser ? null : (
                            <MessageAvatar>
                              <StratumMark
                                animated={false}
                                variant="compact"
                                className="size-6"
                              />
                            </MessageAvatar>
                          )}
                          <MessageContent>
                            <MessageHeader>
                              {isUser ? t("chat.you") : t("chat.assistant")}
                            </MessageHeader>
                            <Bubble
                              variant={isUser ? "secondary" : "ghost"}
                              align={isUser ? "end" : "start"}
                            >
                              <BubbleContent>
                                {t(message.bodyKey)}
                              </BubbleContent>
                            </Bubble>
                            <MessageFooter>{t(message.timeKey)}</MessageFooter>
                          </MessageContent>
                        </Message>
                      </MessageScrollerItem>
                    )
                  })}
                </MessageScrollerContent>
              </MessageScrollerViewport>
              <MessageScrollerButton />
            </MessageScroller>
          </MessageScrollerProvider>

          <Card size="sm" className="w-full shrink-0">
            <CardHeader>
              <CardTitle>{t("chat.composer.title")}</CardTitle>
              <CardDescription>
                {t("chat.composer.description")}
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Textarea
                aria-label={t("chat.composer.label")}
                placeholder={t("chat.composer.placeholder")}
                rows={2}
              />
            </CardContent>
            <CardFooter className="justify-between gap-3 border-t">
              <p className="text-[0.625rem] text-muted-foreground">
                {t("chat.composer.hint")}
              </p>
              <Button type="button" size="lg">
                {t("chat.composer.send")}
                <SendIcon data-icon="inline-end" aria-hidden="true" />
              </Button>
            </CardFooter>
          </Card>
        </div>
      </div>
    </section>
  )
}
