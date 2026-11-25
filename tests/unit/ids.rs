use std::collections::HashSet;

use sessiond::ids::{Guid, GuidGenerator};

#[test]
fn generated_guids_are_unique() {
    let mut generator = GuidGenerator::new();
    let guids: HashSet<Guid> = (0..100_000).map(|_| generator.generate()).collect();
    assert_eq!(guids.len(), 100_000);
}

#[test]
fn generated_guids_are_version_4_rfc_variant() {
    let mut generator = GuidGenerator::new();
    for _ in 0..1_000 {
        let text = generator.generate().to_string();
        let bytes = text.as_bytes();
        assert_eq!(bytes[14], b'4', "version nibble in {text}");
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'), "variant nibble in {text}");
    }
}

#[test]
fn never_generates_nil_or_small_fixed_values() {
    let mut generator = GuidGenerator::new();
    for _ in 0..1_000 {
        let guid = generator.generate();
        assert_ne!(guid, Guid::NIL);
        // Tests use `from_u128` with small numbers as fixed IDs; those must never clash.
        assert!(guid.as_u128() > u32::MAX as u128);
    }
}

#[test]
fn displays_in_hyphenated_form() {
    let guid = Guid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    assert_eq!(guid.to_string(), "01234567-89ab-cdef-0123-456789abcdef");
    assert_eq!(format!("{guid:?}"), "01234567-89ab-cdef-0123-456789abcdef");
    assert_eq!(Guid::NIL.to_string(), "00000000-0000-0000-0000-000000000000");
}
