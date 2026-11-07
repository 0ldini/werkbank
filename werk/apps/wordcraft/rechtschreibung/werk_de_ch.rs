//! Werkbank: the Swiss German (de-CH) spelling dictionary built into Schreibwerk, so German
//! (Switzerland) is checked out of the box, in the browser too, where no Language pack can be
//! installed. Added by werkbank's `werk/tools/rechtschreibung.py`; not part of craft-libs.
//!
//! The data is LibreOffice's `de_CH_frami` Hunspell dictionary (igerman98 by Björn Jacke, frami
//! additions by Franz Michael Baumann), licensed GPL-2.0-only OR GPL-3.0-only; its licence texts
//! are listed in About › Language data ([`licences`]) and in the NOTICE file shipped with the app.
//!
//! It is the last source for `de-ch`: a dictionaries folder, an installed `de-ch` pack or a system
//! dictionary still come first (`pack_lang::load` asks here only when none is found). The files
//! are kept raw-deflated (about a quarter of their size) and inflated on first use.

use std::path::PathBuf;
use std::sync::Arc;

use crate::affix::AffixDict;

/// `de_CH_frami.aff`, raw deflate.
static AFF: &[u8] = include_bytes!("werk_de_ch/spelling.aff.deflate");
/// `de_CH_frami.dic`, raw deflate.
static DIC: &[u8] = include_bytes!("werk_de_ch/spelling.dic.deflate");
/// Largest inflated file (as `hunspell::MAX_FILE`).
const MAX_INFLATED: usize = 64 << 20;
/// Estimated memory per byte of `.aff` and `.dic` (as `pack_lang`).
const BYTES_PER_FILE_BYTE: u64 = 6;

/// (title, text) of the dictionary's licence files.
static LICENCES: &[(&str, &str)] = &[
    ("Deutsch (Schweiz), built in: README_extension_owner.txt", include_str!("werk_de_ch/README_extension_owner.txt")),
    ("Deutsch (Schweiz), built in: README_de_DE_frami.txt", include_str!("werk_de_ch/README_de_DE_frami.txt")),
    ("Deutsch (Schweiz), built in: COPYING_GPLv2", include_str!("werk_de_ch/COPYING_GPLv2")),
    ("Deutsch (Schweiz), built in: COPYING_GPLv3", include_str!("werk_de_ch/COPYING_GPLv3")),
];

/// The built-in dictionary for `locale` (`de-ch` only), with its estimated memory, as
/// `pack_lang::load` returns one; `None` for any other locale or when it can't be read.
pub(crate) fn load(locale: &str) -> Option<(Arc<AffixDict>, Option<PathBuf>, u64)> {
    if locale != "de-ch" {
        return None;
    }
    let inflate = |what: &str, data: &[u8]| match miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATED) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("built-in de-CH spelling: {what}: {e:?}");
            None
        }
    };
    let aff = inflate("aff", AFF)?;
    let dic = inflate("dic", DIC)?;
    let size = u64::try_from(aff.len().saturating_add(dic.len())).unwrap_or(u64::MAX);
    let bytes = size.saturating_mul(BYTES_PER_FILE_BYTE).max(1);
    let (aff_text, dic_text) = match crate::hunspell::decode(&aff, &dic) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("built-in de-CH spelling: {e}");
            return None;
        }
    };
    match AffixDict::from_hunspell(&aff_text, &dic_text, &["Hunspell dictionary de_CH_frami (built into Schreibwerk)"]) {
        Ok(d) if !d.is_empty() => Some((Arc::new(d), None, bytes)),
        Ok(_) => None,
        Err(e) => {
            log::warn!("built-in de-CH spelling: {e}");
            None
        }
    }
}

/// The licence texts of the built-in dictionary, for About › Language data.
pub(crate) fn licences() -> Vec<(String, String)> {
    LICENCES.iter().map(|(t, x)| ((*t).to_string(), (*x).to_string())).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn swiss_spelling_is_built_in() {
        let (d, file, bytes) = super::load("de-ch").expect("the built-in de-CH dictionary loads");
        assert!(file.is_none() && bytes > 0);
        for w in ["Haus", "Strasse", "Wasser", "grösser", "Schule"] {
            assert!(d.check(w), "{w}");
        }
        for w in ["Wassser", "Schuule", "Straße"] {
            assert!(!d.check(w), "{w}");
        }
        assert!(super::load("de-de").is_none() && super::load("en-us").is_none());
        assert_eq!(super::licences().len(), 4);
    }
}
