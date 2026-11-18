//! `LyricsRomanizer`: the Swift app has no romanizer tests, so these cases are new.
//! Expected outputs of the hand-written scripts are traced from the Swift rules, quirks included.
//! Chinese comes from Apple's ICU transform in Swift; its expected outputs are standard Hanyu Pinyin.

use trop::audio::lyrics::romanizer::lyrics_romanizer::{LyricsRomanizer, Script};

fn romanize(line: &str) -> String {
    LyricsRomanizer::romanize(line).expect("supported script")
}

#[test]
fn detection_picks_the_highest_ranked_script() {
    assert_eq!(LyricsRomanizer::romanize("Baby, I love you"), None);
    assert_eq!(LyricsRomanizer::detect_script(""), None);
    // Kana outranks han, so a Japanese line with kanji stays Japanese.
    assert_eq!(
        LyricsRomanizer::detect_script("愛してる"),
        Some(Script::Kana)
    );
    assert_eq!(LyricsRomanizer::detect_script("我爱你"), Some(Script::Han));
    assert_eq!(
        LyricsRomanizer::detect_script("사랑해 my love"),
        Some(Script::Hangul)
    );
}

#[test]
fn japanese_kana() {
    assert_eq!(romanize("さくら さくら"), "sakura sakura");
    assert_eq!(romanize("ありがとう"), "arigatou");
    assert_eq!(romanize("ちょっと待って"), "chotto待tte");
    assert_eq!(romanize("コーヒー"), "koohii");
    assert_eq!(romanize("きっと"), "kitto");
    // Kanji pass through unchanged, as in Swift.
    assert_eq!(romanize("君の名は"), "君no名ha");
}

#[test]
fn korean_revised_romanization() {
    assert_eq!(romanize("사랑해"), "saranghae");
    assert_eq!(romanize("보고 싶어"), "bogo sipeo");
    assert_eq!(romanize("ㅋㅋㅋ"), "kkk");
    assert_eq!(romanize("너를 사랑해 baby"), "neoreul saranghae baby");
}

#[test]
fn chinese_pinyin_with_tone_marks() {
    assert_eq!(romanize("我爱你"), "wǒ ài nǐ");
    assert_eq!(romanize("月亮代表我的心"), "yuè liàng dài biǎo wǒ de xīn");
    assert_eq!(romanize("你好，世界"), "nǐ hǎo，shì jiè");
}

#[test]
fn cyrillic() {
    assert_eq!(romanize("Я тебя люблю"), "Ya tebya lyublyu");
    assert_eq!(romanize("Привіт, Щастя"), "Privit, Shchastya");
    assert_eq!(romanize("Љубав"), "Ljubav");
}

#[test]
fn greek() {
    assert_eq!(romanize("Σ' αγαπώ"), "S' agapo");
    assert_eq!(romanize("Ευχαριστώ"), "Evcharisto");
    assert_eq!(romanize("άγγελος μου"), "angelos mou");
    assert_eq!(romanize("γκρεμός, άγκυρα"), "gremos, angira");
}

#[test]
fn devanagari() {
    assert_eq!(romanize("नमस्ते"), "namaste");
    assert_eq!(
        romanize("मैं तुमसे प्यार करता हूँ"),
        "main tumase pyaar karataa huun"
    );
    // ज + nukta (ज़िंदगी).
    assert_eq!(
        romanize("\u{091C}\u{093C}\u{093F}\u{0902}\u{0926}\u{0917}\u{0940}"),
        "zindagii"
    );
}

#[test]
fn thai() {
    assert_eq!(romanize("สวัสดี"), "sawatdee");
    assert_eq!(romanize("ขอบคุณ"), "khopkhun");
    assert_eq!(romanize("รักเธอ"), "raktheo");
    // ์ silences the syllable before it.
    assert_eq!(romanize("จันทร์"), "chan");
}

#[test]
fn hebrew() {
    // Dictionary words.
    assert_eq!(romanize("אני אוהב אותך"), "ani ohev otacha");
    // Letter rules with an epenthetic "e".
    assert_eq!(romanize("שמש"), "shemesh");
    assert_eq!(romanize("ילדה"), "yeleda");
    // Pointed words go through the niqqud engine.
    // שָׁלוֹם
    assert_eq!(
        romanize("\u{05E9}\u{05B8}\u{05C1}\u{05DC}\u{05D5}\u{05B9}\u{05DD}"),
        "shalom"
    );
    // בְּרֵאשִׁית
    assert_eq!(
        romanize(
            "\u{05D1}\u{05BC}\u{05B0}\u{05E8}\u{05B5}\u{05D0}\u{05E9}\u{05C1}\u{05B4}\u{05D9}\u{05EA}"
        ),
        "bereshit"
    );
}

#[test]
fn arabic() {
    // Unpointed: matres plus an epenthetic "a" between consonants.
    assert_eq!(romanize("حبيبي"), "habibi");
    assert_eq!(romanize("مرحبا"), "marahaba");
    // Unpointed article before a sun letter drops the lām.
    assert_eq!(romanize("الشمس"), "ashamas");
    // Pointed: harakat, tanwin and its seat alif.
    // شُكْرًا
    assert_eq!(
        romanize("\u{0634}\u{064F}\u{0643}\u{0652}\u{0631}\u{064B}\u{0627}"),
        "shukran"
    );
}

#[test]
fn georgian() {
    assert_eq!(romanize("გამარჯობა"), "gamarjoba");
    assert_eq!(romanize("მიყვარხარ"), "miqvarkhar");
}

#[test]
fn armenian() {
    // ու reads "ou", as in Swift.
    assert_eq!(romanize("Ես քեզ սիրում եմ"), "yes qez siroum yem");
    assert_eq!(romanize("Բարև"), "barev");
}
