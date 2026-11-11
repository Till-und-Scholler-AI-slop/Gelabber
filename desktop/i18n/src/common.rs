//! Words many surfaces share (web/src/i18n/catalogs/common.ts). Extraction
//! batches only read this namespace; a batch that needs another shared
//! word keeps it in its own namespace.

crate::strings! {
    cancel = "Abbrechen" | "Cancel";
    save = "Speichern" | "Save";
    close = "Schließen" | "Close";
    delete = "Löschen" | "Delete";
    remove = "Entfernen" | "Remove";
    back = "Zurück" | "Back";
    next = "Weiter" | "Next";
    done = "Fertig" | "Done";
    copy = "Kopieren" | "Copy";
    copied = "Kopiert" | "Copied";
    try_again = "Erneut versuchen" | "Try again";
    saved = "Gespeichert." | "Saved.";
    not_saved = "Nicht gespeichert – erneut versuchen" | "Not saved – try again";
    loading = "Wird geladen…" | "Loading…";
    /// The scope tag of settings kept with the account.
    scope_account = "Für dein Konto" | "For your account";
    /// The scope tag of settings kept on this device.
    scope_device = "Nur dieses Gerät" | "This device only";
    online = "Online" | "Online";
    offline = "Offline" | "Offline";
}
