use anyhow::{bail, Context, Result};
use std::cmp::Ordering;
use std::fmt;

const MAX_COMPONENTS: usize = 8;
const MAX_COMPARATORS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Less,
    LessOrEqual,
    Equal,
    GreaterOrEqual,
    Greater,
}

/// A parsed version requirement, such as `>=1.31.0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionReq {
    text: String,
    comparators: Vec<(Op, Vec<u64>)>,
}

impl VersionReq {
    /// Parse comma-separated comparators: `>=`, `>`, `<=`, `<` or `=`.
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if parts.len() > MAX_COMPARATORS {
            bail!("version requirement has more than {MAX_COMPARATORS} comparators: {text:?}");
        }
        let comparators = parts
            .into_iter()
            .map(|part| {
                let (op, version) = [
                    (">=", Op::GreaterOrEqual),
                    ("<=", Op::LessOrEqual),
                    (">", Op::Greater),
                    ("<", Op::Less),
                    ("=", Op::Equal),
                ]
                .into_iter()
                .find_map(|(prefix, op)| part.strip_prefix(prefix).map(|rest| (op, rest)))
                .with_context(|| format!("version comparator needs an operator: {part:?}"))?;
                Ok((
                    op,
                    numeric(version.trim())
                        .with_context(|| format!("invalid version in requirement {text:?}"))?,
                ))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            text: text.trim().to_owned(),
            comparators,
        })
    }

    /// Whether `version` meets every comparator.
    ///
    /// A version that is not dotted numeric cannot be compared, so it is an
    /// error rather than a silent match.
    pub fn matches(&self, version: &str) -> Result<bool> {
        let version = numeric(version)
            .with_context(|| format!("cannot compare version {version:?} with {self}"))?;
        Ok(self.comparators.iter().all(|(op, required)| {
            let order = compare(&version, required);
            match op {
                Op::Less => order == Ordering::Less,
                Op::LessOrEqual => order != Ordering::Greater,
                Op::Equal => order == Ordering::Equal,
                Op::GreaterOrEqual => order != Ordering::Less,
                Op::Greater => order == Ordering::Greater,
            }
        }))
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

fn numeric(version: &str) -> Result<Vec<u64>> {
    let digits = version.strip_prefix('v').unwrap_or(version);
    let components: Vec<&str> = digits.split('.').collect();
    if components.len() > MAX_COMPONENTS
        || components
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("not a dotted numeric version: {version:?}");
    }
    components
        .into_iter()
        .map(|part| part.parse().context("version component too large"))
        .collect()
}

fn compare(left: &[u64], right: &[u64]) -> Ordering {
    let length = left.len().max(right.len());
    (0..length)
        .map(|i| {
            let a = left.get(i).copied().unwrap_or(0);
            let b = right.get(i).copied().unwrap_or(0);
            a.cmp(&b)
        })
        .find(|order| order.is_ne())
        .unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holds(requirement: &str, version: &str) -> bool {
        VersionReq::parse(requirement)
            .expect("requirement")
            .matches(version)
            .expect("comparable")
    }

    #[test]
    fn comparators_follow_numeric_order() {
        assert!(holds(">=1.31.0", "1.31.0"));
        assert!(holds(">=1.31.0", "1.31.1"));
        assert!(!holds(">=1.31.0", "1.30.9"));
        assert!(holds(">=0.5.32", "v0.5.32"));
        assert!(holds("=0.8", "0.8.0"));
        assert!(holds(">1.9", "1.10"));
        assert!(holds(">=1.28.0, <2", "1.28.6"));
        assert!(!holds(">=1.28.0, <2", "2.0"));
        assert!(holds("<=1.2", "1.2.0"));
    }

    #[test]
    fn malformed_requirements_and_versions_are_errors() {
        for requirement in ["1.0", ">=", ">=1.x", "~1.0", ">=1,<2,<3,<4,<5", ">=1..2"] {
            assert!(VersionReq::parse(requirement).is_err(), "{requirement}");
        }
        let requirement = VersionReq::parse(">=1.0").expect("requirement");
        let error = requirement.matches("1.0-beta").unwrap_err();
        assert!(format!("{error:#}").contains("cannot compare version \"1.0-beta\" with >=1.0"));
    }
}
