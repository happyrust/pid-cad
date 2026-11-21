//! 规则表达式（`SPEC-pid-drc.md` §3.3）：按文法把表达式解析成语法树，在一个对象上按
//! 三值逻辑求值。
//!
//! 解析时就查名字：多段路径的第一段必须是作用对象认识的关系或根（§3.4 的名字表，
//! 见 [`model`](super::model)），`MATCH` 后面写死的正则必须能编译。取值都经 [`Facts`]：求值
//! 只认路径，不管值从哪里来，往后每加一组事实都不用动这里。

use std::cmp::Ordering;
use std::fmt;

use regex::Regex;

use super::model::{root, Kind, Next, Target, ROOTS};
use crate::pid::master_data::parse_dn;

/// 解析错误的错误码（§3.8）。
pub const DSL_PARSE_ERROR: &str = "DSL_PARSE_ERROR";

// 解析错误里的 `expected`：这里本该出现什么。
const START: &str = "路径、左括号或 NOT";
const AFTER_NOT: &str = "路径或左括号";
const OPERATOR: &str = "运算符（=、!=、>、<、>=、<=、IN、NOT IN、MATCH、NOT MATCH）";
const AFTER_NOT_OP: &str = "IN 或 MATCH";
const OPERAND: &str = "值或路径";
const LIST_NEXT: &str = "逗号或右括号";
const SINGLE: &str = "单个值或路径（只有 IN、NOT IN 后面能写列表）";
const NAME: &str = "名字";
const CLOSE: &str = "AND、OR 或右括号";
const END: &str = "AND、OR 或结尾";
const QUOTE: &str = "右引号";
const REGEX: &str = "合法的正则表达式";

/// 表达式解析不过（§3.3）。`offset` 是出错处在表达式里的偏移，从 0 数，数的是字符不是
/// 字节；`expected` 是这里本该出现什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DslError {
    pub offset: usize,
    pub expected: String,
}

impl DslError {
    /// 错误码，恒为 [`DSL_PARSE_ERROR`]。
    pub fn code(&self) -> &'static str {
        DSL_PARSE_ERROR
    }
}

impl fmt::Display for DslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: 偏移 {} 处应为{}",
            self.code(),
            self.offset,
            self.expected
        )
    }
}

impl std::error::Error for DslError {}

/// 解析出来的表达式。`Or`、`And` 里至少有两项，按写的顺序。
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Or(Vec<Expr>),
    And(Vec<Expr>),
    Not(Box<Expr>),
    Compare(Comparison),
}

/// 一个比较：左边总是路径。
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub left: Path,
    pub op: Op,
    pub right: Operand,
}

/// 比较运算符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    In,
    NotIn,
    Match,
    NotMatch,
}

/// 比较的右边。
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Value(Literal),
    Path(Path),
    /// 括号里的列表，只出现在 `IN`、`NOT IN` 后面，可以套列表。别的运算符后面写一项的
    /// 括号，解析时就拆掉了。
    List(Vec<Operand>),
    /// `MATCH`、`NOT MATCH` 后面写死的正则，解析时编译好。
    Pattern(Pattern),
}

/// 写在表达式里的值。值都是文本：字符串去掉引号，`#蒸汽` 是 `蒸汽`，数和 `TRUE`、
/// `FALSE` 照写的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Text(String),
    Null,
}

/// 一条路径：用 `.` 连起来的几段名字。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path {
    pub segments: Vec<String>,
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.segments.join("."))
    }
}

/// 编译好的正则。两个正则写法相同就算相等。
#[derive(Clone)]
pub struct Pattern(Regex);

impl Pattern {
    /// 正则的写法。
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl fmt::Debug for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pattern({:?})", self.as_str())
    }
}

/// 三值逻辑的值：真、假、不知道。只有真才算命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    /// 是不是命中：不知道不命中。
    pub fn is_true(self) -> bool {
        self == Self::True
    }

    /// `AND`：遇假为假，两边都真为真，其余不知道。
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    /// `OR`：遇真为真，两边都假为假，其余不知道。
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}

impl std::ops::Not for Truth {
    type Output = Self;

    /// `NOT`：真假互换，不知道仍是不知道。
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl From<bool> for Truth {
    fn from(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }
}

/// 一条路径在当前对象上取到的东西。
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// 一个值；没有值是 `None`。空串、只有空白的也算没有值。
    Value(Option<String>),
    /// 一张表（`项目.管道等级表`、写成列表的参数）；取不出来（主数据没配置）是 `None`。
    List(Option<FactList>),
}

/// 一张表的内容，和表里的值怎么跟左边比。
#[derive(Debug, Clone, PartialEq)]
pub struct FactList {
    pub items: Vec<String>,
    pub matching: Matching,
}

/// `IN` 拿左边跟表里的值怎么比。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matching {
    /// 和 `=` 一样。
    Equal,
    /// 公称直径：按主数据规格 §3.5 读成数再比，`DN50` 与 `50` 是同一个。
    NominalDiameter,
}

/// 事实提供者：表达式里的路径在当前对象上取值，都从这里拿。
///
/// 单段的名字先看是不是这种对象的派生事实，不是就当属性名；多段的沿关系去取，沿途取不到
/// 对象就是没有值（§3.3）。这些由实现的一方管，求值只管比。
pub trait Facts {
    fn fact(&self, path: &Path) -> Fact;
}

// ── 解析 ─────────────────────────────────────────────────────────────────────

/// 按 §3.3 的文法解析 `expr`，路径按 `target` 这种对象认关系名。
pub fn parse(expr: &str, target: &Target) -> Result<Expr, DslError> {
    let mut parser = Parser {
        tokens: lex(expr),
        at: 0,
        kind: target.kind(),
    };
    let tree = parser.expression()?;
    parser.finish()?;
    Ok(tree)
}

/// 记号。
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Number(String),
    /// 字符串，不含引号。
    Text(String),
    /// 枚举引用，不含 `#`。
    Enum(String),
    And,
    Or,
    Not,
    In,
    Match,
    True,
    False,
    Null,
    Compare(Op),
    LeftParen,
    RightParen,
    Comma,
    Dot,
    /// 没有右引号的字符串，一直到结尾。
    Unterminated,
    /// 文法里没有的字符。
    Bad,
    End,
}

/// 标识符的首字符（§3.3 的 letter）：ASCII 字母或 U+4E00 到 U+9FFF 的汉字。
fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&c)
}

fn is_name_char(c: char) -> bool {
    is_letter(c) || c.is_ascii_digit() || c == '_'
}

/// 从 `from` 起连续满足 `keep` 的字符到哪里为止。
fn scan(chars: &[char], from: usize, keep: impl Fn(char) -> bool) -> usize {
    chars[from..]
        .iter()
        .position(|&c| !keep(c))
        .map_or(chars.len(), |len| from + len)
}

