import { useState, type ReactNode } from "react";

import { safeColor } from "../auth/rules.ts";
import type { ProfileCard as CardFields } from "../auth/types.ts";
import type { PresenceStatus } from "../ws/protocol.ts";
import { Avatar } from "./Avatar.tsx";
import { AvatarFrame } from "./AvatarFrame.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { accentStyle, bannerOf } from "./profileStyle.ts";
import "./profileCard.css";

export type CardPerson = CardFields & {
  name: string;
  avatar_url: string | null;
};

const PRESENCE: Record<PresenceStatus, string> = {
  o: "Online",
  i: "Abwesend",
  x: "Offline",
};

/**
 * The profile card: banner, decorated avatar, name, pronouns, status text
 * and "Über mich". Used for members, DM peers and the profile page preview,
 * so all of them look the same in the browser, the PWA and the desktop app.
 */
export function ProfileCardView({
  person,
  presence,
  children,
}: {
  person: CardPerson;
  presence?: PresenceStatus | undefined;
  children?: ReactNode;
}) {
  const banner = bannerOf(person);
  const [failedBanner, setFailedBanner] = useState<string | null>(null);
  const image = banner && "image" in banner ? banner.image : null;
  const showImage = image !== null && failedBanner !== image;
  const color =
    banner && "color" in banner
      ? banner.color
      : (safeColor(person.banner_color) ?? safeColor(person.accent_color));

  return (
    <div className="profile-card" style={accentStyle(person.accent_color)}>
      <div
        className="profile-card__banner"
        style={color ? { background: color } : undefined}
        data-testid="profile-card-banner"
      >
        {showImage ? (
          <img src={image} alt="" onError={() => setFailedBanner(image)} />
        ) : null}
      </div>
      <div className="profile-card__avatar">
        {presence ? (
          <PresenceAvatar
            name={person.name}
            url={person.avatar_url}
            status={presence}
            size="lg"
            decoration={person.avatar_decoration}
            accent={person.accent_color}
          />
        ) : (
          <AvatarFrame
            decoration={person.avatar_decoration}
            accent={person.accent_color}
          >
            <Avatar name={person.name} url={person.avatar_url} size="lg" />
          </AvatarFrame>
        )}
      </div>
      <div className="profile-card__body">
        <div className="profile-card__identity">
          <h3>{person.name}</h3>
          {person.pronouns ? (
            <p className="profile-card__pronouns">{person.pronouns}</p>
          ) : null}
          {presence ? (
            <p className="profile-card__presence">{PRESENCE[presence]}</p>
          ) : null}
        </div>
        {person.status_text ? (
          <p className="profile-card__status">{person.status_text}</p>
        ) : null}
        {person.bio ? (
          <section className="profile-card__about">
            <h4>Über mich</h4>
            <p>{person.bio}</p>
          </section>
        ) : null}
        {children}
      </div>
    </div>
  );
}
