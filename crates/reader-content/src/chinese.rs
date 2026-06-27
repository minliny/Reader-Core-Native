//! Legado `ChineseUtils` / `ContentProcessor.chineseConverterType` parity.
//!
//! Mirrors `legado/.../utils/ChineseUtils.kt` (s2t / t2s backed by the
//! quick-transfer full TS/ST dictionaries) and the `chineseConvert` branch of
//! `legado/.../help/book/ContentProcessor.kt:135-145`.
//!
//! Backed by [`zhhz`](https://crates.io/crates/zhhz) — a pure-Rust OpenCC
//! reimplementation with embedded dictionaries (no C deps, no runtime data
//! download). Converters are cached per-thread via `OnceCell` because building
//! them is ~ms and `RemoteContentPipeline` is cloned cheaply per request.

/// Legado `AppConfig.chineseConverterType` mirror (ContentProcessor.kt:135).
///
/// - 0 / `None` — no conversion (default)
/// - 1 / `T2S`   — 繁体 → 简体 (`ChineseUtils.t2s`)
/// - 2 / `S2T`   — 简体 → 繁体 (`ChineseUtils.s2t`)
///
/// Applied to chapter body text *before* replace rules, mirroring Legado
/// `ContentProcessor.getContent` ordering (chineseConvert branch runs before
/// the useReplace branch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChineseConverterType {
    #[default]
    None,
    T2S,
    S2T,
}

impl ChineseConverterType {
    /// Parse Legado's persisted int config. Unknown values map to `None`
    /// (Legado treats anything != 1/2 as "no conversion").
    pub fn from_legado_config(value: u8) -> Self {
        match value {
            1 => ChineseConverterType::T2S,
            2 => ChineseConverterType::S2T,
            _ => ChineseConverterType::None,
        }
    }
}

/// Run Chinese simplified↔traditional conversion on `content` per `converter`.
///
/// `ChineseConverterType::None` returns the input unchanged. `T2S`/`S2T` use
/// cached per-thread `zhhz::Converter` instances (OpenCC standard TS/ST
/// dictionaries).
pub fn convert_chinese(content: &str, converter: ChineseConverterType) -> String {
    match converter {
        ChineseConverterType::None => content.to_string(),
        ChineseConverterType::T2S => {
            thread_local! {
                static T2S: std::cell::OnceCell<zhhz::Converter> = const { std::cell::OnceCell::new() };
            }
            T2S.with(|cell| {
                cell.get_or_init(|| zhhz::Converter::new(zhhz::Config::T2s))
                    .convert(content)
            })
        }
        ChineseConverterType::S2T => {
            thread_local! {
                static S2T: std::cell::OnceCell<zhhz::Converter> = const { std::cell::OnceCell::new() };
            }
            S2T.with(|cell| {
                cell.get_or_init(|| zhhz::Converter::new(zhhz::Config::S2t))
                    .convert(content)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_converter_type_from_legado_config_parses_int_values() {
        assert_eq!(
            ChineseConverterType::from_legado_config(0),
            ChineseConverterType::None
        );
        assert_eq!(
            ChineseConverterType::from_legado_config(1),
            ChineseConverterType::T2S
        );
        assert_eq!(
            ChineseConverterType::from_legado_config(2),
            ChineseConverterType::S2T
        );
        assert_eq!(
            ChineseConverterType::from_legado_config(99),
            ChineseConverterType::None
        );
        assert_eq!(ChineseConverterType::default(), ChineseConverterType::None);
    }

    #[test]
    fn convert_chinese_matches_legado_chinese_utils_full_dictionary_paths() {
        // Legado parity: ChineseUtils.t2s / s2t backed by quick-transfer full
        // TS/ST dictionaries. zhhz (pure-Rust OpenCC) must produce the same
        // full-dictionary output for pairs the old 20-char stub could not
        // handle (測試, 計算機, etc.).
        assert_eq!(convert_chinese("測試", ChineseConverterType::T2S), "测试");
        assert_eq!(convert_chinese("测试", ChineseConverterType::S2T), "測試");
        assert_eq!(
            convert_chinese("計算機", ChineseConverterType::T2S),
            "计算机"
        );
        assert_eq!(
            convert_chinese("计算机", ChineseConverterType::S2T),
            "計算機"
        );
        // None passes through unchanged.
        assert_eq!(convert_chinese("測試", ChineseConverterType::None), "測試");
    }
}
