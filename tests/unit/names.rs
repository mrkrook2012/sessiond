use sessiond::backend::session::validate_session_name;
use sessiond::names::random_name;

#[test]
fn names_are_two_lowercase_words_and_valid_session_names() {
    for _ in 0..200 {
        let name = random_name(|_| false);
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2, "not word-word: {name}");
        assert!(
            parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase())),
            "unexpected characters: {name}"
        );
        validate_session_name(&name).unwrap();
    }
}

#[test]
fn names_vary() {
    let names: std::collections::HashSet<String> = (0..50).map(|_| random_name(|_| false)).collect();
    assert!(names.len() > 40, "only {} distinct names in 50", names.len());
}

#[test]
fn taken_names_are_skipped() {
    let taken = random_name(|_| false);
    for _ in 0..200 {
        assert_ne!(random_name(|name| name == taken), taken);
    }
}

#[test]
fn a_number_is_added_once_every_pair_is_taken() {
    let name = random_name(|name| name.matches('-').count() < 2);
    let number = name.rsplit('-').next().unwrap();
    assert_eq!(number, "2", "got {name}");
    validate_session_name(&name).unwrap();
}
