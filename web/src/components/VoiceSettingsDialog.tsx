import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";
import { useMediaSettings } from "../voice/settings.ts";
import { GhostButton, Modal } from "./Modal.tsx";

export function VoiceSettingsDialog() {
  const open = useMediaSettings((s) => s.dialogOpen);
  const close = useMediaSettings((s) => s.closeDialog);
  return (
    <Modal open={open} onClose={close} title="Voice & Video" wide>
      <MediaSettingsForm />
      <div className="flex justify-end">
        <GhostButton onClick={close}>Fertig</GhostButton>
      </div>
    </Modal>
  );
}
