//! Mentions in drafts (web/src/messages/mentions.ts): the composer shows
//! `@Name`, the wire carries `<@user-id>`; the `@…` suggestions.

use std::collections::HashMap;

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Suggestion {
    User(Member),
    Everyone,
    Here,
}

impl Suggestion {
    pub fn word(&self) -> &str {
        match self {
            Self::User(m) => &m.name,
            Self::Everyone => "everyone",
            Self::Here => "here",
        }
    }
}

/// Text between backtick runs stays as typed: even entries are prose.
fn code_segments(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut prose = String::new();
    let mut rest = content;
    while let Some(start) = rest.find('`') {
        let fence_len = rest[start..].chars().take_while(|c| *c == '`').count();
        let fence = &rest[start..start + fence_len];
        match rest[start + fence_len..].find(fence) {
            None => {
                prose.push_str(&rest[..start + fence_len]);
                rest = &rest[start + fence_len..];
            }
            Some(end) => {
                let end = start + fence_len + end;
                out.push(std::mem::take(&mut prose) + &rest[..start]);
                out.push(rest[start..end + fence_len].to_owned());
                rest = &rest[end + fence_len..];
            }
        }
    }
    out.push(prose + rest);
    out
}

/// What the composer shows: `@Name` for every token of a known member.
pub fn from_wire(content: &str, members: &[Member]) -> String {
    let names: HashMap<String, &str> = members
        .iter()
        .map(|m| (m.id.to_string(), m.name.as_str()))
        .collect();
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        if tail.len() > 36
            && tail.as_bytes()[36] == b'>'
            && let Ok(id) = Uuid::parse_str(&tail[..36])
            && let Some(name) = names.get(&id.to_string())
        {
            out.push('@');
            out.push_str(name);
            rest = &tail[37..];
            continue;
        }
        out.push_str("<@");
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn mention_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// What is sent: every `@Name` of a member becomes their token, longer
/// names first; a name two members share stays text unless picked.
pub fn to_wire(text: &str, members: &[Member], picked: &HashMap<String, Uuid>) -> String {
    let mut named: Vec<&Member> = members
        .iter()
        .filter(|m| !m.name.trim().is_empty())
        .filter(|m| {
            let lower = m.name.to_lowercase();
            lower != "everyone" && lower != "here"
        })
        .collect();
    if named.is_empty() || !text.contains('@') {
        return text.to_owned();
    }
    named.sort_by_key(|m| std::cmp::Reverse(m.name.chars().count()));
    let mut by_name: HashMap<&str, Option<Uuid>> = HashMap::new();
    for m in &named {
        let entry = by_name.entry(m.name.as_str()).or_insert(Some(m.id));
        if *entry != Some(m.id) {
            *entry = None;
        }
    }
    for (name, id) in picked {
        if named.iter().any(|m| m.id == *id && m.name == *name) {
            by_name.insert(name.as_str(), Some(*id));
        }
    }
    let mut out = String::new();
    for (index, segment) in code_segments(text).iter().enumerate() {
        if index % 2 == 1 {
            out.push_str(segment);
            continue;
        }
        let chars: Vec<char> = segment.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let lead_ok =
                i == 0 || !(chars[i - 1].is_alphanumeric() || "_@-".contains(chars[i - 1]));
            if chars[i] == '@' && lead_ok {
                let found = named.iter().find_map(|m| {
                    let name: Vec<char> = m.name.chars().collect();
                    let end = i + 1 + name.len();
                    (chars.get(i + 1..end) == Some(&name[..])
                        && chars.get(end).is_none_or(|c| !mention_char(*c)))
                    .then_some((m.name.as_str(), end))
                });
                if let Some((name, end)) = found
                    && let Some(Some(id)) = by_name.get(name)
                {
                    out.push_str(&format!("<@{id}>"));
                    i = end;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The `@…` typed at the caret (char index): where it starts and the query.
pub fn mention_at(text: &str, caret: usize) -> Option<(usize, String)> {
    let before: Vec<char> = text.chars().take(caret).collect();
    let mut query = Vec::new();
    let mut i = before.len();
    while i > 0 {
        let c = before[i - 1];
        if c == '@' {
            break;
        }
        if c.is_whitespace() || query.len() >= 32 {
            return None;
        }
        query.push(c);
        i -= 1;
    }
    if i == 0 {
        return None;
    }
    let at = i - 1;
    if at > 0 && !(before[at - 1].is_whitespace() || before[at - 1] == '(') {
        return None;
    }
    query.reverse();
    Some((at, query.into_iter().collect()))
}

/// Members starting with the query first, then containing it; keywords.
pub fn suggestions(
    query: &str,
    members: &[Member],
    me: Option<Uuid>,
    keywords: bool,
) -> Vec<Suggestion> {
    let q = query.to_lowercase();
    let others: Vec<&Member> = members.iter().filter(|m| Some(m.id) != me).collect();
    let mut out: Vec<Suggestion> = others
        .iter()
        .filter(|m| m.name.to_lowercase().starts_with(&q))
        .map(|m| Suggestion::User((*m).clone()))
        .collect();
    out.extend(
        others
            .iter()
            .filter(|m| {
                let n = m.name.to_lowercase();
                !n.starts_with(&q) && n.contains(&q)
            })
            .map(|m| Suggestion::User((*m).clone())),
    );
    if keywords {
        if "everyone".starts_with(&q) {
            out.push(Suggestion::Everyone);
        }
        if "here".starts_with(&q) {
            out.push(Suggestion::Here);
        }
    }
    out.truncate(8);
    out
}

/// The draft after picking `pick` for the `@…` at `start`, and the caret.
pub fn insert_mention(text: &str, start: usize, query: &str, pick: &Suggestion) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let end = (start + 1 + query.chars().count()).min(chars.len());
    let inserted = format!("@{} ", pick.word());
    let mut rest: String = chars[end..].iter().collect();
    if rest.starts_with(' ') {
        rest.remove(0);
    }
    let head: String = chars[..start].iter().collect();
    let caret = start + inserted.chars().count();
    (head + &inserted + &rest, caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(n: u128, name: &str) -> Member {
        Member {
            id: Uuid::from_u128(n),
            name: name.into(),
        }
    }

    #[test]
    fn wire_round_trip() {
        let members = [
            member(1, "Ada"),
            member(2, "Ada Lovelace"),
            member(3, "Tom"),
        ];
        let picked = HashMap::new();
        let wire = to_wire("hi @Ada Lovelace and @Tom, `@Tom`", &members, &picked);
        assert_eq!(
            wire,
            format!(
                "hi <@{}> and <@{}>, `@Tom`",
                Uuid::from_u128(2),
                Uuid::from_u128(3)
            )
        );
        assert_eq!(
            from_wire(&wire, &members),
            "hi @Ada Lovelace and @Tom, `@Tom`"
        );
        assert_eq!(to_wire("mail@Tom", &members, &picked), "mail@Tom");
    }

    #[test]
    fn suggesting() {
        assert_eq!(mention_at("hey @to", 7), Some((4, "to".into())));
        assert_eq!(mention_at("hey@to", 6), None);
        assert_eq!(mention_at("(@", 2), Some((1, String::new())));
        let members = [member(1, "Tom"), member(2, "Atom"), member(3, "Lena")];
        let got = suggestions("to", &members, Some(Uuid::from_u128(3)), true);
        assert_eq!(
            got,
            vec![
                Suggestion::User(members[0].clone()),
                Suggestion::User(members[1].clone())
            ]
        );
        let (text, caret) = insert_mention("hi @to x", 3, "to", &got[0]);
        assert_eq!(text, "hi @Tom x");
        assert_eq!(caret, 8);
    }
}
