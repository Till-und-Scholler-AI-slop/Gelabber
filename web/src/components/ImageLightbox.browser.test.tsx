import { describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";

import { ImageLightbox, type LightboxImage } from "./ImageLightbox.tsx";

// A one-pixel picture, so nothing is fetched.
const PIXEL =
  "data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==";
const picture = (id: string): LightboxImage => ({
  id,
  src: PIXEL,
  filename: `${id}.png`,
});
const gallery = [picture("eins"), picture("zwei"), picture("drei")];

async function open(image: LightboxImage, pictures?: LightboxImage[]) {
  const onClose = vi.fn();
  const screen = await render(
    <>
      <button type="button">Bild öffnen</button>
      <ImageLightbox image={image} gallery={pictures} onClose={onClose} />
    </>,
  );
  return { screen, onClose };
}

/** The dialog, named by the picture it shows. */
const shown = (name: string) => page.getByRole("dialog", { name });

describe("image lightbox gallery", () => {
  it("starts at the picture clicked, with focus on „Schließen“", async () => {
    await open(gallery[1]!, gallery);
    await expect.element(shown("Bild 2 von 3: zwei.png")).toBeVisible();
    await expect.element(page.getByText("2 / 3")).toBeVisible();
    expect(document.activeElement).toBe(
      page.getByRole("button", { name: "Schließen" }).element(),
    );
  });

  it("steps with → and ←, round from the last picture to the first", async () => {
    await open(gallery[1]!, gallery);
    await userEvent.keyboard("{ArrowRight}");
    await expect.element(shown("Bild 3 von 3: drei.png")).toBeVisible();
    await expect
      .element(page.getByRole("img"))
      .toHaveAccessibleName("drei.png");
    await userEvent.keyboard("{ArrowRight}");
    await expect.element(shown("Bild 1 von 3: eins.png")).toBeVisible();
    await userEvent.keyboard("{ArrowLeft}");
    await expect.element(shown("Bild 3 von 3: drei.png")).toBeVisible();
  });

  it("steps with the arrow buttons", async () => {
    await open(gallery[0]!, gallery);
    await userEvent.click(
      page.getByRole("button", { name: "Vorheriges Bild" }),
    );
    await expect.element(shown("Bild 3 von 3: drei.png")).toBeVisible();
    await userEvent.click(page.getByRole("button", { name: "Nächstes Bild" }));
    await expect.element(shown("Bild 1 von 3: eins.png")).toBeVisible();
  });

  it("has no stepping for a single picture", async () => {
    await open(gallery[0]!);
    await expect.element(shown("eins.png")).toBeVisible();
    await userEvent.keyboard("{ArrowRight}");
    await expect.element(shown("eins.png")).toBeVisible();
    await expect
      .element(page.getByRole("button", { name: "Nächstes Bild" }))
      .not.toBeInTheDocument();
  });

  it("closes on Esc", async () => {
    const { onClose } = await open(gallery[0]!, gallery);
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });
});
