import { HomeContent } from "~/components/home-content"
import { LocaleProvider } from "~/components/locale-provider"

export default function Home() {
  return (
    <LocaleProvider>
      <HomeContent />
    </LocaleProvider>
  )
}
