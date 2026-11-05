//! Which languages OCR can read with the Latin model.

/// Each language OCR reads with [`LATIN_RECOGNITION`](crate::LATIN_RECOGNITION),
/// as a BCP-47 primary subtag, with the letters its alphabet has beyond a–z. A
/// test checks every one of them against the model's own alphabet. Left out
/// because the model lacks letters they need: Azerbaijani (ə), Vietnamese (most
/// letters with tone marks) and Uzbek (ʻ).
pub(crate) const LATIN: &[(&str, &str)] = &[
    ("af", "êëîïôûáéíóú"),
    ("br", "âêîôûñùüé"),
    ("bs", "čćđšž"),
    ("ca", "àçèéíïòóúü"),
    ("cs", "áčďéěíňóřšťúůýž"),
    ("cy", "âêîôûŵŷáéíóúàèìòùäëïöü"),
    ("da", "æøåé"),
    ("de", "äöüß"),
    ("en", ""),
    ("eo", "ĉĝĥĵŝŭ"),
    // ¡ is not in the model; ¿ is.
    ("es", "áéíñóúü¿"),
    ("et", "äõöüšž"),
    ("eu", "ñ"),
    ("fi", "äöåšž"),
    ("fo", "áðíóúýæø"),
    ("fr", "àâæçéèêëîïôœùûüÿ"),
    ("fy", "âêéôûú"),
    ("ga", "áéíóú"),
    ("gd", "àèìòù"),
    ("gl", "áéíñóúü"),
    ("hr", "čćđšž"),
    ("ht", "èò"),
    ("hu", "áéíóöőúüű"),
    ("id", "é"),
    ("is", "áðéíóúýþæö"),
    ("it", "àèéìíîòóùú"),
    ("la", "āēīōū"),
    ("lb", "äëéè"),
    ("lt", "ąčęėįšųūž"),
    ("lv", "āčēģīķļņšūž"),
    ("mg", "àâèéêëìîïñôòù"),
    ("ms", ""),
    ("mt", "ċġħżàèìòù"),
    ("nb", "æøåéèêóòâô"),
    ("nl", "éëïóöüèáíú"),
    ("nn", "æøåéèêóòâô"),
    ("no", "æøåéèêóòâô"),
    ("oc", "àèéíòóúçï"),
    ("pl", "ąćęłńóśźż"),
    ("pt", "áâãàçéêíóôõú"),
    ("rm", "àèéìòù"),
    ("ro", "ăâîșțşţ"),
    ("sk", "áäčďéíĺľňóôŕšťúýž"),
    ("sl", "čšž"),
    ("sq", "çë"),
    ("sv", "åäöé"),
    ("sw", ""),
    ("tl", "ñ"),
    ("tr", "çğıöşüâîû"),
    ("wa", "åâêîôûçéèë"),
    ("zu", ""),
];

/// Languages written in Latin letters only when their script subtag says so.
const LATIN_WHEN_TAGGED: &[(&str, &str)] = &[("sr", "čćđšž")];

/// Can OCR read text in this language, with the recognition model paginaverto
/// names, [`LATIN_RECOGNITION`](crate::LATIN_RECOGNITION)? `language` is a
/// BCP-47 tag such as `tr`, `pt-BR` or `sr-Latn`, matched without regard to case.
///
/// True when the language is written in the Latin alphabet and the model has
/// every letter of it; false for other scripts (Cyrillic, Arabic, Greek, …), a
/// script subtag other than `Latn`, the few Latin-script languages the model
/// lacks letters for, and tags it does not know. Only Turkish and English are
/// measured; for the others, true says the letters are there, not how well
/// they are read. With ocrs's own recognition model, OCR reads ASCII only,
/// whatever this says.
pub fn recognises(language: &str) -> bool {
    let mut parts = language.split(['-', '_']);
    let primary = parts.next().unwrap_or("").to_ascii_lowercase();
    // A script subtag is four letters; a region two letters or three digits.
    let script = parts
        .find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_lowercase);
    match script.as_deref() {
        Some("latn") => LATIN
            .iter()
            .chain(LATIN_WHEN_TAGGED)
            .any(|(tag, _)| *tag == primary),
        Some(_) => false,
        None => LATIN.iter().any(|(tag, _)| *tag == primary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_languages_are_read_and_others_are_not() {
        for tag in [
            "tr",
            "TR",
            "en",
            "en-GB",
            "pt-BR",
            "de-CH",
            "sr-Latn",
            "tr-Latn-TR",
        ] {
            assert!(recognises(tag), "{tag}");
        }
        for tag in [
            "ru", "uk", "el", "ar", "he", "hi", "zh", "ja", "ko", "th", "sr", "sr-Cyrl", "tr-Arab",
            "az", "vi", "uz", "", "xx",
        ] {
            assert!(!recognises(tag), "{tag}");
        }
    }

    /// Every letter the table lists, in both cases, is one the model reads.
    #[cfg(feature = "ocr")]
    #[test]
    fn every_listed_letter_is_in_the_model() {
        let alphabet = paginaverto_ocr::latin_alphabet();
        for (tag, letters) in LATIN.iter().chain(LATIN_WHEN_TAGGED) {
            let all = format!("abcdefghijklmnopqrstuvwxyz{letters}");
            for c in all.chars().chain(all.to_uppercase().chars()) {
                // ß has no single capital in common use: SS.
                if c.is_alphabetic() || c == '¿' {
                    assert!(alphabet.contains(&c), "{tag}: {c}");
                }
            }
        }
    }
}
