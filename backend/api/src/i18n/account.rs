//! Mail about the account's lifecycle (#519), in the recipient's [`Locale`]:
//! the notice after an account was deleted. Like every account mail, the
//! texts never repeat what someone typed.

use super::Locale;

/// Subject and body of the mail sent once a deleted account's data is gone.
/// Sent to the address the account had, in the language it had.
pub fn account_deleted(locale: Locale, instance: &str) -> (String, String) {
    match locale {
        Locale::De => (
            "Dein Gelabber-Konto wurde gelöscht".to_owned(),
            format!(
                "Hallo,\n\n\
                 dein Gelabber-Konto auf {instance} ist gelöscht. Deine Nachrichten, \
                 Dateien, Direktnachrichten und Reaktionen sind entfernt, und du bist \
                 aus allen Servern ausgetreten.\n\n\
                 Diese Mail ist nur eine Bestätigung; du musst nichts tun.\n"
            ),
        ),
        Locale::En => (
            "Your Gelabber account has been deleted".to_owned(),
            format!(
                "Hi,\n\n\
                 Your Gelabber account on {instance} has been deleted. Your messages, \
                 files, direct messages and reactions have been removed, and you have \
                 left every server.\n\n\
                 This email is only a confirmation; there is nothing you need to do.\n"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_deleted_names_the_instance_in_both_languages() {
        let (de_subject, de_body) = account_deleted(Locale::De, "chat.example.org");
        let (en_subject, en_body) = account_deleted(Locale::En, "chat.example.org");
        assert_eq!(de_subject, "Dein Gelabber-Konto wurde gelöscht");
        assert_eq!(en_subject, "Your Gelabber account has been deleted");
        assert!(
            de_body.contains("Konto auf chat.example.org ist gelöscht"),
            "{de_body}"
        );
        assert_eq!(
            en_body,
            "Hi,\n\nYour Gelabber account on chat.example.org has been deleted. Your \
             messages, files, direct messages and reactions have been removed, and you \
             have left every server.\n\n\
             This email is only a confirmation; there is nothing you need to do.\n"
        );
    }
}
