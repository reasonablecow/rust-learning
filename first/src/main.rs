//! # First 🦀 Project

/// Pairs of "hello world" translations.
/// First element is an alpha-3/ISO 639-2 T (terminology) code of the language.
/// Second element is a google translation of "hello world" into the language.
///
/// Languages were selected based on the countries
/// mentioned in the "lets-meet" discord room.
static HELLO_WORLDS: [(&str, &str); 5] = [
    ("ces", "ahoj světe"),
    ("slk", "ahoj svet"),
    ("ukr", "привіт світ"),
    ("ara", "مرحبا بالعالم"),
    ("ben", "ওহে বিশ্ব"),
];

/// Greets the world in many human languages.
fn main() {
    println!("{:#?}", HELLO_WORLDS)
}
