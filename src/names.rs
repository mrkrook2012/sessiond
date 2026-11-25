//! Random, human readable session names like `banana-harbor`.

/// First half of a name: things.
const THINGS: &[&str] = &[
    "acorn", "anchor", "apple", "badger", "banana", "beacon", "birch", "bison", "biscuit",
    "bramble", "cactus", "candle", "cedar", "cherry", "clover", "cobalt", "comet", "copper",
    "coral", "cricket", "dingo", "ember", "falcon", "fern", "fig", "fox", "gecko", "ginger",
    "granite", "hazel", "heron", "iris", "jasper", "juniper", "kiwi", "lantern", "lemon",
    "lynx", "mango", "maple", "marble", "mint", "moose", "nutmeg", "olive", "otter", "owl",
    "panda", "pebble", "pepper", "pine", "plum", "puffin", "quartz", "raven", "saffron",
    "salmon", "sparrow", "thistle", "tiger", "tulip", "walnut", "willow", "yak",
];

/// Second half of a name: places.
const PLACES: &[&str] = &[
    "arch", "bay", "beach", "bluff", "bridge", "brook", "canyon", "cape", "castle", "cave",
    "cliff", "coast", "cove", "creek", "dale", "delta", "dune", "falls", "field", "fjord",
    "forest", "garden", "glacier", "glen", "grove", "harbor", "haven", "heath", "hill",
    "hollow", "island", "lagoon", "lake", "marsh", "meadow", "mesa", "mill", "moor",
    "orchard", "pass", "peak", "pier", "plain", "pond", "prairie", "quarry", "reef", "ridge",
    "river", "rock", "shore", "spring", "summit", "tower", "trail", "tundra", "valley",
    "village", "volcano", "wharf", "woods",
];

/// Tries this many random pairs before falling back to a numbered one.
const ATTEMPTS: usize = 100;

/// A random `thing-place` name that `is_taken` does not reject. With all pairs taken
/// (or very bad luck), a number is added: `banana-harbor-2`.
pub fn random_name(is_taken: impl Fn(&str) -> bool) -> String {
    let pick = |words: &[&'static str]| words[rand::random_range(0..words.len())];

    for _ in 0..ATTEMPTS {
        let name = format!("{}-{}", pick(THINGS), pick(PLACES));
        if !is_taken(&name) {
            return name;
        }
    }

    let base = format!("{}-{}", pick(THINGS), pick(PLACES));
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !is_taken(name))
        .expect("some numbered name is free")
}
