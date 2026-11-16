// The signed-in user's friend code with copy and regenerate (#563). Shown
// on the friend list and in the settings area "Profil".

import { confirmAction } from "../components/confirm.ts";
import { notify } from "../components/toasts.ts";
import { Button } from "../components/ui/Button.tsx";
import { CopyButton } from "../components/ui/CopyButton.tsx";
import { useT } from "../i18n/catalog.ts";
import { friends as friendsCatalog } from "../i18n/catalogs/friends.ts";
import { copyText } from "../servers/clipboard.ts";
import { useFriendCode, useRegenerateFriendCode } from "./queries.ts";
import "./friends.css";

export function FriendCodeCard() {
  const t = useT(friendsCatalog);
  const { data } = useFriendCode();
  const regenerate = useRegenerateFriendCode();
  const code = data?.code;
  const renew = async () => {
    const ok = await confirmAction({
      title: t("code.regenerateConfirm"),
      message: t("code.regenerateHint"),
      confirmLabel: t("code.regenerate"),
    });
    if (ok) regenerate.mutate();
  };
  return (
    <section className="friend-code" aria-labelledby="friend-code-title">
      <h2 id="friend-code-title">{t("code.title")}</h2>
      <p className="friend-code__hint">{t("code.hint")}</p>
      <div className="friend-code__row">
        <output className="friend-code__value" aria-live="polite">
          {code ?? t("code.loading")}
        </output>
        <CopyButton
          size="sm"
          disabled={!code}
          onCopy={async () => {
            if (!code) return false;
            const done = await copyText(code);
            if (!done) notify(t("code.copyFailed"));
            return done;
          }}
        >
          {t("code.copy")}
        </CopyButton>
        <Button
          size="sm"
          variant="ghost"
          disabled={!code || regenerate.isPending}
          onClick={() => void renew()}
        >
          {t("code.regenerate")}
        </Button>
      </div>
    </section>
  );
}
