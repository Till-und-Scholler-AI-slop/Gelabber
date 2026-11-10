import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Composer } from "./Composer.tsx";

// Enter in a field submits the form around it: a picker's search inside the
// composer form would send the half-written draft.
describe("composer", () => {
  it("keeps its picker dialogs outside the form", () => {
    const client = new QueryClient();
    const html = renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <Composer
          handle={null}
          channelId="c1"
          channelName="allgemein"
          canSend
          canSendFiles
          mention="#"
          author={{ id: "u1", name: "Ada", avatar_url: null }}
          attempts={[]}
          onSent={() => {}}
          onShowAttempt={() => {}}
        />
      </QueryClientProvider>,
    );
    const form = html.slice(html.indexOf("<form"), html.indexOf("</form>"));
    expect(html).toContain("<form");
    expect(html).toContain("<dialog");
    expect(form).not.toContain("<dialog");
  });
});
