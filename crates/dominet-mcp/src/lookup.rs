// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding an item by the name a person (or a model) gives it: « cafetière » for `Prise-cafetiere`, « ambiance cinéma » for
//! `Cinéma`. Accents, case, hyphens and a name that is only part of the real one do not matter; a name that fits several items is
//! refused with the list, never guessed (an action on the wrong device is worse than a question).

/// A name without accents or case, words separated by one space.
#[must_use]
pub fn normalize(text: &str) -> String {
    let folded: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The result of a search among names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Exactly one name fits.
    One(usize),
    /// Several fit (the indexes): the caller must say which.
    Many(Vec<usize>),
    None,
}

/// Looks `query` up among `names`: the same name (normalised) first, then a name that contains the query or is contained in it.
#[must_use]
pub fn find(names: &[String], query: &str) -> Found {
    let q = normalize(query);
    if q.is_empty() {
        return Found::None;
    }
    let normalized: Vec<String> = names.iter().map(|n| normalize(n)).collect();
    let same: Vec<usize> = (0..names.len()).filter(|&i| normalized[i] == q).collect();
    let pool = if same.is_empty() {
        (0..names.len())
            .filter(|&i| {
                !normalized[i].is_empty()
                    && (normalized[i].contains(&q) || q.contains(&normalized[i]))
            })
            .collect()
    } else {
        same
    };
    match pool.as_slice() {
        [] => Found::None,
        [one] => Found::One(*one),
        _ => Found::Many(pool),
    }
}

/// The sentence added to a « not found » error: the names that exist (the first ones), so that the caller can try again with a
/// real one.
#[must_use]
pub fn hint(names: &[String], found: &Found) -> String {
    let list = |idx: Vec<&String>| {
        idx.into_iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    match found {
        Found::Many(idx) => format!(
            ". Several match, say which one: {}",
            list(idx.iter().map(|&i| &names[i]).collect())
        ),
        _ if names.is_empty() => ". There is none".to_string(),
        _ => format!(". Existing names: {}", list(names.iter().collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn accents_case_hyphens_and_part_of_a_name_do_not_matter() {
        let n = names(&["Prise-cafetiere", "Volet-salon", "Volet-chambre", "Cinéma"]);
        assert_eq!(find(&n, "cafetière"), Found::One(0));
        assert_eq!(find(&n, "PRISE CAFETIERE"), Found::One(0));
        assert_eq!(find(&n, "ambiance cinema"), Found::One(3));
        assert_eq!(find(&n, "Volet-salon"), Found::One(1));
    }

    #[test]
    fn a_name_that_fits_several_is_refused_and_an_unknown_one_lists_what_exists() {
        let n = names(&["Volet-salon", "Volet-chambre"]);
        assert_eq!(find(&n, "volet"), Found::Many(vec![0, 1]));
        assert_eq!(find(&n, "garage"), Found::None);
        assert_eq!(find(&n, ""), Found::None);
        assert!(hint(&n, &Found::Many(vec![0, 1])).contains("Volet-chambre"));
        assert!(hint(&n, &Found::None).starts_with(". Existing names: Volet-salon"));
        assert_eq!(hint(&[], &Found::None), ". There is none");
    }

    #[test]
    fn an_exact_name_wins_over_a_longer_one_that_contains_it() {
        let n = names(&["Lampe", "Lampe-salon"]);
        assert_eq!(find(&n, "lampe"), Found::One(0));
    }
}
