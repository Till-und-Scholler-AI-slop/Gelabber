import { createContext } from "react";

import type { LightboxImage } from "./ImageLightbox.tsx";

// Opens the one lightbox of the pane. It lives outside the virtual rows: a
// row scrolled out of the overscan unmounts, and would close it with it.
export const OpenImage = createContext<(image: LightboxImage) => void>(
  () => {},
);
