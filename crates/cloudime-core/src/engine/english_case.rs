//! 英文候选的大小写档位：反引号在「全小写 → 全大写 → 首字母大写」之间轮换。

/// 一档英文候选的大小写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnglishCase {
    /// 全小写（缺省；`github` → `github`）。
    #[default]
    Lower,

    /// 全大写（`github` → `GITHUB`）。
    Upper,

    /// 首字母大写、其余小写（`github` → `Github`）。
    Title,
}

impl EnglishCase {
    /// 轮换到下一档：全小写 → 全大写 → 首字母大写 → 全小写。
    pub fn next(self) -> Self {
        match self {
            Self::Lower => Self::Upper,
            Self::Upper => Self::Title,
            Self::Title => Self::Lower,
        }
    }

    /// 把一个英文词套上这一档大小写。
    pub fn apply(self, word: &str) -> String {
        match self {
            Self::Lower => word.to_ascii_lowercase(),
            Self::Upper => word.to_ascii_uppercase(),
            Self::Title => {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                    None => String::new(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EnglishCase;

    #[test]
    fn cycles_and_cases_words() {
        assert_eq!(EnglishCase::default(), EnglishCase::Lower);
        assert_eq!(EnglishCase::Lower.next(), EnglishCase::Upper);
        assert_eq!(EnglishCase::Upper.next(), EnglishCase::Title);
        assert_eq!(EnglishCase::Title.next(), EnglishCase::Lower);
        assert_eq!(EnglishCase::Lower.apply("GitHub"), "github");
        assert_eq!(EnglishCase::Upper.apply("github"), "GITHUB");
        assert_eq!(EnglishCase::Title.apply("github"), "Github");
        assert_eq!(EnglishCase::Title.apply("GitHub"), "Github");
    }
}
