//! A test `CssChecker` that runs the real theme API v1 rule engine.

use crate::parser;
use modn_core::themes::{check_sheet, CheckedCss, CssChecker, CssContext, ThemeProblem};

/// Runs [`check_sheet`] over the facts of the strict test parser.
pub struct EngineChecker;

impl CssChecker for EngineChecker {
    fn check(
        &self,
        file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        let sheet = match parser::parse_sheet(css) {
            Ok(sheet) => sheet,
            Err(problem) => return Err(vec![problem.with_path(file)]),
        };
        match check_sheet(file, &sheet) {
            Ok(outcome) => Ok(CheckedCss::new(css, outcome.asset_urls)),
            Err(problems) => Err(problems),
        }
    }
}
