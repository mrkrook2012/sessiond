use sessiond::backend::name_match::{MAX_SUGGESTIONS, edit_distance, similar_names};

const NAMES: [&str; 5] = ["webserver", "web-proxy", "database", "backup", "minecraft"];

#[test]
fn edit_distance_counts_each_kind_of_edit_once() {
    assert_eq!(edit_distance("web", "web"), 0);
    assert_eq!(edit_distance("web", "wen"), 1); // substitution
    assert_eq!(edit_distance("web", "webs"), 1); // insertion
    assert_eq!(edit_distance("webs", "web"), 1); // deletion
    assert_eq!(edit_distance("wbe", "web"), 1); // swap of neighbours
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
}

#[test]
fn typos_suggest_the_intended_name() {
    assert_eq!(similar_names("websrever", NAMES), vec!["webserver"]);
    assert_eq!(similar_names("databse", NAMES), vec!["database"]);
    assert_eq!(similar_names("mincraft", NAMES), vec!["minecraft"]);
}

#[test]
fn prefixes_and_substrings_match() {
    assert_eq!(similar_names("web", NAMES), vec!["web-proxy", "webserver"]);
    assert_eq!(similar_names("proxy", NAMES), vec!["web-proxy"]);
    // Typing too much still finds the name it starts with.
    assert_eq!(similar_names("backup-old", NAMES), vec!["backup"]);
}

#[test]
fn prefix_matches_rank_above_typos() {
    // "back" is a prefix of "backup"; nothing else is close.
    assert_eq!(similar_names("back", ["backup", "bark"]), vec!["backup", "bark"]);
}

#[test]
fn comparison_ignores_case() {
    assert_eq!(similar_names("WEBSRVER", NAMES), vec!["webserver"]);
}

#[test]
fn unrelated_names_are_not_suggested() {
    assert!(similar_names("zzz", NAMES).is_empty());
    assert!(similar_names("printer", NAMES).is_empty());
}

#[test]
fn suggestions_are_capped_and_deduplicated() {
    let many: Vec<String> = (0..20).map(|i| format!("app{i}")).collect();
    let names = many.iter().map(String::as_str).chain(["app1"]);
    let found = similar_names("app", names);
    assert_eq!(found.len(), MAX_SUGGESTIONS);
    let mut unique = found.clone();
    unique.dedup();
    assert_eq!(unique, found);
}
