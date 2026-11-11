//! The room's lines about who is there (web voice/roomSummary.ts).

use gelabber_i18n::voice as texts;

/// "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere".
pub fn name_list(names: &[String]) -> String {
    names_in(names, false)
}

/// [`name_list`] after "mit", in the dative: "Mia, Lena und 2 weiteren".
pub fn name_list_dative(names: &[String]) -> String {
    names_in(names, true)
}

fn names_in(names: &[String], dative: bool) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => texts::names_two(a, b),
        [a, b, c] => texts::names_three(a, b, c),
        [a, b, ..] if dative => texts::names_more_dative(a, b, &(names.len() - 2).to_string()),
        [a, b, ..] => texts::names_more(a, b, &(names.len() - 2).to_string()),
    }
}

/// The viewer among the names of [`live_sentence`]: "Du" / "You".
pub fn you_name() -> String {
    texts::names_you().to_owned()
}

/// "Mia ist live", "Mia und Lena sind live", "Du bist live", "Du und Mia
/// seid live". With `own`, the first name is the viewer's ([`you_name`]).
pub fn live_sentence(names: &[String], own: bool) -> String {
    let list = name_list(names);
    match (own, names.len() == 1) {
        (true, true) => texts::is_live_self(&list),
        (true, false) => texts::is_live_self_and_others(&list),
        (false, true) => texts::is_live_one(&list),
        (false, false) => texts::is_live_many(&list),
    }
}

/// `roomSummary`: occupancy, never a claim that anyone is talking.
/// `others` leaves the viewer out, and so does `live`.
pub fn room_summary(others: &[String], here: bool, live: &[String]) -> String {
    if here {
        return if others.is_empty() {
            texts::summary_only_you().into()
        } else {
            texts::summary_you_with(&name_list_dative(others))
        };
    }
    if others.is_empty() {
        return texts::summary_nobody().into();
    }
    let there = if others.len() == 1 {
        texts::summary_there_one(&name_list(others))
    } else {
        texts::summary_there_many(&name_list(others))
    };
    if live.is_empty() {
        return there;
    }
    texts::summary_there_and_live(&there, &live_sentence(live, false))
}

/// `stageFacts`, the header's facts while pictures run: how many are in
/// the channel and how many Lives run, the own included, or else how many
/// cameras are on. "3 im Kanal · 1 Live", "3 im Kanal · 2 Kameras".
pub fn stage_facts(people: usize, cameras: usize, lives: usize) -> String {
    let count = texts::facts_people(&people.to_string());
    if lives > 0 {
        return format!("{count} · {}", texts::facts_live(&lives.to_string()));
    }
    match cameras {
        0 => count,
        n => format!("{count} · {}", texts::facts_cameras(n as u64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn sentences_like_the_web() {
        assert_eq!(
            name_list(&s(&["Mia", "Lena", "Kim", "Tom"])),
            "Mia, Lena und 2 weitere"
        );
        assert_eq!(
            room_summary(&s(&["Jonas", "Mira"]), false, &[]),
            "Jonas und Mira sind da."
        );
        assert_eq!(
            room_summary(&s(&["Jonas"]), false, &s(&["Jonas"])),
            "Jonas ist da. Jonas ist live."
        );
        assert_eq!(room_summary(&[], false, &[]), "Niemand im Kanal.");
        assert_eq!(room_summary(&[], true, &[]), "Nur du bist im Kanal.");
        assert_eq!(
            room_summary(&s(&["Jonas", "Mira"]), true, &[]),
            "Du bist hier mit Jonas und Mira."
        );
        assert_eq!(stage_facts(3, 2, 0), "3 im Kanal · 2 Kameras");
        assert_eq!(stage_facts(3, 1, 0), "3 im Kanal · 1 Kamera");
        assert_eq!(stage_facts(3, 0, 0), "3 im Kanal");
        // A Live counts, the own too, before the cameras.
        assert_eq!(stage_facts(4, 2, 1), "4 im Kanal · 1 Live");
        assert_eq!(stage_facts(2, 0, 2), "2 im Kanal · 2 Live");
    }

    /// "Du bist hier mit …" puts the rest in the dative; the other lists
    /// keep "weitere".
    #[test]
    fn the_names_after_mit_are_dative() {
        let many: Vec<String> = std::iter::once("Jonas".to_owned())
            .chain(std::iter::once("Mira".to_owned()))
            .chain((0..22).map(|i| format!("P{i}")))
            .collect();
        assert_eq!(
            room_summary(&many, true, &[]),
            "Du bist hier mit Jonas, Mira und 22 weiteren."
        );
        assert_eq!(
            room_summary(&many, false, &[]),
            "Jonas, Mira und 22 weitere sind da."
        );
        assert_eq!(name_list(&many), "Jonas, Mira und 22 weitere");
        assert_eq!(
            room_summary(&s(&["Jonas", "Mira", "Tom"]), true, &[]),
            "Du bist hier mit Jonas, Mira und Tom."
        );
    }

    #[test]
    fn sentences_in_english() {
        let _english = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
        assert_eq!(
            name_list(&s(&["Mia", "Lena", "Kim", "Tom"])),
            "Mia, Lena and 2 more"
        );
        assert_eq!(
            room_summary(&s(&["Jonas"]), false, &s(&["Jonas"])),
            "Jonas is here. Jonas is live."
        );
        assert_eq!(
            room_summary(&[], true, &[]),
            "It’s just you in the channel."
        );
        assert_eq!(stage_facts(3, 2, 0), "3 in the channel · 2 cameras");
        assert_eq!(stage_facts(4, 0, 1), "4 in the channel · 1 live");
        assert_eq!(
            room_summary(&s(&["Mia", "Lena", "Kim", "Tom"]), true, &[]),
            "You’re here with Mia, Lena and 2 more."
        );
    }
}