/// 关键字只认大写。
fn keyword(word: &str) -> Option<Token> {
    let token = match word {
        "AND" => Token::And,
        "OR" => Token::Or,
        "NOT" => Token::Not,
        "IN" => Token::In,
        "MATCH" => Token::Match,
        "TRUE" => Token::True,
        "FALSE" => Token::False,
        "NULL" => Token::Null,
        _ => return None,
    };
    Some(token)
}

/// 把表达式切成记号，每个记号带它第一个字符的偏移（按字符数）。最后总有一个 `End`，
/// 偏移是字符总数。
fn lex(expr: &str) -> Vec<(usize, Token)> {
    let chars: Vec<char> = expr.chars().collect();
    let digit = |c: char| c.is_ascii_digit();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let (start, c, next) = (at, chars[at], chars.get(at + 1).copied());
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        let token = if is_letter(c) {
            at = scan(&chars, at, is_name_char);
            let word: String = chars[start..at].iter().collect();
            keyword(&word).unwrap_or(Token::Name(word))
        } else if digit(c) || (c == '-' && next.is_some_and(digit)) {
            at = scan(&chars, at + 1, digit);
            if chars.get(at) == Some(&'.') && chars.get(at + 1).copied().is_some_and(digit) {
                at = scan(&chars, at + 1, digit);
            }
            Token::Number(chars[start..at].iter().collect())
        } else if c == '"' {
            match chars[at + 1..].iter().position(|&ch| ch == '"') {
                Some(len) => {
                    at += len + 2;
                    Token::Text(chars[start + 1..at - 1].iter().collect())
                }
                None => {
                    at = chars.len();
                    Token::Unterminated
                }
            }
        } else if c == '#' && next.is_some_and(is_letter) {
            at = scan(&chars, at + 1, is_name_char);
            Token::Enum(chars[start + 1..at].iter().collect())
        } else {
            let (token, len) = match (c, next) {
                ('!', Some('=')) => (Token::Compare(Op::Ne), 2),
                ('>', Some('=')) => (Token::Compare(Op::Ge), 2),
                ('<', Some('=')) => (Token::Compare(Op::Le), 2),
                ('=', _) => (Token::Compare(Op::Eq), 1),
                ('>', _) => (Token::Compare(Op::Gt), 1),
                ('<', _) => (Token::Compare(Op::Lt), 1),
                ('(', _) => (Token::LeftParen, 1),
                (')', _) => (Token::RightParen, 1),
                (',', _) => (Token::Comma, 1),
                ('.', _) => (Token::Dot, 1),
                _ => (Token::Bad, 1),
            };
            at += len;
            token
        };
        tokens.push((start, token));
    }
    tokens.push((chars.len(), Token::End));
    tokens
}

/// 递归下降：文法的一个产生式一个方法。
struct Parser {
    tokens: Vec<(usize, Token)>,
    at: usize,
    kind: Kind,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at].1
    }

    fn offset(&self) -> usize {
        self.tokens[self.at].0
    }

    /// 往后走一个记号；到了 `End` 就停在那里。
    fn advance(&mut self) {
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
    }

    /// 当前记号是 `token` 就吃掉它。
    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == token;
        if found {
            self.advance();
        }
        found
    }

    /// 当前记号不是这里要的。没收尾的字符串不管出现在哪里，都报缺右引号，位置在结尾。
    fn error<T>(&self, expected: &str) -> Result<T, DslError> {
        let unterminated = *self.peek() == Token::Unterminated;
        let end = self.tokens.last().map_or(0, |(offset, _)| *offset);
        Err(DslError {
            offset: if unterminated { end } else { self.offset() },
            expected: if unterminated { QUOTE } else { expected }.to_owned(),
        })
    }

    /// expression = and_expr { "OR" and_expr }
    fn expression(&mut self) -> Result<Expr, DslError> {
        let mut terms = vec![self.conjunction()?];
        while self.eat(&Token::Or) {
            terms.push(self.conjunction()?);
        }
        Ok(join(terms, Expr::Or))
    }

    /// and_expr = not_expr { "AND" not_expr }
    fn conjunction(&mut self) -> Result<Expr, DslError> {
        let mut terms = vec![self.negation()?];
        while self.eat(&Token::And) {
            terms.push(self.negation()?);
        }
        Ok(join(terms, Expr::And))
    }

    /// not_expr = [ "NOT" ] atom
    fn negation(&mut self) -> Result<Expr, DslError> {
        if self.eat(&Token::Not) {
            Ok(Expr::Not(Box::new(self.atom(AFTER_NOT)?)))
        } else {
            self.atom(START)
        }
    }

    /// atom = "(" expression ")" | comparison
    fn atom(&mut self, expected: &str) -> Result<Expr, DslError> {
        match self.peek() {
            Token::LeftParen => {
                self.advance();
                let inner = self.expression()?;
                if !self.eat(&Token::RightParen) {
                    return self.error(CLOSE);
                }
                Ok(inner)
            }
            Token::Name(_) => self.comparison(),
            _ => self.error(expected),
        }
    }

    /// comparison = path operator operand
    fn comparison(&mut self) -> Result<Expr, DslError> {
        let left = self.path()?;
        let op = self.operator()?;
        let right = self.right_side(op)?;
        Ok(Expr::Compare(Comparison { left, op, right }))
    }

    /// operator：`NOT` 后面只能是 `IN` 或 `MATCH`。
    fn operator(&mut self) -> Result<Op, DslError> {
        let negated = self.eat(&Token::Not);
        let op = match (self.peek(), negated) {
            (Token::Compare(op), false) => *op,
            (Token::In, false) => Op::In,
            (Token::Match, false) => Op::Match,
            (Token::In, true) => Op::NotIn,
            (Token::Match, true) => Op::NotMatch,
            (_, false) => return self.error(OPERATOR),
            (_, true) => return self.error(AFTER_NOT_OP),
        };
        self.advance();
        Ok(op)
    }

    /// 比较的右边。只有 `IN`、`NOT IN` 后面能写多项的列表，别的运算符后面写一项的括号当分组
    /// 拆掉；`MATCH`、`NOT MATCH` 后面写死的正则在这里编译。
    fn right_side(&mut self, op: Op) -> Result<Operand, DslError> {
        let offset = self.offset();
        let operand = self.operand()?;
        if matches!(op, Op::In | Op::NotIn) {
            return Ok(operand);
        }
        match single(operand, offset)? {
            Operand::Value(Literal::Text(text))
                if matches!(op, Op::Match | Op::NotMatch) && !text.trim().is_empty() =>
            {
                Regex::new(&text)
                    .map(|regex| Operand::Pattern(Pattern(regex)))
                    .map_err(|_| DslError {
                        offset,
                        expected: REGEX.to_owned(),
                    })
            }
            operand => Ok(operand),
        }
    }

    /// operand = value | path | "(" operand { "," operand } ")"
    fn operand(&mut self) -> Result<Operand, DslError> {
        let literal = match self.peek() {
            Token::Name(_) => return self.path().map(Operand::Path),
            Token::LeftParen => return self.list(),
            Token::Null => Literal::Null,
            Token::Text(text) | Token::Number(text) | Token::Enum(text) => {
                Literal::Text(text.clone())
            }
            Token::True => Literal::Text("TRUE".to_owned()),
            Token::False => Literal::Text("FALSE".to_owned()),
            _ => return self.error(OPERAND),
        };
        self.advance();
        Ok(Operand::Value(literal))
    }

    /// "(" operand { "," operand } ")"
    fn list(&mut self) -> Result<Operand, DslError> {
        self.advance();
        let mut items = vec![self.operand()?];
        while self.eat(&Token::Comma) {
            items.push(self.operand()?);
        }
        if !self.eat(&Token::RightParen) {
            return self.error(LIST_NEXT);
        }
        Ok(Operand::List(items))
    }

    /// path = segment { "." segment }。读完就按作用对象查名字。
    fn path(&mut self) -> Result<Path, DslError> {
        let mut segments = Vec::new();
        loop {
            let Token::Name(name) = self.peek() else {
                return self.error(NAME);
            };
            segments.push((self.offset(), name.clone()));
            self.advance();
            if !self.eat(&Token::Dot) {
                break;
            }
        }
        check_path(&segments, self.kind, true)?;
        Ok(Path {
            segments: segments.into_iter().map(|(_, name)| name).collect(),
        })
    }

    /// 整个表达式读完之后只能是结尾。
    fn finish(&self) -> Result<(), DslError> {
        if *self.peek() == Token::End {
            Ok(())
        } else {
            self.error(END)
        }
    }
}

