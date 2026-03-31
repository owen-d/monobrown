// Many private helpers with very few public entry points, triggering code_economy.

pub struct Token {
    pub kind: u8,
    pub text: String,
}

pub struct Document {
    pub tokens: Vec<Token>,
}

// -- Public API: only two entry points --

/// Parse raw input into a document.
pub fn parse(input: &str) -> Document {
    let cleaned = strip_comments(input);
    let normalized = normalize_whitespace(&cleaned);
    let raw_tokens = tokenize(&normalized);
    let filtered = filter_empty(&raw_tokens);
    let merged = merge_adjacent(&filtered);
    let validated = validate_tokens(&merged);
    let indexed = build_index(&validated);
    let sorted = sort_by_kind(&indexed);
    Document { tokens: sorted }
}

/// Render a document back to a string.
pub fn render(doc: &Document) -> String {
    let formatted = format_tokens(&doc.tokens);
    let indented = apply_indentation(&formatted);
    let wrapped = wrap_lines(&indented, 80);
    let trimmed = trim_trailing(&wrapped);
    let numbered = add_line_numbers(&trimmed);
    join_output(&numbered)
}

// -- Private helpers: lots of internal machinery --

fn strip_comments(input: &str) -> String {
    input
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_whitespace(input: &str) -> String {
    input
        .lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tokenize(input: &str) -> Vec<Token> {
    input
        .split_whitespace()
        .enumerate()
        .map(|(i, word)| Token {
            kind: (i % 4) as u8,
            text: word.to_string(),
        })
        .collect()
}

fn filter_empty(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .filter(|t| !t.text.is_empty())
        .map(|t| Token {
            kind: t.kind,
            text: t.text.clone(),
        })
        .collect()
}

fn merge_adjacent(tokens: &[Token]) -> Vec<Token> {
    let mut merged = Vec::new();
    let mut prev: Option<Token> = None;
    for token in tokens {
        if let Some(ref mut p) = prev {
            if p.kind == token.kind {
                p.text.push(' ');
                p.text.push_str(&token.text);
                continue;
            }
            merged.push(Token {
                kind: p.kind,
                text: p.text.clone(),
            });
        }
        prev = Some(Token {
            kind: token.kind,
            text: token.text.clone(),
        });
    }
    if let Some(p) = prev {
        merged.push(p);
    }
    merged
}

fn validate_tokens(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .filter(|t| t.kind < 8 && t.text.len() < 256)
        .map(|t| Token {
            kind: t.kind,
            text: t.text.clone(),
        })
        .collect()
}

fn build_index(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .map(|t| Token {
            kind: t.kind,
            text: t.text.clone(),
        })
        .collect()
}

fn sort_by_kind(tokens: &[Token]) -> Vec<Token> {
    let mut sorted: Vec<Token> = tokens
        .iter()
        .map(|t| Token {
            kind: t.kind,
            text: t.text.clone(),
        })
        .collect();
    sorted.sort_by_key(|t| t.kind);
    sorted
}

fn format_tokens(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .map(|t| format!("[{}] {}", t.kind, t.text))
        .collect()
}

fn apply_indentation(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| format!("  {l}")).collect()
}

fn wrap_lines(lines: &[String], max_width: usize) -> Vec<String> {
    let mut wrapped = Vec::new();
    for line in lines {
        if line.len() <= max_width {
            wrapped.push(line.clone());
        } else {
            let mut remaining = line.as_str();
            while remaining.len() > max_width {
                let (chunk, rest) = remaining.split_at(max_width);
                wrapped.push(chunk.to_string());
                remaining = rest;
            }
            if !remaining.is_empty() {
                wrapped.push(remaining.to_string());
            }
        }
    }
    wrapped
}

fn trim_trailing(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn add_line_numbers(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>4} | {l}", i + 1))
        .collect()
}

fn join_output(lines: &[String]) -> String {
    lines.join("\n")
}
