import { Clock3Icon, PlusIcon, SendIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

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

  return (
    <section
      id="longzhong"
      className="min-h-[100dvh] scroll-mt-20 px-4 pt-4 pb-8 md:px-8 md:pb-10"
    >
      <div className="relative mx-auto w-full max-w-5xl">
        <Card
          size="sm"
          className="mb-6 h-[80dvh] w-full 2xl:absolute 2xl:top-0 2xl:right-[calc(100%+1.5rem)] 2xl:mb-0 2xl:w-70"
        >
          <CardHeader>
            <CardTitle>{t("chat.history.title")}</CardTitle>
            <CardDescription>{t("chat.history.description")}</CardDescription>
            <CardAction>
              <Button
                variant="outline"
                size="icon-lg"
                aria-label={t("chat.history.new")}
                title={t("chat.history.new")}
              >
                <PlusIcon aria-hidden="true" />
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col gap-1.5 overflow-y-auto">
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
          <CardFooter className="border-t">
            <p className="text-[0.625rem] text-muted-foreground">
              {t("chat.history.localOnly")}
            </p>
          </CardFooter>
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
