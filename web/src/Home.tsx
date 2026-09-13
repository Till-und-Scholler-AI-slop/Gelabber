import { useAppStore } from "./store.ts";

export function Home() {
  const name = useAppStore((state) => state.name);
  const apiBase = import.meta.env.VITE_API_BASE_URL ?? "/api";

  return (
    <main className="mx-auto flex min-h-dvh max-w-xl flex-col justify-center gap-4 px-6">
      <h1 className="text-3xl font-semibold tracking-tight">{name}</h1>
      <p className="text-neutral-600">
        Monorepo-Schnitt steht. Chat, Voice und Live kommen in späteren
        Tickets — ohne LiveKit, ohne fremdes Produkt-SDK.
      </p>
      <p className="text-sm text-neutral-500">API: {apiBase}</p>
    </main>
  );
}
