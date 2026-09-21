import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";
import { useTheme, type ThemePreference } from "../theme/theme.ts";

const themes: Array<{ value: ThemePreference; label: string }> = [
  { value: "light", label: "Hell" },
  { value: "dark", label: "Dunkel" },
  { value: "system", label: "System" },
];

export function UserSettingsPage() {
  const theme = useTheme((state) => state.preference);
  const setTheme = useTheme((state) => state.setPreference);

  return (
    <section className="mx-auto max-w-lg">
      <h1 className="text-2xl font-semibold tracking-tight">Einstellungen</h1>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        Darstellung, Voice, Video und Nachrichten-Popups für diesen Browser.
      </p>
      <fieldset className="mt-8 flex flex-col gap-2">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Darstellung
        </legend>
        <div className="grid grid-cols-3 gap-2">
          {themes.map((option) => (
            <label
              key={option.value}
              className={[
                "cursor-pointer rounded-lg border px-3 py-2 text-center text-sm font-medium transition focus-within:ring-2 focus-within:ring-neutral-400 dark:focus-within:ring-neutral-500",
                theme === option.value
                  ? "border-neutral-900 dark:border-neutral-300 bg-neutral-900 dark:bg-neutral-700 text-white"
                  : "border-neutral-300 dark:border-neutral-600 bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800",
              ].join(" ")}
            >
              <input
                type="radio"
                name="theme"
                value={option.value}
                checked={theme === option.value}
                onChange={() => setTheme(option.value)}
                className="sr-only"
              />
              {option.label}
            </label>
          ))}
        </div>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          System folgt automatisch der Einstellung deines Betriebssystems.
        </p>
      </fieldset>
      <div className="mt-8 border-t border-neutral-200 dark:border-neutral-700 pt-8">
        <MediaSettingsForm />
      </div>
    </section>
  );
}
