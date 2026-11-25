use super::facts::DeclFacts;
use crate::themes::hooks::{
    is_protected_class, is_protected_context_class, is_protected_reachable_attribute,
    is_stable_class, is_stable_data_attribute,
};
use crate::themes::problem::Rule;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectorInfo {
    /// The selector contains a bare `:root` or `html` compound.
    pub root_only: bool,
    /// The selector names a protected hook.
    pub protected: bool,
    /// The subject compound names a hook that holds a protected region.
    pub context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectorViolation {
    /// Stable diagnostic id.
    pub rule: Rule,
    /// Explanation that names the selector or hook.
    pub message: String,
}

pub(crate) fn check_selector(
    selector: &str,
    declarations: &[DeclFacts],
) -> Result<SelectorInfo, SelectorViolation> {
    let mut parser = Parser::new(selector);
    let info = parser.parse()?;
    if info.root_only && !root_declarations_allowed(declarations) {
        return Err(forbidden(format!(
            "selector \"{selector}\" uses :root or html with declarations outside the root allowlist"
        )));
    }
    Ok(info)
}

fn root_declarations_allowed(declarations: &[DeclFacts]) -> bool {
    declarations.iter().all(|declaration| {
        declaration.property.starts_with("--modn-")
            || declaration.property == "color"
            || declaration.property == "font"
            || declaration.property.starts_with("font-")
    })
}

fn forbidden(message: String) -> SelectorViolation {
    SelectorViolation {
        rule: Rule::ForbiddenSelector,
        message,
    }
}

fn unknown_hook(selector: &str, hook: &str) -> SelectorViolation {
    SelectorViolation {
        rule: Rule::UnknownHook,
        message: format!("selector \"{selector}\" uses unknown hook \"{hook}\""),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Compound {
    simples: usize,
    hooks: usize,
    root: bool,
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    position: usize,
    root_only: bool,
    protected: bool,
    context: bool,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            position: 0,
            root_only: false,
            protected: false,
            context: false,
        }
    }

    fn parse(&mut self) -> Result<SelectorInfo, SelectorViolation> {
        self.skip_space();
        if self.at_end() || !self.input.is_ascii() {
            return Err(forbidden(format!(
                "selector \"{}\" is outside the selector grammar",
                self.input
            )));
        }
        self.parse_compound()?;
        loop {
            let had_space = self.skip_space();
            if self.at_end() {
                break;
            }
            match self.peek() {
                Some(b'>') | Some(b'+') | Some(b'~') => {
                    self.position += 1;
                    self.skip_space();
                    if self.at_end() {
                        return Err(self.forbidden_here());
                    }
                    self.parse_compound()?;
                }
                _ if had_space => self.parse_compound()?,
                _ => return Err(self.forbidden_here()),
            }
        }
        Ok(SelectorInfo {
            root_only: self.root_only,
            protected: self.protected,
            context: self.context,
        })
    }

    fn parse_compound(&mut self) -> Result<(), SelectorViolation> {
        let start = self.position;
        self.context = false;
        let mut compound = Compound::default();
        while let Some(byte) = self.peek() {
            match byte {
                b'.' => self.parse_class(&mut compound)?,
                b'[' => self.parse_attribute(&mut compound)?,
                b':' => self.parse_pseudo(&mut compound)?,
                b'h' if self.starts_with("html") && !self.ident_continues(self.position + 4) => {
                    if compound.simples > 0 {
                        return Err(self.forbidden_here());
                    }
                    self.position += 4;
                    compound.simples += 1;
                    compound.root = true;
                }
                b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => break,
                b'>' | b'+' | b'~' | b',' => break,
                _ => return Err(self.forbidden_here()),
            }
        }
        if compound.simples == 0 {
            return Err(self.forbidden_here());
        }
        if compound.root {
            if compound.simples != 1 {
                return Err(self.forbidden_here());
            }
            self.root_only = true;
        } else if compound.hooks == 0 {
            return Err(forbidden(format!(
                "selector \"{}\" has a compound without a stable hook",
                self.input
            )));
        }
        if self.position == start {
            return Err(self.forbidden_here());
        }
        Ok(())
    }

    fn parse_class(&mut self, compound: &mut Compound) -> Result<(), SelectorViolation> {
        self.position += 1;
        let Some(name) = self.read_ident() else {
            return Err(self.forbidden_here());
        };
        if !name.starts_with("modn-") {
            return Err(self.forbidden_here());
        }
        if !is_stable_class(name) {
            return Err(unknown_hook(self.input, &format!(".{name}")));
        }
        compound.simples += 1;
        compound.hooks += 1;
        if is_protected_class(name) {
            self.protected = true;
        }
        if is_protected_context_class(name) {
            self.context = true;
        }
        Ok(())
    }

    fn parse_attribute(&mut self, compound: &mut Compound) -> Result<(), SelectorViolation> {
        self.position += 1;
        self.skip_space();
        let Some(name) = self.read_ident() else {
            return Err(self.forbidden_here());
        };
        self.skip_space();
        if self.peek() != Some(b'=') {
            return Err(self.forbidden_here());
        }
        self.position += 1;
        self.skip_space();
        let value = match self.peek() {
            Some(quote @ (b'\'' | b'"')) => {
                self.position += 1;
                let start = self.position;
                while self.peek().is_some_and(|byte| byte != quote) {
                    self.position += 1;
                }
                if self.at_end() {
                    return Err(self.forbidden_here());
                }
                let value = &self.input[start..self.position];
                if value.contains('\\') {
                    return Err(self.forbidden_here());
                }
                self.position += 1;
                value
            }
            Some(_) => {
                let start = self.position;
                while self
                    .peek()
                    .is_some_and(|byte| byte != b']' && !byte.is_ascii_whitespace())
                {
                    if matches!(self.peek(), Some(b'\\' | b'|' | b'^' | b'$' | b'*' | b'~')) {
                        return Err(self.forbidden_here());
                    }
                    self.position += 1;
                }
                &self.input[start..self.position]
            }
            None => return Err(self.forbidden_here()),
        };
        if value.is_empty() {
            return Err(self.forbidden_here());
        }
        self.skip_space();
        if self.peek() != Some(b']') {
            return Err(self.forbidden_here());
        }
        self.position += 1;
        if !name.starts_with("data-") {
            return Err(self.forbidden_here());
        }
        if !is_stable_data_attribute(name) {
            return Err(unknown_hook(self.input, &format!("[{name}]")));
        }
        compound.simples += 1;
        compound.hooks += 1;
        if name == crate::themes::hooks::PROTECTED_REGION_ATTRIBUTE
            || is_protected_reachable_attribute(name)
        {
            self.protected = true;
        }
        Ok(())
    }

    fn parse_pseudo(&mut self, compound: &mut Compound) -> Result<(), SelectorViolation> {
        self.position += 1;
        if self.peek() == Some(b':') {
            return Err(self.forbidden_here());
        }
        let Some(name) = self.read_ident() else {
            return Err(self.forbidden_here());
        };
        if name == "root" {
            if compound.simples > 0 {
                return Err(self.forbidden_here());
            }
            compound.simples += 1;
            compound.root = true;
            return Ok(());
        }
        if name == "not" {
            if self.peek() != Some(b'(') {
                return Err(self.forbidden_here());
            }
            self.position += 1;
            self.skip_space();
            let mut inner = Compound::default();
            self.parse_inner_compound(&mut inner)?;
            self.skip_space();
            if self.peek() != Some(b')') || inner.simples == 0 {
                return Err(self.forbidden_here());
            }
            self.position += 1;
            compound.simples += 1;
            return Ok(());
        }
        if !matches!(
            name,
            "hover" | "focus-visible" | "active" | "disabled" | "first-child" | "last-child"
        ) {
            return Err(self.forbidden_here());
        }
        compound.simples += 1;
        Ok(())
    }

    fn parse_inner_compound(&mut self, compound: &mut Compound) -> Result<(), SelectorViolation> {
        let context = self.context;
        while let Some(byte) = self.peek() {
            match byte {
                b'.' => self.parse_class(compound)?,
                b'[' => self.parse_attribute(compound)?,
                b':' => {
                    if self.starts_with(":not(") {
                        return Err(self.forbidden_here());
                    }
                    self.parse_pseudo(compound)?;
                }
                b')' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => break,
                _ => return Err(self.forbidden_here()),
            }
        }
        if compound.hooks == 0 {
            return Err(forbidden(format!(
                "selector \"{}\" uses :not() without a stable hook",
                self.input
            )));
        }
        self.context = context;
        Ok(())
    }

    fn read_ident(&mut self) -> Option<&'a str> {
        let start = self.position;
        if !self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_' || byte == b'-')
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            self.position += 1;
        }
        Some(&self.input[start..self.position])
    }

    fn ident_continues(&self, offset: usize) -> bool {
        self.bytes
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
    }

    fn skip_space(&mut self) -> bool {
        let start = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
        self.position > start
    }

    fn starts_with(&self, text: &str) -> bool {
        self.input[self.position..].starts_with(text)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn forbidden_here(&self) -> SelectorViolation {
        forbidden(format!(
            "selector \"{}\" is outside the selector grammar",
            self.input
        ))
    }
}

#[cfg(test)]
#[path = "selector_tests.rs"]
mod tests;
