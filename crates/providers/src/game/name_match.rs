//! Deciding whether a local game name and a store result name the same game.
//!
//! Deliberately conservative: an executable's file name is not a store title,
//! so only an equal normalised name (or the same words in another order) counts.
//! A near miss is worse than "No store match", because the wrong cover is
//! confidently wrong.

/// Words a store title may carry that a local name need not.
const EDITIONS: &[&str] = &[
    "game of the year edition",
    "definitive edition",
    "ultimate edition",
    "deluxe edition",
    "complete edition",
    "enhanced edition",
    "digital deluxe edition",
    "premium edition",
    "gold edition",
    "special edition",
    "collector's edition",
    "collectors edition",
    "anniversary edition",
    "remastered edition",
    "game of the year",
    "remastered",
    "goty",
];

/// Lower-case, punctuation-free, without a leading article or a trailing
/// edition qualifier. `™`, `®` and `©` disappear with the other symbols.
pub fn normalize(name: &str) -> String {
    let mut text: String = name
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .collect();
    text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for article in ["the ", "an ", "a "] {
        if let Some(rest) = text.strip_prefix(article)
            && !rest.is_empty()
        {
            text = rest.to_owned();
            break;
        }
    }
    while let Some(stripped) = strip_edition(&text) {
        text = stripped;
    }
    text
}

fn strip_edition(text: &str) -> Option<String> {
    let mut editions = EDITIONS.to_vec();
    editions.sort_by_key(|edition| std::cmp::Reverse(edition.len()));
    for edition in editions {
        if let Some(base) = text.strip_suffix(edition) {
            let base = base.trim_end();
            if !base.is_empty() {
                return Some(base.to_owned());
            }
        }
    }
    None
}

/// The same game by name, whether or not one side carries an edition suffix or
/// the words came in another order.
pub fn same_game(local: &str, store: &str) -> bool {
    let local = normalize(local);
    let store = normalize(store);
    if local.is_empty() || store.is_empty() {
        return false;
    }
    if local == store {
        return true;
    }
    let words = |text: &str| -> Vec<String> {
        let mut words: Vec<String> = text.split(' ').map(str::to_owned).collect();
        words.sort_unstable();
        words
    };
    words(&local) == words(&store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_folded_to_its_words() {
        assert_eq!(
            normalize("The Witcher® 3: Wild Hunt™"),
            "witcher 3 wild hunt"
        );
        assert_eq!(normalize("  A   Plague Tale  "), "plague tale");
        assert_eq!(normalize("Half-Life 2"), "halflife 2");
        assert_eq!(
            normalize("Tom Clancy's Rainbow Six® Siege"),
            "tom clancys rainbow six siege"
        );
        assert_eq!(normalize("®™©"), "");
    }

    #[test]
    fn an_edition_suffix_does_not_hide_a_match() {
        assert_eq!(
            normalize("The Witcher 3: Wild Hunt - Game of the Year Edition"),
            "witcher 3 wild hunt"
        );
        assert!(same_game(
            "The Witcher 3 Wild Hunt",
            "The Witcher 3: Wild Hunt"
        ));
        assert!(!same_game("DOOM", "DOOM Eternal"));
        assert!(!same_game("Hades", "Hades II"));
        // Nothing in common is not a match, and an empty name matches nothing.
        assert!(!same_game("notepad", "Grim Fandango"));
        assert!(!same_game("", ""));
    }

    #[test]
    fn a_store_that_lists_words_in_another_order_still_matches() {
        assert!(same_game("Wild Hunt Witcher 3", "The Witcher 3: Wild Hunt"));
        assert!(same_game("Hunt Wild", "Wild Hunt"));
    }
}
