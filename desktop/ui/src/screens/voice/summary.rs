//! The room's lines about who is there (web voice/roomSummary.ts).

use gelabber_i18n::voice as texts;

/// "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere".
pub fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => texts::names_two(a, b),
        [a, b, c] => texts::names_three(a, b, c),
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
            texts::summary_you_with(&name_list(others))
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

/// `stageFacts`: "3 im Kanal · 2 Kameras", "3 im Kanal · Jonas ist live".
pub fn stage_facts(people: usize, cameras: usize, live: &[String], own_live: bool) -> String {
    let count = texts::facts_people(&people.to_string());
    if !live.is_empty() || own_live {
        let names: Vec<String> = if own_live {
            std::iter::once(you_name())
                .chain(live.iter().cloned())
                .collect()
        } else {
            live.to_vec()
        };
        return format!("{count} · {}", live_sentence(&names, own_live));
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
        assert_eq!(stage_facts(3, 2, &[], false), "3 im Kanal · 2 Kameras");
        assert_eq!(
            stage_facts(3, 0, &s(&["Jonas"]), false),
            "3 im Kanal · Jonas ist live"
        );
        assert_eq!(
            stage_facts(2, 0, &s(&["Mia"]), true),
            "2 im Kanal · Du und Mia seid live"
        );
        assert_eq!(stage_facts(1, 0, &[], true), "1 im Kanal · Du bist live");
    }
}