/// 只有一项的 `OR`、`AND` 就是那一项。
fn join(mut terms: Vec<Expr>, combine: fn(Vec<Expr>) -> Expr) -> Expr {
    if terms.len() == 1 {
        terms.swap_remove(0)
    } else {
        combine(terms)
    }
}

/// 一项的括号拆掉（可以套几层）；多项的列表报错，位置在左括号。
fn single(operand: Operand, offset: usize) -> Result<Operand, DslError> {
    match operand {
        Operand::List(mut items) if items.len() == 1 => single(items.swap_remove(0), offset),
        Operand::List(_) => Err(DslError {
            offset,
            expected: SINGLE.to_owned(),
        }),
        operand => Ok(operand),
    }
}

/// 多段路径的名字要对得上 §3.4：第一段是 `kind` 的关系（`top` 时也可以是根），后面按
/// 关系接。单段的不查：不是派生事实就当属性名，属性名不限。
fn check_path(segments: &[(usize, String)], kind: Kind, top: bool) -> Result<(), DslError> {
    let [(offset, first), rest @ ..] = segments else {
        return Ok(());
    };
    let Some((second_offset, second)) = rest.first() else {
        return Ok(());
    };
    let next = kind.relation(first).or_else(|| root(first).filter(|_| top));
    match next {
        None => Err(DslError {
            offset: *offset,
            expected: relation_names(kind, top),
        }),
        Some(Next::Object(kind)) => check_path(rest, kind, false),
        Some(Next::OneOf(names)) if !names.contains(&second.as_str()) => Err(DslError {
            offset: *second_offset,
            expected: format!("{first} 后面的名字：{}", names.join("、")),
        }),
        Some(Next::OneOf(_) | Next::AnyName) => match rest.get(1) {
            Some((third_offset, _)) => Err(DslError {
                offset: *third_offset,
                expected: format!("运算符（{first}.{second} 不能再往下取）"),
            }),
            None => Ok(()),
        },
    }
}

/// 报错时列出这里能写的关系名。
fn relation_names(kind: Kind, top: bool) -> String {
    let roots = if top { ROOTS } else { &[] };
    let names: Vec<&str> = kind
        .relations()
        .iter()
        .chain(roots)
        .map(|relation| relation.name)
        .collect();
    let what = if top {
        "关系或根".to_owned()
    } else {
        format!("{}的关系", kind.name())
    };
    format!("{what}：{}", names.join("、"))
}

// ── 求值 ─────────────────────────────────────────────────────────────────────

impl Expr {
    /// 在一个对象上求值，取值都经 `facts`。`AND`、`OR` 结果定了就不再往下算。
    pub fn eval(&self, facts: &dyn Facts) -> Truth {
        match self {
            Expr::Or(terms) => {
                let mut truth = Truth::False;
                for term in terms {
                    truth = truth.or(term.eval(facts));
                    if truth == Truth::True {
                        break;
                    }
                }
                truth
            }
            Expr::And(terms) => {
                let mut truth = Truth::True;
                for term in terms {
                    truth = truth.and(term.eval(facts));
                    if truth == Truth::False {
                        break;
                    }
                }
                truth
            }
            Expr::Not(inner) => !inner.eval(facts),
            Expr::Compare(comparison) => comparison.eval(facts),
        }
    }
}

impl Comparison {
    /// 在一个对象上求值。
    pub fn eval(&self, facts: &dyn Facts) -> Truth {
        let left = Side::of(facts.fact(&self.left));
        let right = || Side::operand(&self.right, facts);
        match self.op {
            Op::Eq => equal(&left, &right()),
            Op::Ne => !equal(&left, &right()),
            Op::Gt => order(&left, &right(), Ordering::is_gt),
            Op::Lt => order(&left, &right(), Ordering::is_lt),
            Op::Ge => order(&left, &right(), Ordering::is_ge),
            Op::Le => order(&left, &right(), Ordering::is_le),
            Op::In => is_in(&left, &self.right, facts),
            Op::NotIn => !is_in(&left, &self.right, facts),
            Op::Match => pattern_found(&left, &self.right, facts),
            Op::NotMatch => !pattern_found(&left, &self.right, facts),
        }
    }
}

/// 比较的一侧取到的东西。值都去掉了首尾空白。
enum Side {
    /// 写在表达式里的 `NULL` 或空串。
    Null,
    /// 路径取不到值，或取到的是空的。
    Missing,
    One(String),
    /// 一张有内容的表。
    Many,
}

