import { strings } from "@/i18n/zh-TW";

export default function App() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-2 bg-background text-foreground">
      <h1 className="text-2xl font-semibold">{strings.appName}</h1>
      <p className="text-sm text-muted-foreground">{strings.tagline}</p>
    </main>
  );
}
