import { Button } from "./ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";

export function LoadError({
  message,
  pending = false,
  onRetry,
}: {
  message: string;
  pending?: boolean;
  onRetry: () => void;
}) {
  const t = useT(common);
  return (
    <div role="alert" className="ui-load-error">
      <p>{message}</p>
      <Button
        size="sm"
        pending={pending}
        pendingLabel={t("loading")}
        onClick={onRetry}
      >
        {t("tryAgain")}
      </Button>
    </div>
  );
}