impl Side {
    fn of(fact: Fact) -> Self {
        match fact {
            Fact::Value(value) => value
                .as_deref()
                .and_then(trimmed)
                .map_or(Side::Missing, Side::One),
            Fact::List(Some(list)) if list.items.iter().any(|item| !item.trim().is_empty()) => {
                Side::Many
            }
            Fact::List(_) => Side::Missing,
        }
    }

    /// 比较右边写的东西。
    fn operand(operand: &Operand, facts: &dyn Facts) -> Self {
        match operand {
            Operand::Value(Literal::Null) => Side::Null,
            Operand::Value(Literal::Text(text)) => trimmed(text).map_or(Side::Null, Side::One),
            Operand::Path(path) => Side::of(facts.fact(path)),
            Operand::List(_) | Operand::Pattern(_) => Side::Many,
        }
    }
}

/// 去掉首尾空白；剩下空的是 `None`。
fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// 按文法里数的写法读成数：可有一个负号，整数部分必有，小数点后必有数字。`1e3`、`+5`、
/// `.5`、`1.` 都不算数。
fn number(text: &str) -> Option<f64> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, "0"));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if digits(whole) && digits(fraction) {
        text.parse().ok()
    } else {
        None
    }
}

/// `=` 的比法：两边都读得成数就按数比，否则逐字比，区分大小写。两边都已去掉首尾空白。
fn same(a: &str, b: &str) -> bool {
    match (number(a), number(b)) {
        (Some(a), Some(b)) => a.partial_cmp(&b).is_some_and(Ordering::is_eq),
        _ => a == b,
    }
}

/// `=`：右边写的是 `NULL` 时看左边有没有值；否则两边都要有单个值，按 [`same`] 比。
fn equal(left: &Side, right: &Side) -> Truth {
    match (left, right) {
        (_, Side::Null) => Truth::from(matches!(left, Side::Missing)),
        (Side::One(a), Side::One(b)) => Truth::from(same(a, b)),
        _ => Truth::Unknown,
    }
}

/// `>`、`<`、`>=`、`<=`：两边都读得成数才比，否则不知道。
fn order(left: &Side, right: &Side, holds: fn(Ordering) -> bool) -> Truth {
    let (Side::One(a), Side::One(b)) = (left, right) else {
        return Truth::Unknown;
    };
    match (number(a), number(b)) {
        (Some(a), Some(b)) => a
            .partial_cmp(&b)
            .map_or(Truth::Unknown, |ordering| Truth::from(holds(ordering))),
        _ => Truth::Unknown,
    }
}

/// `IN`：左边与列表里某一项相等为真；都不相等为假；有说不准的项（取不到值的路径、取不出来
/// 的表）又没有相等的，为不知道。左边没有值也是不知道。
fn is_in(left: &Side, list: &Operand, facts: &dyn Facts) -> Truth {
    match left {
        Side::One(value) => member(value, list, facts),
        _ => Truth::Unknown,
    }
}

/// `value` 与列表的一项比；这一项可以又是列表。
fn member(value: &str, item: &Operand, facts: &dyn Facts) -> Truth {
    match item {
        Operand::List(items) => items.iter().fold(Truth::False, |truth, item| {
            truth.or(member(value, item, facts))
        }),
        Operand::Value(Literal::Null) => Truth::False,
        Operand::Value(Literal::Text(text)) => Truth::from(same(value, text.trim())),
        Operand::Path(path) => match facts.fact(path) {
            Fact::Value(found) => found
                .as_deref()
                .and_then(trimmed)
                .map_or(Truth::Unknown, |found| Truth::from(same(value, &found))),
            Fact::List(Some(list)) => Truth::from(
                list.items
                    .iter()
                    .any(|item| list.matching.agrees(value, item)),
            ),
            Fact::List(None) => Truth::Unknown,
        },
        Operand::Pattern(_) => Truth::Unknown,
    }
}

impl Matching {
    /// 左边的值与表里的一项是不是同一个。
    fn agrees(self, value: &str, item: &str) -> bool {
        match self {
            Matching::Equal => same(value, item.trim()),
            Matching::NominalDiameter => {
                parse_dn(value).is_some_and(|dn| parse_dn(item) == Some(dn))
            }
        }
    }
}

