use rowan::SmolStr;

// - Lossless
// - Incremental
// - Error-Recovery
// - Parent-Pointer
// - Syntax-Highlighting


#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(non_camel_case_types)]
#[repr(u16)]
enum SyntaxKind {
    L_PAREN = 0, // '('
    R_PAREN,     // ')'
    LAM,         // '\'
    ARROW,       // '->'
    WORD,        // 'x'
    WHITESPACE,  // whitespaces is explicit
    ERROR,       // as well as errors

    // composite nodes
    PARENTHESIZED, // `(+ 2 3)`
    VAR, // `+`, `15`, wraps a WORD token
    LAMBDA, // a Lambda abstraction
    APPLICATION, // a function application
    ROOT, // The top-level node
}

use SyntaxKind::*;

/// Some boilerplate is needed, as rowan settled on using its own
/// `struct SyntaxKind(u16)` internally, instead of accepting the
/// user's `enum SyntaxKind` as a type parameter.
///
/// First, to easily pass the enum variants into rowan via `.into()`:
impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// Second, implementing the `Language` trait teaches rowan to convert between
/// these two SyntaxKind types, allowing for a nicer SyntaxNode API where
/// "kinds" are values from our `enum SyntaxKind`, instead of plain u16 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Lang {}
impl rowan::Language for Lang {
    type Kind = SyntaxKind;
    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        assert!(raw.0 <= ROOT as u16);
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }
    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// GreenNode is an immutable tree, which is cheap to change,
/// but doesn't contain offsets and parent pointers.
use rowan::GreenNode;

/// You can construct GreenNodes by hand, but a builder
/// is helpful for top-down parsers: it maintains a stack
/// of currently in-progress nodes
use rowan::GreenNodeBuilder;
use crate::ExprRes::Eof;

/// The parse results are stored as a "green tree".
/// We'll discuss working with the results later
struct Parse {
    green_node: GreenNode,
    #[allow(unused)]
    errors: Vec<String>,
}

struct Parser {
    /// input tokens, including whitespace,
    /// in *reverse* order.
    tokens: Vec<(SyntaxKind, SmolStr)>,
    /// the in-progress tree.
    builder: GreenNodeBuilder<'static>,
    /// the list of syntax errors we've accumulated
    /// so far.
    errors: Vec<String>,
}

/// The outcome of parsing a single S-expression
enum ExprRes {
    /// An expression was successfully parsed
    Ok,
    /// Nothing was parsed, as no significant tokens remained
    Eof,
    /// An unexpected ')' was found
    Lul(String),
}

impl Parser {
    fn parse(mut self) -> Parse {
        // Make sure that the root node covers all source
        self.builder.start_node(ROOT.into());
        // Parse zero or more S-expressions
        let _ = self.expr();
        // Don't forget to eat *trailing* whitespace
        self.skip_ws();
        if let Some(token) = self.current() {
            self.errors.push(format!("Unexpected token {:?}", token))
        }
        // Close the root node.
        self.builder.finish_node();

        // Turn the builder into a GreenNode
        Parse { green_node: self.builder.finish(), errors: self.errors }
    }
    fn expr(&mut self) -> ExprRes {
        let mut is_application = false;
        self.skip_ws();
        let checkpoint = self.builder.checkpoint();
        match self.atom() {
            None => return ExprRes::Lul("Expected expression".to_string()),
            Some(ExprRes::Ok) => (),
            Some(res) => return res,
        }
        loop {
            match self.atom() {
                None => break,
                Some(ExprRes::Eof) => break,
                Some(ExprRes::Lul(s)) => {
                    self.builder.start_node(ERROR.into());
                    self.errors.push(s);
                    self.builder.finish_node();
                }
                Some(ExprRes::Ok) => {
                    self.builder.start_node_at(checkpoint, APPLICATION.into());
                    is_application = true;
                },
            }
        }
        if is_application {
            self.builder.finish_node();
        }
        ExprRes::Ok
    }

