//! The room's lines about who is there (web voice/roomSummary.ts).

/// "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere".
pub fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} und {b}"),
        [a, b, c] => format!("{a}, {b} und {c}"),
        [a, b, ..] => format!("{a}, {b} und {} weitere", names.len() - 2),
    }
}

/// "Mia ist live", "Mia und Lena sind live", "Du bist live", "Du und Mia
/// seid live".
pub fn live_sentence(names: &[String], own: bool) -> String {
    let verb = match (own, names.len() == 1) {
        (true, true) => "bist",
        (true, false) => "seid",
        (false, true) => "ist",
        (false, false) => "sind",
    };
    format!("{} {verb} live", name_list(names))
}

/// `roomSummary`: occupancy, never a claim that anyone is talking.
/// `others` leaves the viewer out, and so does `live`.
pub fn room_summary(others: &[String], here: bool, live: &[String]) -> String {
    if here {
        return if others.is_empty() {
            "Nur du bist im Kanal.".into()
        } else {
            format!("Du bist hier mit {}.", name_list(others))
        };
    }
    if others.is_empty() {
        return "Niemand im Kanal.".into();
    }
    let verb = |n: usize| if n == 1 { "ist" } else { "sind" };
    let there = format!("{} {} da.", name_list(others), verb(others.len()));
    if live.is_empty() {
        return there;
    }
    format!("{there} {} {} live.", name_list(live), verb(live.len()))
}

/// `stageFacts`: "3 im Kanal · 2 Kameras", "3 im Kanal · Jonas ist live".
pub fn stage_facts(people: usize, cameras: usize, live: &[String], own_live: bool) -> String {
    let count = format!("{people} im Kanal");
    if !live.is_empty() || own_live {
        let names: Vec<String> = if own_live {
            std::iter::once("Du".to_owned())
                .chain(live.iter().cloned())
                .collect()
        } else {
            live.to_vec()
        };
        return format!("{count} · {}", live_sentence(&names, own_live));
    }
    match cameras {
        0 => count,
        1 => format!("{count} · 1 Kamera"),
        n => format!("{count} · {n} Kameras"),
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
