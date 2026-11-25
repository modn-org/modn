use modn_core::themes::Rule;
use modn_core::themes::{
    DeclFacts, FontFaceFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts, ThemeProblem,
    UnsupportedFacts,
};

pub fn parse_sheet(text: &str) -> Result<SheetFacts, ThemeProblem> {
    let lines: Vec<&str> = text.lines().collect();
    let mut cursor = Cursor {
        lines: &lines,
        index: 0,
    };
    Ok(SheetFacts::new(cursor.parse_rules(false)?))
}

struct Cursor<'a> {
    lines: &'a [&'a str],
    index: usize,
}

impl<'a> Cursor<'a> {
    fn line_number(&self) -> u32 {
        u32::try_from(self.index + 1).expect("a theme stylesheet has fewer than u32::MAX lines")
    }

    fn next_line(&mut self) -> Option<(&'a str, u32)> {
        while self.index < self.lines.len() {
            let number = self.line_number();
            let text = self.lines[self.index].trim();
            self.index += 1;
            if !text.is_empty() {
                return Some((text, number));
            }
        }
        None
    }

    fn parse_rules(&mut self, nested: bool) -> Result<Vec<RuleFacts>, ThemeProblem> {
        let mut rules = Vec::new();
        loop {
            let Some((text, line)) = self.next_line() else {
                if nested {
                    return Err(syntax(line_number_of(self), "unclosed block"));
                }
                return Ok(rules);
            };
            if text == "}" {
                if nested {
                    return Ok(rules);
                }
                return Err(syntax(line, "unexpected closing brace"));
            }
            rules.push(self.parse_rule(text, line)?);
        }
    }

    fn parse_rule(&mut self, text: &str, line: u32) -> Result<RuleFacts, ThemeProblem> {
        reject_comments(text, line)?;
        if text.starts_with('@') {
            return self.parse_at_rule(text, line);
        }
        let Some(header) = text.strip_suffix('{') else {
            return Err(syntax(line, "a rule must end with an opening brace"));
        };
        let selectors = parse_selectors(header, line)?;
        let declarations = self.parse_declarations()?;
        Ok(RuleFacts::Style(StyleFacts::new(
            selectors,
            declarations,
            line,
        )))
    }

    fn parse_at_rule(&mut self, text: &str, line: u32) -> Result<RuleFacts, ThemeProblem> {
        let header = text
            .strip_prefix('@')
            .expect("the caller checked the at sign");
        if let Some(statement) = header.strip_suffix(';') {
            return Ok(RuleFacts::Unsupported(UnsupportedFacts::new(
                at_rule_name(statement),
                line,
            )));
        }
        let Some(body) = header.strip_suffix('{') else {
            return Err(syntax(line, "an at-rule must end with `{` or `;`"));
        };
        if body.contains(['{', '}']) {
            return Err(syntax(line, "an at-rule header may not hold braces"));
        }
        let (name, rest) = split_name(body);
        match name {
            "font-face" => {
                if !rest.trim().is_empty() {
                    return Err(syntax(line, "@font-face takes no condition"));
                }
                Ok(RuleFacts::FontFace(FontFaceFacts::new(
                    self.parse_declarations()?,
                    line,
                )))
            }
            "media" => {
                let query = rest.trim();
                if query.is_empty() {
                    return Err(syntax(line, "@media needs a query"));
                }
                Ok(RuleFacts::Media(MediaFacts::new(
                    query,
                    self.parse_rules(true)?,
                    line,
                )))
            }
            other => {
                self.skip_block()?;
                Ok(RuleFacts::Unsupported(UnsupportedFacts::new(other, line)))
            }
        }
    }

    fn skip_block(&mut self) -> Result<(), ThemeProblem> {
        let mut depth = 1usize;
        loop {
            let Some((text, line)) = self.next_line() else {
                return Err(syntax(line_number_of(self), "unclosed block"));
            };
            reject_comments(text, line)?;
            depth += text.matches('{').count();
            depth = depth.saturating_sub(text.matches('}').count());
            if depth == 0 {
                return Ok(());
            }
        }
    }

    fn parse_declarations(&mut self) -> Result<Vec<DeclFacts>, ThemeProblem> {
        let mut declarations = Vec::new();
        loop {
            let Some((text, line)) = self.next_line() else {
                return Err(syntax(line_number_of(self), "unclosed block"));
            };
            if text == "}" {
                return Ok(declarations);
            }
            declarations.push(parse_declaration(text, line)?);
        }
    }
}

fn line_number_of(cursor: &Cursor<'_>) -> u32 {
    u32::try_from(cursor.index.max(1)).expect("a theme stylesheet has fewer than u32::MAX lines")
}

fn parse_declaration(text: &str, line: u32) -> Result<DeclFacts, ThemeProblem> {
    reject_comments(text, line)?;
    let Some(statement) = text.strip_suffix(';') else {
        return Err(syntax(line, "a declaration must end with a semicolon"));
    };
    let Some((property, value)) = statement.split_once(':') else {
        return Err(syntax(line, "a declaration needs a colon"));
    };
    let property = property.trim();
    if property.is_empty()
        || !property
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(syntax(line, "a property name is malformed"));
    }
    let value = value.trim();
    if value.is_empty() || value.contains(['{', '}', ';']) {
        return Err(syntax(line, "a declaration value is malformed"));
    }
    let (value, important) = match strip_important(value) {
        Some(rest) => (rest, true),
        None => (value, false),
    };
    Ok(DeclFacts::new(
        property.to_ascii_lowercase(),
        value,
        important,
        line,
    ))
}

fn strip_important(value: &str) -> Option<&str> {
    let lower = value.to_ascii_lowercase();
    let rest = lower.strip_suffix("!important")?;
    Some(value[..rest.len()].trim())
}

fn parse_selectors(header: &str, line: u32) -> Result<Vec<String>, ThemeProblem> {
    reject_comments(header, line)?;
    let header = header.trim();
    if header.is_empty() || header.contains(['{', '}', ';']) {
        return Err(syntax(line, "a selector list is malformed"));
    }
    let mut selectors = Vec::new();
    for selector in header.split(',') {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(syntax(line, "a selector list has an empty entry"));
        }
        selectors.push(selector.to_owned());
    }
    Ok(selectors)
}

fn reject_comments(text: &str, line: u32) -> Result<(), ThemeProblem> {
    if text.contains("/*") || text.contains("*/") {
        return Err(syntax(line, "comments are not part of the theme format"));
    }
    Ok(())
}

fn split_name(body: &str) -> (&str, &str) {
    let end = body
        .find(|character: char| character.is_ascii_whitespace() || character == '(')
        .unwrap_or(body.len());
    (&body[..end], &body[end..])
}

fn at_rule_name(statement: &str) -> String {
    split_name(statement.trim()).0.to_ascii_lowercase()
}

fn syntax(line: u32, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::CssSyntax, message).with_line(line)
}
