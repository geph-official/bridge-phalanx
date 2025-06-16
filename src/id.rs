pub fn new_id() -> String {
    format!(
        "{}-{}-{}-{}-{}",
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word()
    )
}