/// `MATCH`：正则在左边的值里找得到为真，不加锚点。左边没有值、正则取不到或写错了（写成
/// 路径的正则求值时才编译），都是不知道。
fn pattern_found(left: &Side, pattern: &Operand, facts: &dyn Facts) -> Truth {
    let Side::One(value) = left else {
        return Truth::Unknown;
    };
    match pattern {
        Operand::Pattern(pattern) => Truth::from(pattern.0.is_match(value)),
        operand => match Side::operand(operand, facts) {
            Side::One(text) => {
                Regex::new(&text).map_or(Truth::Unknown, |regex| Truth::from(regex.is_match(value)))
            }
            _ => Truth::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// 手写的事实：路径（各段用 `.` 连起来）→ 取到的东西；没写的路径没有值。
    #[derive(Default)]
    struct Sample(HashMap<String, Fact>);

    impl Sample {
        fn with(mut self, path: &str, value: &str) -> Self {
            self.0
                .insert(path.to_owned(), Fact::Value(Some(value.to_owned())));
            self
        }

        fn list(mut self, path: &str, items: &[&str], matching: Matching) -> Self {
            let items = items.iter().map(ToString::to_string).collect();
            self.0.insert(
                path.to_owned(),
                Fact::List(Some(FactList { items, matching })),
            );
            self
        }

        fn unavailable(mut self, path: &str) -> Self {
            self.0.insert(path.to_owned(), Fact::List(None));
            self
        }
    }

    impl Facts for Sample {
        fn fact(&self, path: &Path) -> Fact {
            self.0
                .get(&path.to_string())
                .cloned()
                .unwrap_or(Fact::Value(None))
        }
    }

    fn on(kind: Kind) -> Target {
        Target::Kind(kind)
    }

    fn tree(expr: &str) -> Expr {
        parse(expr, &on(Kind::Object)).unwrap_or_else(|err| panic!("{expr:?} should parse: {err}"))
    }

    fn eval(expr: &str, facts: &Sample) -> Truth {
        tree(expr).eval(facts)
    }

    fn failure(expr: &str, target: &Target) -> DslError {
        match parse(expr, target) {
            Ok(tree) => panic!("{expr:?} should not parse, got {tree:?}"),
            Err(err) => err,
        }
    }

    fn path(text: &str) -> Path {
        Path {
            segments: text.split('.').map(str::to_owned).collect(),
        }
    }

    fn text(value: &str) -> Operand {
        Operand::Value(Literal::Text(value.to_owned()))
    }

    fn at(text: &str) -> Operand {
        Operand::Path(path(text))
    }

    fn pattern(regex: &str) -> Operand {
        Operand::Pattern(Pattern(Regex::new(regex).expect("a valid regex")))
    }

    fn compare(left: &str, op: Op, right: Operand) -> Expr {
        Expr::Compare(Comparison {
            left: path(left),
            op,
            right,
        })
    }

    /// 文法的每个产生式各有例子，解析出来的树与手写的一样。
    #[test]
    fn every_production_of_the_grammar_parses_to_its_tree() {
        // comparison = path operator operand；value 的五种写法
        assert_eq!(
            tree("管道等级 = NULL"),
            compare("管道等级", Op::Eq, Operand::Value(Literal::Null))
        );
        assert_eq!(
            tree(r#"名称 = "离心泵 A""#),
            compare("名称", Op::Eq, text("离心泵 A"))
        );
        assert_eq!(tree("开度 > 100"), compare("开度", Op::Gt, text("100")));
        assert_eq!(tree("开度 < -0.5"), compare("开度", Op::Lt, text("-0.5")));
        assert_eq!(
            tree("流体介质 = #蒸汽"),
            compare("流体介质", Op::Eq, text("蒸汽"))
        );
        assert_eq!(
            tree("配对状态 = #one_sided"),
            compare("配对状态", Op::Eq, text("one_sided"))
        );
        assert_eq!(
            tree("位号合规 = FALSE"),
            compare("位号合规", Op::Eq, text("FALSE"))
        );
        assert_eq!(
            tree("流向相反 = TRUE"),
            compare("流向相反", Op::Eq, text("TRUE"))
        );
        // operator 的十个
        for (written, op) in [
            ("=", Op::Eq),
            ("!=", Op::Ne),
            (">", Op::Gt),
            ("<", Op::Lt),
            (">=", Op::Ge),
            ("<=", Op::Le),
        ] {
            assert_eq!(
                tree(&format!("开度 {written} 1")),
                compare("开度", op, text("1")),
                "{written}"
            );
        }
        assert_eq!(
            tree("种类 IN (#仪表, #阀门)"),
            compare(
                "种类",
                Op::In,
                Operand::List(vec![text("仪表"), text("阀门")])
            )
        );
        assert_eq!(
            tree("管道等级 NOT IN (项目.管道等级表)"),
            compare(
                "管道等级",
                Op::NotIn,
                Operand::List(vec![at("项目.管道等级表")])
            )
        );
        assert_eq!(
            tree(r#"位号 MATCH "^P-[0-9]{4}$""#),
            compare("位号", Op::Match, pattern("^P-[0-9]{4}$"))
        );
        assert_eq!(
            tree(r#"位号 NOT MATCH "-0[0-9]$""#),
            compare("位号", Op::NotMatch, pattern("-0[0-9]$"))
        );
        // operand：路径；列表里可以有路径、可以套列表；IN 后面不带括号的路径
        assert_eq!(
            tree("公称直径 != 所属管路.公称直径"),
            compare("公称直径", Op::Ne, at("所属管路.公称直径"))
        );
        assert_eq!(
            tree(r#"图层 IN ("0", (参数.允许图层, #PID))"#),
            compare(
                "图层",
                Op::In,
                Operand::List(vec![
                    text("0"),
                    Operand::List(vec![at("参数.允许图层"), text("PID")]),
                ])
            )
        );
        assert_eq!(
            tree("管道等级 IN 项目.管道等级表"),
            compare("管道等级", Op::In, at("项目.管道等级表"))
        );
        // path 与 identifier：多段；字母开头，带数字和下划线
        assert_eq!(
            tree("所属管线.管道等级.最大公称压力 >= 1.6"),
            compare("所属管线.管道等级.最大公称压力", Op::Ge, text("1.6"))
        );
        assert_eq!(tree("Tag_2 = x1"), compare("Tag_2", Op::Eq, at("x1")));
        // not_expr、and_expr、or_expr，和 atom 的括号
        assert_eq!(
            tree("NOT 开度 > 1"),
            Expr::Not(Box::new(compare("开度", Op::Gt, text("1"))))
        );
        assert_eq!(
            tree("a = 1 AND b = 2"),
            Expr::And(vec![
                compare("a", Op::Eq, text("1")),
                compare("b", Op::Eq, text("2")),
            ])
        );
        assert_eq!(
            tree("a = 1 OR b = 2 OR c = 3"),
            Expr::Or(vec![
                compare("a", Op::Eq, text("1")),
                compare("b", Op::Eq, text("2")),
                compare("c", Op::Eq, text("3")),
            ])
        );
        assert_eq!(tree("((a = 1))"), compare("a", Op::Eq, text("1")));
        // 记号之间的空白可有可无
        assert_eq!(tree("开度>=1"), compare("开度", Op::Ge, text("1")));
    }

    /// OR 最松，AND 次之，NOT 只管紧跟着的一项；括号改变归属。
    #[test]
    fn or_binds_loosest_then_and_then_not_and_parentheses_regroup() {
        let a = || compare("a", Op::Eq, text("1"));
        let b = || compare("b", Op::Eq, text("1"));
        let c = || compare("c", Op::Eq, text("1"));
        assert_eq!(
            tree("a = 1 OR b = 1 AND c = 1"),
            Expr::Or(vec![a(), Expr::And(vec![b(), c()])])
        );
        assert_eq!(
            tree("a = 1 AND b = 1 OR c = 1"),
            Expr::Or(vec![Expr::And(vec![a(), b()]), c()])
        );
        assert_eq!(
            tree("NOT a = 1 AND b = 1"),
            Expr::And(vec![Expr::Not(Box::new(a())), b()])
        );
        assert_eq!(
            tree("(a = 1 OR b = 1) AND c = 1"),
            Expr::And(vec![Expr::Or(vec![a(), b()]), c()])
        );
        assert_eq!(
            tree("NOT (a = 1 OR b = 1)"),
            Expr::Not(Box::new(Expr::Or(vec![a(), b()])))
        );
        // 求值也是这样归属的：a、b 为真，c 为假
        let facts = Sample::default()
            .with("a", "1")
            .with("b", "1")
            .with("c", "0");
        assert_eq!(eval("a = 1 OR b = 1 AND c = 1", &facts), Truth::True);
        assert_eq!(eval("(a = 1 OR b = 1) AND c = 1", &facts), Truth::False);
        assert_eq!(eval("NOT a = 1 OR b = 1", &facts), Truth::True);
        assert_eq!(eval("NOT (a = 1 OR b = 1)", &facts), Truth::False);
    }

    /// 三值逻辑的真值表逐格断言：直接算一遍，再经表达式求值一遍。
    #[test]
    fn the_three_valued_logic_tables_hold_cell_by_cell() {
        use Truth::{False as F, True as T, Unknown as U};
        let values = [T, F, U];
        // 行是左边，列是右边，都按 真、假、不知道 排
        let and = [[T, F, U], [F, F, F], [U, F, U]];
        let or = [[T, T, T], [T, F, U], [T, U, U]];
        let not = [F, T, U];
        // t = 1 为真，f = 1 为假，u 没有值，u = 1 不知道
        let facts = Sample::default().with("t", "1").with("f", "0");
        let name = |truth: Truth| match truth {
            T => "t",
            F => "f",
            U => "u",
        };
        for (i, left) in values.into_iter().enumerate() {
            for (j, right) in values.into_iter().enumerate() {
                let (l, r) = (name(left), name(right));
                assert_eq!(left.and(right), and[i][j], "{l} AND {r}");
                assert_eq!(left.or(right), or[i][j], "{l} OR {r}");
                let both = format!("{l} = 1 AND {r} = 1");
                assert_eq!(eval(&both, &facts), and[i][j], "{both}");
                let either = format!("{l} = 1 OR {r} = 1");
                assert_eq!(eval(&either, &facts), or[i][j], "{either}");
            }
            assert_eq!(!left, not[i], "NOT {}", name(left));
            let negated = format!("NOT {} = 1", name(left));
            assert_eq!(eval(&negated, &facts), not[i], "{negated}");
        }
        assert!(T.is_true());
        assert!(!F.is_true());
        assert!(!U.is_true());
    }

    /// 属性没有、为空串、只有空白，都是 NULL；`""` 等同 NULL。
    #[test]
    fn null_and_the_empty_string_are_the_same() {
        let facts = Sample::default()
            .with("填了", "A")
            .with("空串", "")
            .with("空白", "  ");
        for name in ["没有", "空串", "空白"] {
            assert_eq!(
                eval(&format!("{name} = NULL"), &facts),
                Truth::True,
                "{name}"
            );
            assert_eq!(
                eval(&format!(r#"{name} = """#), &facts),
                Truth::True,
                "{name}"
            );
            assert_eq!(
                eval(&format!("{name} != NULL"), &facts),
                Truth::False,
                "{name}"
            );
            assert_eq!(
                eval(&format!(r#"{name} != """#), &facts),
                Truth::False,
                "{name}"
            );
            // 和一个具体的值比，没有值就不知道
            assert_eq!(
                eval(&format!(r#"{name} = "A""#), &facts),
                Truth::Unknown,
                "{name}"
            );
            assert_eq!(
                eval(&format!(r#"{name} != "A""#), &facts),
                Truth::Unknown,
                "{name}"
            );
            assert_eq!(
                eval(&format!("{name} > 1"), &facts),
                Truth::Unknown,
                "{name}"
            );
        }
        assert_eq!(eval("填了 = NULL", &facts), Truth::False);
        assert_eq!(eval(r#"填了 = """#, &facts), Truth::False);
        assert_eq!(eval("填了 != NULL", &facts), Truth::True);
        // 右边是取不到值的路径，不是字面的 NULL：不知道
        assert_eq!(eval("填了 = 没有", &facts), Truth::Unknown);
        assert_eq!(eval("填了 != 没有", &facts), Truth::Unknown);
    }

    /// `=`、`!=` 两边都是数就按数比，否则逐字比；`>` 这些只按数比。
    #[test]
    fn numbers_compare_as_numbers_and_everything_else_as_text() {
        let facts = Sample::default()
            .with("设计温度", "260")
            .with("压力", " 1.60 ")
            .with("编号", "01001")
            .with("位号", "P-1001")
            .with("规格", "1e3")
            .with("名称", "Abc")
            .with("差值", "-5");
        for expr in [
            "设计温度 > 200",
            "设计温度 >= 260",
            "设计温度 <= 260",
            "设计温度 < 300",
            "设计温度 = 260.0",
            "设计温度 != 261",
            "压力 = 1.6",
            "编号 = 1001",
            "差值 < 0",
            "位号 != 1001",
            r#"位号 = "P-1001""#,
            r#"名称 = " Abc ""#,
            r#"规格 = "1e3""#,
        ] {
            assert_eq!(eval(expr, &facts), Truth::True, "{expr}");
        }
        for expr in [
            "设计温度 > 260",
            "设计温度 < 260",
            "位号 = 1001",
            r#"名称 = "abc""#,
            "规格 = 1000",
        ] {
            assert_eq!(eval(expr, &facts), Truth::False, "{expr}");
        }
        for expr in [
            "位号 > 1000",
            "规格 > 1",
            r#"设计温度 > "高""#,
            "设计温度 > NULL",
            "没有 > 1",
            "设计温度 >= 没有",
        ] {
            assert_eq!(eval(expr, &facts), Truth::Unknown, "{expr}");
        }
    }

    /// `IN` 的列表可以写值、路径和表；左边没有值、表取不出来都是不知道。
    #[test]
    fn in_lists_take_values_paths_and_tables() {
        let facts = Sample::default()
            .with("流体介质", "氯气")
            .with("管道等级", "A1A")
            .with("公称直径", "DN50")
            .with("图层", "PID")
            .with("所属管线.管道等级", "B1B")
            .list("项目.管道等级表", &["A1A", "B1B"], Matching::Equal)
            .list("项目.公称直径表", &["50", "80"], Matching::NominalDiameter)
            .list("参数.允许图层", &["0", "PID"], Matching::Equal)
            .unavailable("项目.流体介质表");
        for expr in [
            "流体介质 IN (#氯气, #氢氟酸)",
            "管道等级 IN (项目.管道等级表)",
            "管道等级 IN 项目.管道等级表",
            "公称直径 IN (项目.公称直径表)",
            "图层 IN (参数.允许图层)",
            r#"图层 IN ("0", ("X", 参数.允许图层))"#,
            r#"管道等级 IN ("X", 所属管线.管道等级, "A1A")"#,
            // 列表里有取不到值的路径，别的项等上了仍是真
            r#"管道等级 IN (所属管路.管道等级, "A1A")"#,
            "流体介质 NOT IN (#蒸汽, NULL)",
        ] {
            assert_eq!(eval(expr, &facts), Truth::True, "{expr}");
        }
        for expr in [
            "流体介质 IN (#蒸汽)",
            "管道等级 NOT IN (项目.管道等级表)",
            "管道等级 IN (所属管线.管道等级)",
            // 公称直径只在跟主数据表比时才把 DN50 当 50
            "公称直径 = 50",
            "公称直径 IN (50, 80)",
        ] {
            assert_eq!(eval(expr, &facts), Truth::False, "{expr}");
        }
        for expr in [
            "流体介质 IN (项目.流体介质表)",
            "流体介质 NOT IN (项目.流体介质表)",
            "没有 IN (#氯气)",
            "没有 NOT IN (#氯气)",
            "管道等级 IN (所属管路.管道等级)",
            r#"管道等级 NOT IN ("X", 所属管路.管道等级)"#,
        ] {
            assert_eq!(eval(expr, &facts), Truth::Unknown, "{expr}");
        }
        for (value, hit) in [("DN80", true), ("dn 80", true), ("80", true), ("65", false)] {
            let facts = Sample::default().with("公称直径", value).list(
                "项目.公称直径表",
                &["50", "80"],
                Matching::NominalDiameter,
            );
            let expr = "公称直径 IN (项目.公称直径表)";
            assert_eq!(eval(expr, &facts), Truth::from(hit), "{value}");
        }
    }

    /// `MATCH` 用 Rust `regex` 的写法，不自动加首尾锚点。
    #[test]
    fn match_uses_the_regex_crate_without_anchors() {
        let facts = Sample::default()
            .with("位号", "L-1001-01")
            .with("名称", "进料泵")
            .with("参数.格式", "^L-")
            .with("参数.坏的", "(");
        for expr in [
            r#"位号 MATCH "1001""#,
            r#"位号 MATCH "^L-[0-9]{4}""#,
            r#"名称 MATCH "泵$""#,
            r#"位号 NOT MATCH "^[A-Z]{1,2}-[0-9]{4}$""#,
            "位号 MATCH 参数.格式",
        ] {
            assert_eq!(eval(expr, &facts), Truth::True, "{expr}");
        }
        for expr in [
            r#"位号 MATCH "^[A-Z]{1,2}-[0-9]{4}$""#,
            r#"位号 NOT MATCH "1001""#,
            r#"位号 MATCH "l-1001""#,
        ] {
            assert_eq!(eval(expr, &facts), Truth::False, "{expr}");
        }
        for expr in [
            r#"没有 MATCH ".""#,
            r#"没有 NOT MATCH ".""#,
            "位号 MATCH 参数.坏的",
            "位号 MATCH 参数.没有",
            "位号 MATCH NULL",
        ] {
            assert_eq!(eval(expr, &facts), Truth::Unknown, "{expr}");
        }
    }

    /// §3.3 与 §6 第 3 条的例子：解析成功的、报错及报在哪里的，和 §3.3 的自定义规则。
    #[test]
    fn the_spec_examples_parse_or_fail_where_the_spec_says() {
        let line = on(Kind::PipeLine);
        assert_eq!(
            parse("管道等级 = NULL", &line),
            Ok(compare("管道等级", Op::Eq, Operand::Value(Literal::Null)))
        );
        let err = failure("管道等级 ==", &line);
        assert_eq!(err.code(), "DSL_PARSE_ERROR");
        assert_eq!(err.offset, 6, "the second '='");
        assert_eq!(err.expected, "值或路径");
        for target in [
            on(Kind::Object),
            on(Kind::Equipment),
            on(Kind::Valve),
            Target::TypeCode("PUMP".to_owned()),
        ] {
            let err = failure("所属泵.位号", &target);
            assert_eq!(err.offset, 0, "points at 所属泵 on {target:?}");
            assert!(err.expected.contains("所属管线"), "{}", err.expected);
        }
        assert_eq!(failure("位号 = 所属泵.位号", &on(Kind::Object)).offset, 5);
        let custom = "公称直径 < 管道等级.公称直径下限 OR 公称直径 > 管道等级.公称直径上限";
        assert!(parse(custom, &line).is_ok());
    }

    /// 解析不过时报出错处的字符偏移和这里本该出现什么。
    #[test]
    fn parse_errors_say_where_and_what_was_expected() {
        let object = on(Kind::Object);
        for (expr, offset, expected) in [
            ("", 0, START),
            ("   ", 3, START),
            ("开度", 2, OPERATOR),
            ("开度 =", 4, OPERAND),
            ("开度 = 1 位号", 7, END),
            ("(开度 = 1", 7, CLOSE),
            ("开度 = 1)", 6, END),
            ("NOT NOT 开度 = 1", 4, AFTER_NOT),
            ("开度 NOT = 1", 7, AFTER_NOT_OP),
            ("开度 IN ()", 7, OPERAND),
            ("开度 IN (1,)", 9, OPERAND),
            ("开度 IN (1 2)", 9, LIST_NEXT),
            ("开度 = (1, 2)", 5, SINGLE),
            ("开度 = \"1", 7, QUOTE),
            ("开度 = -a", 5, OPERAND),
            ("开度 = #1", 5, OPERAND),
            ("开度 & 1", 3, OPERATOR),
            ("开度 ! 1", 3, OPERATOR),
            ("1 = 开度", 0, START),
            ("开度 = 1 AND", 10, START),
            ("开度 = 1 and 位号 = 2", 7, END),
            ("参数. = 1", 4, NAME),
            ("位号 MATCH \"(\"", 9, REGEX),
        ] {
            let err = failure(expr, &object);
            assert_eq!(
                (err.offset, err.expected.as_str()),
                (offset, expected),
                "{expr:?}"
            );
            assert_eq!(err.code(), DSL_PARSE_ERROR);
        }
    }

    /// 多段路径的第一段要是作用对象认识的关系或根（§3.4），关系后面能接什么也按表来。
    #[test]
    fn relation_names_depend_on_the_target() {
        let parses = |expr: &str, kind: Kind| parse(expr, &on(kind)).is_ok();
        // 各种类自己的关系
        assert!(parses(r#"所属设备.位号 = "P-1001""#, Kind::Nozzle));
        assert!(parses("位号前缀 != 所属设备.位号", Kind::Nozzle));
        assert!(parses(
            "对端线.管道等级 != 所在线.管道等级",
            Kind::Connector
        ));
        assert!(parses("符号.状态 != #有效", Kind::Symbol));
        assert!(parses("所属管路.公称直径 = 50", Kind::Segment));
        // 关系可以连用，到了别的对象就按那种对象认
        assert!(parses("所属管线.管道等级.最大公称压力 > 1", Kind::Valve));
        assert!(parses(
            r#"所属管路.所属管线.位号 = "L-1001""#,
            Kind::Segment
        ));
        // 根哪种对象都能用
        for kind in Kind::ALL {
            let roots = r#"项目.图例标准 = NULL AND 图纸.PBS = "A" AND 参数.上限 > 1"#;
            assert!(parses(roots, kind), "{kind:?}");
        }
        // 别的种类有、这种没有的关系；根只能开头
        for (expr, kind, offset) in [
            ("所属设备.位号 = 1", Kind::PipeLine, 0),
            ("对端线.管道等级 = 1", Kind::Equipment, 0),
            ("符号.状态 = #缺失", Kind::Segment, 0),
            ("位号.长度 > 32", Kind::Connector, 0),
            ("所属管线.所属设备.位号 = 1", Kind::Valve, 5),
            ("所属管线.项目.管道等级表 = 1", Kind::Valve, 5),
        ] {
            assert_eq!(
                failure(expr, &on(kind)).offset,
                offset,
                "{expr:?} on {kind:?}"
            );
        }
        // 这几个关系后面只能接一段，有的还只能是表里的名字
        for (expr, offset, expected) in [
            (
                "项目.管道等级 = 1",
                3,
                "项目 后面的名字：管道等级表、公称直径表、绝热等级表、流体介质表、图例标准",
            ),
            ("图纸.名称 = 1", 3, "图纸 后面的名字：图号、PBS"),
            (
                "位号.宽度 > 1",
                3,
                "位号 后面的名字：长度、同类型重复数、跨图重复数",
            ),
            ("位号.长度.x > 1", 6, "运算符（位号.长度 不能再往下取）"),
            ("参数.a.b = 1", 5, "运算符（参数.a 不能再往下取）"),
            (
                "管道等级.材质.x = 1",
                8,
                "运算符（管道等级.材质 不能再往下取）",
            ),
        ] {
            let err = failure(expr, &on(Kind::Object));
            assert_eq!(
                (err.offset, err.expected.as_str()),
                (offset, expected),
                "{expr:?}"
            );
        }
        // 类型编码按符号实例认
        let pump = Target::TypeCode("PUMP".to_owned());
        assert!(parse("位号.长度 > 32 AND 符号.状态 = #缺失", &pump).is_ok());
        assert_eq!(failure("所属设备.位号 = 1", &pump).offset, 0);
    }

    /// §3.5 的 41 条内置规则，表达式在各自的作用对象上都能解析；需求的例子求值命中。
    #[test]
    fn every_builtin_expression_parses_on_its_target() {
        let rules = [
            ("DR-001", "管线", "管道等级 = NULL"),
            ("DR-002", "管线", "公称直径 = NULL"),
            ("DR-003", "设备", r#"位号 = NULL OR 位号 = """#),
            ("DR-004", "对象实例", "位号.同类型重复数 > 1"),
            ("DR-005", "仪表", "仪表回路 = NULL"),
            ("DR-006", "页间连接符", "对端图纸 = NULL OR 对端位号 = NULL"),
            (
                "DR-007",
                "阀门",
                "压力等级 > 所属管线.管道等级.最大公称压力",
            ),
            ("DR-008", "管口", "连接状态 = #未连接"),
            ("DR-009", "图纸", "图例标准 = NULL"),
            ("DR-010", "对象实例", "必填项缺失数 > 0"),
            ("DR-011", "管路", r#"后缀 = NULL OR 后缀 = """#),
            ("DR-012", "管段", "长度 <= 0"),
            ("DR-013", "仪表回路", "仪表数 < 2"),
            ("DR-014", "管线", "端口未连接数 > 0"),
            ("DR-015", "信号线", "起点仪表 = NULL OR 终点仪表 = NULL"),
            ("DR-016", "标注", r#"文字 = NULL OR 文字 = """#),
            ("DR-017", "阀门", "开度 > 100 OR 开度 < 0"),
            ("DR-018", "对象实例", "位号合规 = FALSE"),
            ("DR-019", "对象实例", "位号.长度 > 32"),
            ("DR-020", "管口", "位号前缀 != 所属设备.位号"),
            (
                "DR-021",
                "管线",
                "流体介质 IN (#氯气, #氢氟酸) AND 管道等级.材质 = #碳钢",
            ),
            ("DR-022", "管线", "管道等级 NOT IN (项目.管道等级表)"),
            ("DR-023", "设备", "管口数 < 2 AND NOT (设备类别 = #储罐)"),
            ("DR-024", "符号实例", "符号.状态 != #有效"),
            (
                "DR-025",
                "管线",
                "(设计温度 > 200 OR 流体介质 = #蒸汽) AND 绝热等级 = NULL",
            ),
            (
                "DR-101",
                "对象实例",
                r#"种类 IN (#仪表, #阀门) AND (位号 = NULL OR 位号 = "")"#,
            ),
            ("DR-102", "对象实例", "位号.跨图重复数 > 0"),
            (
                "DR-103",
                "对象实例",
                "种类 IN (#管线, #管路) AND 公称直径 NOT IN (项目.公称直径表)",
            ),
            ("DR-104", "管线", "流体介质 NOT IN (项目.流体介质表)"),
            ("DR-105", "管路", "直连异径管路数 > 0"),
            ("DR-106", "阀门", "公称直径 != 所属管路.公称直径"),
            (
                "DR-107",
                "对象实例",
                "种类 IN (#阀门, #管件) AND 管道等级 != 所属管线.管道等级",
            ),
            ("DR-108", "信号线", "接到设备管口数 > 0"),
            ("DR-109", "阀门", "流向相反 = TRUE"),
            ("DR-110", "管段", "重叠管段数 > 0"),
            (
                "DR-111",
                "对象实例",
                "种类 IN (#阀门, #管件) AND 未连接端口数 > 0",
            ),
            (
                "DR-112",
                "页间连接符",
                "对端线.管道等级 != 所在线.管道等级 OR 对端线.公称直径 != 所在线.公称直径",
            ),
            ("DR-113", "仪表回路", "控制器数 = 0 OR 测量元件数 = 0"),
            ("DR-114", "对象实例", "图层 NOT IN (参数.允许图层)"),
            ("DR-115", "管路", "未标记覆盖数 > 0"),
            (
                "DR-116",
                "页间连接符",
                "配对状态 IN (#one_sided, #target_missing)",
            ),
        ];
        assert_eq!(rules.len(), 41);
        for (id, target, expr) in rules {
            let target = Target::parse(target);
            assert!(
                matches!(target, Target::Kind(_)),
                "{id}: {target:?} is a kind"
            );
            if let Err(err) = parse(expr, &target) {
                panic!("{id} {expr:?}: {err}");
            }
        }
        // 需求的例子（§6 第 4 条）：设计温度 260、流体介质 工艺水、没有绝热等级 → DR-025 命中
        let (id, target, expr) = rules[24];
        assert_eq!(id, "DR-025");
        let dr025 = parse(expr, &Target::parse(target)).expect("DR-025 parses");
        let line = Sample::default()
            .with("设计温度", "260")
            .with("流体介质", "工艺水");
        assert_eq!(dr025.eval(&line), Truth::True);
        assert_eq!(dr025.eval(&line.with("绝热等级", "H")), Truth::False);
    }
}
