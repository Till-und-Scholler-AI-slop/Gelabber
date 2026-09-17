import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";

export function UserSettingsPage() {
  return (
    <section className="mx-auto max-w-lg">
      <h1 className="text-2xl font-semibold tracking-tight">Einstellungen</h1>
      <p className="mt-1 text-sm text-neutral-600">
        Voice, Video und Nachrichten-Popups für diesen Browser.
      </p>
      <div className="mt-8">
        <MediaSettingsForm />
      </div>
    </section>
  );
}
