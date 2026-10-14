import { useUserProfile } from "../profile/queries.ts";
import type { PresenceStatus } from "../ws/protocol.ts";
import { GhostButton, Modal } from "./Modal.tsx";
import { ProfileCardView, type CardPerson } from "./ProfileCard.tsx";

/**
 * Someone's profile card outside a server (DM peers). Shows what is known
 * at once and fills in the rest from `GET /api/users/{id}`.
 */
export function UserProfileDialog({
  userId,
  known,
  presence,
  open,
  onClose,
}: {
  userId: string;
  known: CardPerson;
  presence?: PresenceStatus | undefined;
  open: boolean;
  onClose: () => void;
}) {
  const { data: card } = useUserProfile(userId, open);
  const person: CardPerson = {
    ...known,
    ...(card?.id === userId ? card : {}),
  };
  return (
    <Modal open={open} title={`Profil von ${person.name}`} onClose={onClose}>
      <div className="member-profile">
        <ProfileCardView person={person} presence={presence} />
        <div className="member-profile__actions">
          <GhostButton onClick={onClose}>Schließen</GhostButton>
        </div>
      </div>
    </Modal>
  );
}