    fn atom(&mut self) -> Option<ExprRes> {
        // Eat leading whitespace
        self.skip_ws();
        // Either a list, an atom, a closing paren,
        // or an eof.
        match self.current() {
            Some(L_PAREN) => {
                self.builder.start_node(PARENTHESIZED.into());
                self.bump();
                self.expr();
                match self.current() {
                    None => {
                        self.builder.finish_node();
                        return Some(ExprRes::Lul("MISSING CLOSING PAREN".to_string()))
                    }
                    Some(R_PAREN) => {
                        self.bump();
                        self.builder.finish_node();
                    }
                    Some(_) => {
                        self.builder.finish_node();
                        return Some(ExprRes::Lul("UNEXPECTED".to_string()))
                    }
                }
            },
            Some(WORD) => {
                self.builder.start_node(VAR.into());
                self.bump();
                self.builder.finish_node();
            }
            Some(LAM) => {
                self.builder.start_node(LAMBDA.into());
                self.bump(); // \
                self.skip_ws();
                self.bump(); // var
                self.skip_ws();
                self.bump(); // ->
                self.expr();
                self.builder.finish_node()
            }
            Some(ERROR) => self.bump(),
            _ => return None,
        }
        Some(ExprRes::Ok)
    }



    /// Advance one token, adding it to the current branch of the tree builder.
    fn bump(&mut self) {
        let (kind, text) = self.tokens.pop().unwrap();
        self.builder.token(kind.into(), text);
    }
    /// Peek at the first unprocessed token
    fn current(&self) -> Option<SyntaxKind> {
        self.tokens.last().map(|(kind, _)| *kind)
    }
    fn skip_ws(&mut self) {
        while self.current() == Some(WHITESPACE) {
            self.bump()
        }
    }
}

fn parse(text: &str) -> Parse {
    let mut tokens = lex(text);
    tokens.reverse();
    Parser { tokens, builder: GreenNodeBuilder::new(), errors: Vec::new() }.parse()
}

type SyntaxNode = rowan::SyntaxNode<Lang>;
#[allow(unused)]
type SyntaxToken = rowan::SyntaxToken<Lang>;
#[allow(unused)]
type SyntaxElement = rowan::NodeOrToken<SyntaxNode, SyntaxToken>;

impl Parse {
    fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green_node.clone())
    }
}


/// Split the input string into a flat list of tokens
/// (such as L_PAREN, WORD, and WHITESPACE)
fn lex(text: &str) -> Vec<(SyntaxKind, SmolStr)> {
    fn tok(t: SyntaxKind) -> m_lexer::TokenKind {
        m_lexer::TokenKind(rowan::SyntaxKind::from(t).0)
    }
    fn kind(t: m_lexer::TokenKind) -> SyntaxKind {
        match t.0 {
            0 => L_PAREN,
            1 => R_PAREN,
            2 => LAM,
            3 => ARROW,
            4 => WORD,
            5 => WHITESPACE,
            6 => ERROR,
            _ => unreachable!(),
        }
    }

    let lexer = m_lexer::LexerBuilder::new()
        .error_token(tok(ERROR))
        .tokens(&[
            (tok(L_PAREN), r"\("),
            (tok(R_PAREN), r"\)"),
            (tok(LAM), r"\\"),
            (tok(ARROW), r"->"),
            (tok(WORD), r"[\w]+"),
            (tok(WHITESPACE), r"\s+"),
        ])
        .build();

    lexer
        .tokenize(text)
        .into_iter()
        .map(|t| (t.len, kind(t.kind)))
        .scan(0usize, |start_offset, (len, kind)| {
            let s: SmolStr = text[*start_offset..*start_offset + len].into();
            *start_offset += len;
            Some((kind, s))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn it_lexes() {
        let text = r"(\x -> x)";
        let mut tokens = lex(text);
        println!("{:?}", tokens)
    }

    #[test]
    fn it_parses() {
        let text = r"(\x -> x) (";
        let parse = parse(text);
        let mut node = parse.syntax();
        println!("{}\n{:#?}\n{:#?}", text, parse.errors, node);
    }
}
