//! The database client demo: browse a fake database and query it with SQL.
//!
//! There are four hardcoded tables (customers, products, orders, employees)
//! and a small read-only SQL engine: `SELECT` with expressions, `JOIN` /
//! `LEFT JOIN`, `WHERE`, `GROUP BY` with aggregates, `HAVING`, `ORDER BY`,
//! `LIMIT`/`OFFSET`, plus `SHOW TABLES` and `DESCRIBE`. Results are drawn as
//! aligned text and paged; past queries can be recalled.

use super::common::menu;
use hypereffect::{cel, json, ui, Action, App, Handler, Map, Outbox, Value};
use std::cmp::Ordering;
use std::collections::HashMap;

const PAGE: usize = 10;
const MAX_ROWS: usize = 1000;
const MAX_WIDTH: usize = 24;
const HISTORY: usize = 6;

// --- the data ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
}

impl Cell {
    fn text(s: &str) -> Cell {
        Cell::Text(s.to_owned())
    }

    fn show(&self) -> String {
        match self {
            Cell::Null => "NULL".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Float(f) => format!("{f:.2}"),
            Cell::Text(s) => s.clone(),
            Cell::Bool(b) => b.to_string().to_uppercase(),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(*i as f64),
            Cell::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn truthy(&self) -> bool {
        matches!(self, Cell::Bool(true))
    }
}

struct Table {
    name: &'static str,
    /// Column names and SQL types.
    columns: &'static [(&'static str, &'static str)],
    rows: Vec<Vec<Cell>>,
}

/// A small deterministic generator, so the data is the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % below
    }
}

fn database() -> Vec<Table> {
    let mut rng = Lcg(42);
    let first = [
        "Ada", "Brian", "Chen", "Dalia", "Emeka", "Farah", "Goran", "Hana", "Ivan", "Jules",
        "Kira", "Liam", "Mira", "Noor", "Omar",
    ];
    let last = ["Okafor", "Silva", "Novak", "Haddad", "Lindqvist", "Tanaka"];
    let countries = ["Brazil", "Canada", "Germany", "Japan", "Nigeria", "Sweden"];
    let plans = ["free", "pro", "team", "enterprise"];
    let customers: Vec<Vec<Cell>> = (1..=30)
        .map(|id| {
            let name = format!(
                "{} {}",
                first[(id as usize * 7) % first.len()],
                last[rng.next(last.len() as u64) as usize]
            );
            let referrer = if id > 6 && rng.next(3) == 0 {
                Cell::Int(rng.next(id as u64 - 1) as i64 + 1)
            } else {
                Cell::Null
            };
            vec![
                Cell::Int(id),
                Cell::Text(name),
                Cell::text(countries[rng.next(6) as usize]),
                Cell::text(plans[rng.next(4) as usize]),
                Cell::Text(format!(
                    "2025-{:02}-{:02}",
                    rng.next(12) + 1,
                    rng.next(28) + 1
                )),
                referrer,
            ]
        })
        .collect();
    let catalog = [
        ("Desk lamp", "home", 34.90, 120),
        ("Standing desk", "furniture", 429.00, 18),
        ("Office chair", "furniture", 259.50, 32),
        ("USB-C hub", "electronics", 49.99, 210),
        ("Mechanical keyboard", "electronics", 119.00, 64),
        ("Monitor 27in", "electronics", 289.00, 22),
        ("Notebook (3-pack)", "stationery", 12.40, 400),
        ("Fountain pen", "stationery", 38.00, 75),
        ("Desk mat", "home", 24.00, 140),
        ("Webcam", "electronics", 79.90, 48),
        ("Headphones", "electronics", 199.00, 9),
        ("Plant pot", "home", 15.50, 300),
    ];
    let products: Vec<Vec<Cell>> = catalog
        .iter()
        .enumerate()
        .map(|(i, (name, cat, price, stock))| {
            vec![
                Cell::Int(i as i64 + 1),
                Cell::text(name),
                Cell::text(cat),
                Cell::Float(*price),
                Cell::Int(*stock),
            ]
        })
        .collect();
    let statuses = [
        "shipped",
        "shipped",
        "shipped",
        "pending",
        "cancelled",
        "refunded",
    ];
    let orders: Vec<Vec<Cell>> = (1..=80)
        .map(|id| {
            let product = rng.next(12) as usize;
            let qty = rng.next(5) as i64 + 1;
            vec![
                Cell::Int(id),
                Cell::Int(rng.next(26) as i64 + 1),
                Cell::Int(product as i64 + 1),
                Cell::Int(qty),
                Cell::Float(catalog[product].2 * qty as f64),
                Cell::text(statuses[rng.next(6) as usize]),
                Cell::Text(format!(
                    "2026-{:02}-{:02}",
                    rng.next(9) + 1,
                    rng.next(28) + 1
                )),
            ]
        })
        .collect();
    let staff = [
        ("Priya Raman", "management", 185000, None, "2019-02-01"),
        ("Tomás Ruiz", "engineering", 142000, Some(1), "2020-06-15"),
        ("Lena Fischer", "engineering", 128000, Some(2), "2021-03-08"),
        ("Kofi Mensah", "engineering", 118000, Some(2), "2022-09-19"),
        ("Yuki Mori", "engineering", 99000, Some(3), "2024-01-22"),
        ("Sara Lund", "sales", 121000, Some(1), "2020-11-02"),
        ("Diego Alves", "sales", 87000, Some(6), "2023-04-17"),
        ("Aisha Khan", "sales", 83000, Some(6), "2024-07-01"),
        ("Marek Zielinski", "support", 72000, Some(1), "2021-05-24"),
        ("Hye-jin Park", "support", 64000, Some(9), "2023-10-09"),
        ("Oscar Webb", "support", 61000, Some(9), "2025-02-11"),
        ("Nadia Petrova", "finance", 109000, Some(1), "2022-01-31"),
        ("Samuel Adeyemi", "finance", 76000, Some(12), "2024-03-25"),
        ("Clara Jones", "engineering", 135000, Some(2), "2021-08-30"),
    ];
    let employees: Vec<Vec<Cell>> = staff
        .iter()
        .enumerate()
        .map(|(i, (name, dept, salary, boss, hired))| {
            vec![
                Cell::Int(i as i64 + 1),
                Cell::text(name),
                Cell::text(dept),
                Cell::Int(*salary),
                boss.map_or(Cell::Null, Cell::Int),
                Cell::text(hired),
            ]
        })
        .collect();
    vec![
        Table {
            name: "customers",
            columns: &[
                ("id", "INTEGER"),
                ("name", "TEXT"),
                ("country", "TEXT"),
                ("plan", "TEXT"),
                ("signup", "DATE"),
                ("referrer_id", "INTEGER NULL"),
            ],
            rows: customers,
        },
        Table {
            name: "products",
            columns: &[
                ("id", "INTEGER"),
                ("name", "TEXT"),
                ("category", "TEXT"),
                ("price", "REAL"),
                ("stock", "INTEGER"),
            ],
            rows: products,
        },
        Table {
            name: "orders",
            columns: &[
                ("id", "INTEGER"),
                ("customer_id", "INTEGER"),
                ("product_id", "INTEGER"),
                ("qty", "INTEGER"),
                ("total", "REAL"),
                ("status", "TEXT"),
                ("placed", "DATE"),
            ],
            rows: orders,
        },
        Table {
            name: "employees",
            columns: &[
                ("id", "INTEGER"),
                ("name", "TEXT"),
                ("department", "TEXT"),
                ("salary", "INTEGER"),
                ("manager_id", "INTEGER NULL"),
                ("hired", "DATE"),
            ],
            rows: employees,
        },
    ]
}

// --- tokens ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(String),
    Str(String),
    Word(String),
    Sym(&'static str),
}

fn tokenize(src: &str) -> Result<Vec<(Tok, usize, usize)>, String> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        let end_of = |j: usize| chars.get(j).map_or(src.len(), |(b, _)| *b);
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].1.is_ascii_digit() || chars[j].1 == '.') {
                j += 1;
            }
            toks.push((Tok::Num(src[at..end_of(j)].to_owned()), at, end_of(j)));
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].1.is_alphanumeric() || chars[j].1 == '_') {
                j += 1;
            }
            toks.push((Tok::Word(src[at..end_of(j)].to_owned()), at, end_of(j)));
            i = j;
        } else if c == '\'' {
            let mut j = i + 1;
            let mut s = String::new();
            loop {
                match chars.get(j) {
                    None => {
                        return Err(format!(
                            "Unterminated string starting at position {}.",
                            at + 1
                        ))
                    }
                    Some((_, '\'')) if chars.get(j + 1).is_some_and(|(_, q)| *q == '\'') => {
                        s.push('\'');
                        j += 2;
                    }
                    Some((_, '\'')) => break,
                    Some((_, ch)) => {
                        s.push(*ch);
                        j += 1;
                    }
                }
            }
            toks.push((Tok::Str(s), at, end_of(j + 1)));
            i = j + 1;
        } else {
            let two = src[at..].get(..2).unwrap_or("");
            let sym = match two {
                "<=" => Some("<="),
                ">=" => Some(">="),
                "<>" | "!=" => Some("!="),
                _ => None,
            };
            if let Some(sym) = sym {
                toks.push((Tok::Sym(sym), at, at + 2));
                i += 2;
                continue;
            }
            let sym = match c {
                '=' => "=",
                '<' => "<",
                '>' => ">",
                '(' => "(",
                ')' => ")",
                ',' => ",",
                '*' => "*",
                '+' => "+",
                '-' => "-",
                '/' => "/",
                '.' => ".",
                ';' => ";",
                _ => {
                    return Err(format!(
                        "Unexpected character '{c}' at position {}.",
                        at + 1
                    ))
                }
            };
            toks.push((Tok::Sym(sym), at, at + c.len_utf8()));
            i += 1;
        }
    }
    Ok(toks)
}

// --- the syntax tree ---------------------------------------------------------

#[derive(Clone, Debug, Copy, PartialEq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug)]
enum Expr {
    Lit(Cell),
    Col(Option<String>, String),
    Call(String, Vec<Expr>, bool),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Like(Box<Expr>, Box<Expr>, bool),
    In(Box<Expr>, Vec<Expr>, bool),
    Between(Box<Expr>, Box<Expr>, Box<Expr>, bool),
    IsNull(Box<Expr>, bool),
}

const AGGREGATES: [&str; 5] = ["COUNT", "SUM", "AVG", "MIN", "MAX"];

impl Expr {
    fn has_aggregate(&self) -> bool {
        match self {
            Expr::Call(n, args, _) => {
                AGGREGATES.contains(&n.as_str()) || args.iter().any(Expr::has_aggregate)
            }
            Expr::Lit(_) | Expr::Col(..) => false,
            Expr::Neg(e) | Expr::Not(e) | Expr::IsNull(e, _) => e.has_aggregate(),
            Expr::Bin(_, a, b) | Expr::Like(a, b, _) => a.has_aggregate() || b.has_aggregate(),
            Expr::In(e, list, _) => e.has_aggregate() || list.iter().any(Expr::has_aggregate),
            Expr::Between(a, b, c, _) => {
                a.has_aggregate() || b.has_aggregate() || c.has_aggregate()
            }
        }
    }
}

struct Item {
    /// `None` for a star, which expands to columns, optionally of one table.
    expr: Option<Expr>,
    star_of: Option<String>,
    name: String,
}

struct TableRef {
    name: String,
    alias: String,
}

struct Join {
    left: bool,
    table: TableRef,
    on: Expr,
}

struct Select {
    distinct: bool,
    items: Vec<Item>,
    from: TableRef,
    joins: Vec<Join>,
    filter: Option<Expr>,
    group: Vec<Expr>,
    having: Option<Expr>,
    order: Vec<(Expr, bool)>,
    limit: Option<usize>,
    offset: usize,
}

const RESERVED: [&str; 16] = [
    "FROM", "WHERE", "GROUP", "ORDER", "LIMIT", "OFFSET", "JOIN", "INNER", "LEFT", "ON", "HAVING",
    "AS", "AND", "OR", "NOT", "OUTER",
];

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize, usize)>,
    pos: usize,
}

type Parse<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn near(&self) -> String {
        match self.toks.get(self.pos) {
            Some((_, a, b)) => format!("'{}'", &self.src[*a..*b]),
            None => "the end of the query".into(),
        }
    }

    fn fail<T>(&self, expected: &str) -> Parse<T> {
        Err(format!(
            "Syntax error near {}: expected {expected}.",
            self.near()
        ))
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        let yes = self.is_kw(kw);
        if yes {
            self.pos += 1;
        }
        yes
    }

    fn expect_kw(&mut self, kw: &str) -> Parse<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            self.fail(kw)
        }
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        let yes = matches!(self.peek(), Some(Tok::Sym(x)) if *x == s);
        if yes {
            self.pos += 1;
        }
        yes
    }

    fn expect_sym(&mut self, s: &str) -> Parse<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            self.fail(&format!("'{s}'"))
        }
    }

    fn ident(&mut self, what: &str) -> Parse<String> {
        match self.peek() {
            Some(Tok::Word(w)) if !RESERVED.contains(&w.to_uppercase().as_str()) => {
                let w = w.clone();
                self.pos += 1;
                Ok(w)
            }
            _ => self.fail(what),
        }
    }

    fn alias(&mut self) -> Parse<Option<String>> {
        if self.eat_kw("AS") {
            return self.ident("an alias").map(Some);
        }
        match self.peek() {
            Some(Tok::Word(w)) if !RESERVED.contains(&w.to_uppercase().as_str()) => {
                let w = w.clone();
                self.pos += 1;
                Ok(Some(w))
            }
            _ => Ok(None),
        }
    }

    fn table_ref(&mut self) -> Parse<TableRef> {
        let name = self.ident("a table name")?.to_lowercase();
        let alias = self
            .alias()?
            .map_or_else(|| name.clone(), |a| a.to_lowercase());
        Ok(TableRef { name, alias })
    }

    fn select(&mut self) -> Parse<Select> {
        self.expect_kw("SELECT")?;
        let distinct = self.eat_kw("DISTINCT");
        let mut items = Vec::new();
        loop {
            let start = self.toks.get(self.pos).map_or(self.src.len(), |t| t.1);
            if self.eat_sym("*") {
                items.push(Item {
                    expr: None,
                    star_of: None,
                    name: "*".into(),
                });
            } else if matches!(
                self.toks.get(self.pos..self.pos + 3),
                Some([(Tok::Word(_), ..), (Tok::Sym("."), ..), (Tok::Sym("*"), ..)])
            ) {
                let Some(Tok::Word(t)) = self.peek().cloned() else {
                    unreachable!()
                };
                self.pos += 3;
                items.push(Item {
                    expr: None,
                    star_of: Some(t.to_lowercase()),
                    name: "*".into(),
                });
            } else {
                let expr = self.expr()?;
                let end = self.toks[self.pos - 1].2;
                let name = match self.alias()? {
                    Some(a) => a,
                    None => match &expr {
                        Expr::Col(_, c) => c.clone(),
                        _ => self.src[start..end].to_owned(),
                    },
                };
                items.push(Item {
                    expr: Some(expr),
                    star_of: None,
                    name,
                });
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_kw("FROM")?;
        let from = self.table_ref()?;
        let mut joins = Vec::new();
        loop {
            let left = if self.eat_kw("LEFT") {
                self.eat_kw("OUTER");
                true
            } else {
                self.eat_kw("INNER");
                false
            };
            if !self.eat_kw("JOIN") {
                if left {
                    return self.fail("JOIN");
                }
                break;
            }
            let table = self.table_ref()?;
            self.expect_kw("ON")?;
            joins.push(Join {
                left,
                table,
                on: self.expr()?,
            });
        }
        let filter = if self.eat_kw("WHERE") {
            Some(self.expr()?)
        } else {
            None
        };
        let mut group = Vec::new();
        if self.eat_kw("GROUP") {
            self.expect_kw("BY")?;
            loop {
                group.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let having = if self.eat_kw("HAVING") {
            Some(self.expr()?)
        } else {
            None
        };
        let mut order = Vec::new();
        if self.eat_kw("ORDER") {
            self.expect_kw("BY")?;
            loop {
                let e = self.expr()?;
                let desc = if self.eat_kw("DESC") {
                    true
                } else {
                    self.eat_kw("ASC");
                    false
                };
                order.push((e, desc));
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let count = |p: &mut Parser, what: &str| -> Parse<usize> {
            match p.peek().cloned() {
                Some(Tok::Num(n)) => {
                    p.pos += 1;
                    n.parse()
                        .map_err(|_| format!("{what} must be a whole number."))
                }
                _ => p.fail("a number"),
            }
        };
        let mut limit = None;
        let mut offset = 0;
        if self.eat_kw("LIMIT") {
            limit = Some(count(self, "LIMIT")?);
        }
        if self.eat_kw("OFFSET") {
            offset = count(self, "OFFSET")?;
        }
        self.eat_sym(";");
        if self.pos < self.toks.len() {
            return self.fail("the end of the query");
        }
        Ok(Select {
            distinct,
            items,
            from,
            joins,
            filter,
            group,
            having,
            order,
            limit,
            offset,
        })
    }

    fn expr(&mut self) -> Parse<Expr> {
        let mut left = self.and()?;
        while self.eat_kw("OR") {
            left = Expr::Bin(Op::Or, Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Parse<Expr> {
        let mut left = self.not()?;
        while self.eat_kw("AND") {
            left = Expr::Bin(Op::And, Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Parse<Expr> {
        if self.eat_kw("NOT") {
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Parse<Expr> {
        let left = self.sum()?;
        for (sym, op) in [
            ("=", Op::Eq),
            ("!=", Op::Ne),
            ("<=", Op::Le),
            (">=", Op::Ge),
            ("<", Op::Lt),
            (">", Op::Gt),
        ] {
            if self.eat_sym(sym) {
                return Ok(Expr::Bin(op, Box::new(left), Box::new(self.sum()?)));
            }
        }
        if self.eat_kw("IS") {
            let negated = self.eat_kw("NOT");
            self.expect_kw("NULL")?;
            return Ok(Expr::IsNull(Box::new(left), negated));
        }
        let negated = self.is_kw("NOT")
            && matches!(self.toks.get(self.pos + 1), Some((Tok::Word(w), ..)) if ["LIKE", "IN", "BETWEEN"].contains(&w.to_uppercase().as_str()));
        if negated {
            self.pos += 1;
        }
        if self.eat_kw("LIKE") {
            return Ok(Expr::Like(Box::new(left), Box::new(self.sum()?), negated));
        }
        if self.eat_kw("IN") {
            self.expect_sym("(")?;
            let mut list = vec![self.expr()?];
            while self.eat_sym(",") {
                list.push(self.expr()?);
            }
            self.expect_sym(")")?;
            return Ok(Expr::In(Box::new(left), list, negated));
        }
        if self.eat_kw("BETWEEN") {
            let low = self.sum()?;
            self.expect_kw("AND")?;
            let high = self.sum()?;
            return Ok(Expr::Between(
                Box::new(left),
                Box::new(low),
                Box::new(high),
                negated,
            ));
        }
        Ok(left)
    }

    fn sum(&mut self) -> Parse<Expr> {
        let mut left = self.product()?;
        loop {
            let op = if self.eat_sym("+") {
                Op::Add
            } else if self.eat_sym("-") {
                Op::Sub
            } else {
                return Ok(left);
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Parse<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat_sym("*") {
                Op::Mul
            } else if self.eat_sym("/") {
                Op::Div
            } else {
                return Ok(left);
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Parse<Expr> {
        if self.eat_sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Parse<Expr> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                if let Ok(i) = n.parse::<i64>() {
                    Ok(Expr::Lit(Cell::Int(i)))
                } else {
                    n.parse::<f64>()
                        .map(|f| Expr::Lit(Cell::Float(f)))
                        .map_err(|_| format!("Bad number '{n}'."))
                }
            }
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Ok(Expr::Lit(Cell::Text(s)))
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Some(Tok::Word(w)) => {
                let up = w.to_uppercase();
                match up.as_str() {
                    "NULL" => {
                        self.pos += 1;
                        return Ok(Expr::Lit(Cell::Null));
                    }
                    "TRUE" | "FALSE" => {
                        self.pos += 1;
                        return Ok(Expr::Lit(Cell::Bool(up == "TRUE")));
                    }
                    _ => {}
                }
                if RESERVED.contains(&up.as_str()) {
                    return self.fail("an expression");
                }
                self.pos += 1;
                if self.eat_sym("(") {
                    if self.eat_sym("*") {
                        self.expect_sym(")")?;
                        return Ok(Expr::Call(up, vec![], true));
                    }
                    let mut args = Vec::new();
                    if !self.eat_sym(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_sym(",") {
                                break;
                            }
                        }
                        self.expect_sym(")")?;
                    }
                    return Ok(Expr::Call(up, args, false));
                }
                if self.eat_sym(".") {
                    let col = self.ident("a column name")?;
                    return Ok(Expr::Col(Some(w.to_lowercase()), col.to_lowercase()));
                }
                Ok(Expr::Col(None, w.to_lowercase()))
            }
            _ => self.fail("an expression"),
        }
    }
}

// --- evaluation --------------------------------------------------------------

#[derive(Clone)]
struct Col {
    table: String,
    name: String,
}

struct Cx<'a> {
    cols: &'a [Col],
    row: &'a [Cell],
    /// The rows of the group, inside an aggregate query.
    group: Option<&'a [Vec<Cell>]>,
    /// Output values by lowercase name, for `ORDER BY` and `HAVING`.
    aliases: &'a [(String, Cell)],
}

fn compare(a: &Cell, b: &Cell) -> Result<Option<Ordering>, String> {
    Ok(match (a, b) {
        (Cell::Null, _) | (_, Cell::Null) => None,
        (Cell::Text(x), Cell::Text(y)) => Some(x.cmp(y)),
        (Cell::Bool(x), Cell::Bool(y)) => Some(x.cmp(y)),
        _ => match (a.number(), b.number()) {
            (Some(x), Some(y)) => x.partial_cmp(&y),
            _ => return Err(format!("Cannot compare {} with {}.", a.show(), b.show())),
        },
    })
}

fn like(text: &[char], pat: &[char]) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some(('%', rest)) => (0..=text.len()).any(|i| like(&text[i..], rest)),
        Some((p, rest)) => text
            .split_first()
            .is_some_and(|(t, tail)| (*p == '_' || *p == *t) && like(tail, rest)),
    }
}

fn arithmetic(op: Op, a: &Cell, b: &Cell) -> Result<Cell, String> {
    if matches!(a, Cell::Null) || matches!(b, Cell::Null) {
        return Ok(Cell::Null);
    }
    if let (Cell::Int(x), Cell::Int(y)) = (a, b) {
        let r = match op {
            Op::Add => x.checked_add(*y),
            Op::Sub => x.checked_sub(*y),
            Op::Mul => x.checked_mul(*y),
            _ if *y == 0 => return Ok(Cell::Null),
            _ if x % y == 0 => x.checked_div(*y),
            _ => return Ok(Cell::Float(*x as f64 / *y as f64)),
        };
        return r
            .map(Cell::Int)
            .ok_or_else(|| "Integer overflow.".to_owned());
    }
    let (Some(x), Some(y)) = (a.number(), b.number()) else {
        return Err(format!(
            "Cannot do arithmetic on {} and {}.",
            a.show(),
            b.show()
        ));
    };
    Ok(match op {
        Op::Add => Cell::Float(x + y),
        Op::Sub => Cell::Float(x - y),
        Op::Mul => Cell::Float(x * y),
        _ if y == 0.0 => Cell::Null,
        _ => Cell::Float(x / y),
    })
}

fn eval(e: &Expr, cx: &Cx) -> Result<Cell, String> {
    Ok(match e {
        Expr::Lit(c) => c.clone(),
        Expr::Col(table, name) => {
            let hits: Vec<usize> = cx
                .cols
                .iter()
                .enumerate()
                .filter(|(_, c)| c.name == *name && table.as_ref().is_none_or(|t| c.table == *t))
                .map(|(i, _)| i)
                .collect();
            match hits.as_slice() {
                [i] => cx.row[*i].clone(),
                [] if table.is_none() => match cx.aliases.iter().find(|(n, _)| n == name) {
                    Some((_, v)) => v.clone(),
                    None => return Err(format!("Unknown column '{name}'.")),
                },
                [] => {
                    return Err(format!(
                        "Unknown column '{}.{name}'.",
                        table.as_deref().unwrap_or("")
                    ))
                }
                _ => return Err(format!("Column '{name}' is ambiguous: say which table.")),
            }
        }
        Expr::Neg(e) => arithmetic(Op::Sub, &Cell::Int(0), &eval(e, cx)?)?,
        Expr::Not(e) => match eval(e, cx)? {
            Cell::Null => Cell::Null,
            v => Cell::Bool(!v.truthy()),
        },
        Expr::Bin(Op::And, a, b) => {
            let (x, y) = (eval(a, cx)?, eval(b, cx)?);
            match (&x, &y) {
                (Cell::Bool(false), _) | (_, Cell::Bool(false)) => Cell::Bool(false),
                (Cell::Null, _) | (_, Cell::Null) => Cell::Null,
                _ => Cell::Bool(true),
            }
        }
        Expr::Bin(Op::Or, a, b) => {
            let (x, y) = (eval(a, cx)?, eval(b, cx)?);
            match (&x, &y) {
                (Cell::Bool(true), _) | (_, Cell::Bool(true)) => Cell::Bool(true),
                (Cell::Null, _) | (_, Cell::Null) => Cell::Null,
                _ => Cell::Bool(false),
            }
        }
        Expr::Bin(op @ (Op::Add | Op::Sub | Op::Mul | Op::Div), a, b) => {
            arithmetic(*op, &eval(a, cx)?, &eval(b, cx)?)?
        }
        Expr::Bin(op, a, b) => match compare(&eval(a, cx)?, &eval(b, cx)?)? {
            None => Cell::Null,
            Some(o) => Cell::Bool(match op {
                Op::Eq => o == Ordering::Equal,
                Op::Ne => o != Ordering::Equal,
                Op::Lt => o == Ordering::Less,
                Op::Le => o != Ordering::Greater,
                Op::Gt => o == Ordering::Greater,
                _ => o != Ordering::Less,
            }),
        },
        Expr::Like(a, p, neg) => match (eval(a, cx)?, eval(p, cx)?) {
            (Cell::Null, _) | (_, Cell::Null) => Cell::Null,
            (x, Cell::Text(p)) => {
                let t: Vec<char> = x.show().to_lowercase().chars().collect();
                let p: Vec<char> = p.to_lowercase().chars().collect();
                Cell::Bool(like(&t, &p) != *neg)
            }
            _ => return Err("LIKE needs a text pattern.".into()),
        },
        Expr::In(a, list, neg) => {
            let x = eval(a, cx)?;
            if matches!(x, Cell::Null) {
                return Ok(Cell::Null);
            }
            let mut found = false;
            for item in list {
                found |= compare(&x, &eval(item, cx)?)? == Some(Ordering::Equal);
            }
            Cell::Bool(found != *neg)
        }
        Expr::Between(a, lo, hi, neg) => {
            let x = eval(a, cx)?;
            match (compare(&x, &eval(lo, cx)?)?, compare(&x, &eval(hi, cx)?)?) {
                (Some(l), Some(h)) => {
                    Cell::Bool((l != Ordering::Less && h != Ordering::Greater) != *neg)
                }
                _ => Cell::Null,
            }
        }
        Expr::IsNull(a, neg) => Cell::Bool(matches!(eval(a, cx)?, Cell::Null) != *neg),
        Expr::Call(name, args, star) => call(name, args, *star, cx)?,
    })
}

fn call(name: &str, args: &[Expr], star: bool, cx: &Cx) -> Result<Cell, String> {
    if AGGREGATES.contains(&name) {
        let Some(rows) = cx.group else {
            return Err(format!("{name}() is not allowed here."));
        };
        if star {
            return if name == "COUNT" {
                Ok(Cell::Int(rows.len() as i64))
            } else {
                Err(format!("{name}(*) is not valid."))
            };
        }
        let [arg] = args else {
            return Err(format!("{name}() takes one argument."));
        };
        let mut values = Vec::new();
        for row in rows {
            let v = eval(
                arg,
                &Cx {
                    cols: cx.cols,
                    row,
                    group: None,
                    aliases: cx.aliases,
                },
            )?;
            if !matches!(v, Cell::Null) {
                values.push(v);
            }
        }
        return match name {
            "COUNT" => Ok(Cell::Int(values.len() as i64)),
            "MIN" | "MAX" => {
                let mut best: Option<Cell> = None;
                for v in values {
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let o = compare(&v, b)?;
                            (name == "MIN" && o == Some(Ordering::Less))
                                || (name == "MAX" && o == Some(Ordering::Greater))
                        }
                    };
                    if better {
                        best = Some(v);
                    }
                }
                Ok(best.unwrap_or(Cell::Null))
            }
            _ => {
                if values.is_empty() {
                    return Ok(Cell::Null);
                }
                if values.iter().any(|v| v.number().is_none()) {
                    return Err(format!("{name}() needs numbers."));
                }
                let sum: f64 = values.iter().filter_map(Cell::number).sum();
                if name == "AVG" {
                    Ok(Cell::Float(sum / values.len() as f64))
                } else if values.iter().all(|v| matches!(v, Cell::Int(_))) {
                    Ok(Cell::Int(sum as i64))
                } else {
                    Ok(Cell::Float(sum))
                }
            }
        };
    }
    let vals = args
        .iter()
        .map(|a| eval(a, cx))
        .collect::<Result<Vec<_>, _>>()?;
    match (name, vals.as_slice()) {
        (_, [Cell::Null, ..]) => Ok(Cell::Null),
        ("UPPER", [v]) => Ok(Cell::Text(v.show().to_uppercase())),
        ("LOWER", [v]) => Ok(Cell::Text(v.show().to_lowercase())),
        ("LENGTH", [v]) => Ok(Cell::Int(v.show().chars().count() as i64)),
        ("ABS", [v]) => v
            .number()
            .map(|n| {
                if matches!(v, Cell::Int(_)) {
                    Cell::Int(n.abs() as i64)
                } else {
                    Cell::Float(n.abs())
                }
            })
            .ok_or_else(|| "ABS() needs a number.".into()),
        ("ROUND", [v]) => v
            .number()
            .map(|n| Cell::Float(n.round()))
            .ok_or_else(|| "ROUND() needs a number.".into()),
        ("ROUND", [v, Cell::Int(d)]) => v
            .number()
            .map(|n| {
                let k = 10f64.powi((*d).clamp(0, 9) as i32);
                Cell::Float((n * k).round() / k)
            })
            .ok_or_else(|| "ROUND() needs a number.".into()),
        ("SUBSTR", [v, Cell::Int(from), rest @ ..]) => {
            let chars: Vec<char> = v.show().chars().collect();
            let start = (*from - 1).max(0) as usize;
            let len = match rest {
                [Cell::Int(n)] => (*n).max(0) as usize,
                _ => usize::MAX,
            };
            Ok(Cell::Text(chars.iter().skip(start).take(len).collect()))
        }
        _ => Err(format!("Unknown function {name}() or wrong arguments.")),
    }
}

// --- running a query ---------------------------------------------------------

struct Answer {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

fn find<'a>(db: &'a [Table], name: &str) -> Result<&'a Table, String> {
    db.iter().find(|t| t.name == name).ok_or_else(|| {
        format!(
            "Unknown table '{name}'. Tables: {}.",
            db.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        )
    })
}

fn columns_of(table: &Table, alias: &str) -> Vec<Col> {
    table
        .columns
        .iter()
        .map(|(n, _)| Col {
            table: alias.to_owned(),
            name: (*n).to_owned(),
        })
        .collect()
}

fn run(db: &[Table], sql: &str) -> Result<Answer, String> {
    let words: Vec<String> = sql
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    match words
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => return Err("Write a query first.".into()),
        ["show", "tables"] => {
            return Ok(Answer {
                columns: vec!["table".into(), "rows".into()],
                rows: db
                    .iter()
                    .map(|t| vec![Cell::text(t.name), Cell::Int(t.rows.len() as i64)])
                    .collect(),
            })
        }
        ["describe" | "desc", t] => {
            return Ok(Answer {
                columns: vec!["column".into(), "type".into()],
                rows: find(db, t)?
                    .columns
                    .iter()
                    .map(|(n, ty)| vec![Cell::text(n), Cell::text(ty)])
                    .collect(),
            })
        }
        [first, ..] if *first != "select" => {
            return Err(
                if ["insert", "update", "delete", "drop", "alter", "create"].contains(first) {
                    "This connection is read-only: only SELECT, SHOW TABLES and DESCRIBE work."
                        .into()
                } else {
                    format!("Expected SELECT, SHOW TABLES or DESCRIBE, not '{first}'.")
                },
            )
        }
        _ => {}
    }
    let toks = tokenize(sql)?;
    let q = Parser {
        src: sql,
        toks,
        pos: 0,
    }
    .select()?;

    // The rows to work on: the first table, then each join.
    let base = find(db, &q.from.name)?;
    let mut cols = columns_of(base, &q.from.alias);
    let mut rows: Vec<Vec<Cell>> = base.rows.clone();
    for join in &q.joins {
        let right = find(db, &join.table.name)?;
        let right_cols = columns_of(right, &join.table.alias);
        let mut all = cols.clone();
        all.extend(right_cols);
        let mut joined = Vec::new();
        for l in &rows {
            let mut matched = false;
            for r in &right.rows {
                let mut row = l.clone();
                row.extend(r.iter().cloned());
                let cx = Cx {
                    cols: &all,
                    row: &row,
                    group: None,
                    aliases: &[],
                };
                if eval(&join.on, &cx)?.truthy() {
                    matched = true;
                    joined.push(row);
                }
            }
            if !matched && join.left {
                let mut row = l.clone();
                row.extend(std::iter::repeat_n(Cell::Null, right.columns.len()));
                joined.push(row);
            }
        }
        cols = all;
        rows = joined;
    }
    if let Some(f) = &q.filter {
        let mut kept = Vec::new();
        for row in rows {
            let cx = Cx {
                cols: &cols,
                row: &row,
                group: None,
                aliases: &[],
            };
            if eval(f, &cx)?.truthy() {
                kept.push(row);
            }
        }
        rows = kept;
    }

    // Output columns.
    let mut names = Vec::new();
    let mut exprs: Vec<Expr> = Vec::new();
    for item in &q.items {
        match &item.expr {
            Some(e) => {
                names.push(item.name.clone());
                exprs.push(e.clone());
            }
            None => {
                for c in cols
                    .iter()
                    .filter(|c| item.star_of.as_ref().is_none_or(|t| c.table == *t))
                {
                    names.push(c.name.clone());
                    exprs.push(Expr::Col(Some(c.table.clone()), c.name.clone()));
                }
                if item.star_of.is_some() && names.is_empty() {
                    return Err(format!(
                        "Unknown table '{}' in the select list.",
                        item.star_of.as_deref().unwrap_or("")
                    ));
                }
            }
        }
    }
    let grouped = !q.group.is_empty()
        || exprs.iter().any(Expr::has_aggregate)
        || q.having.is_some()
        || q.order.iter().any(|(e, _)| e.has_aggregate());

    // Each output row, with what its sort keys are computed from.
    struct Out {
        values: Vec<Cell>,
        keys: Vec<Cell>,
    }
    let order_exprs: Vec<Expr> = q.order.iter().map(|(e, _)| e.clone()).collect();
    let mut out: Vec<Out> = Vec::new();
    let mut emit = |row: &[Cell], group: Option<&[Vec<Cell>]>| -> Result<(), String> {
        let cx = Cx {
            cols: &cols,
            row,
            group,
            aliases: &[],
        };
        let values = exprs
            .iter()
            .map(|e| eval(e, &cx))
            .collect::<Result<Vec<_>, _>>()?;
        let aliases: Vec<(String, Cell)> = names
            .iter()
            .map(|n| n.to_lowercase())
            .zip(values.iter().cloned())
            .collect();
        let cx = Cx {
            cols: &cols,
            row,
            group,
            aliases: &aliases,
        };
        if let Some(h) = &q.having {
            if !eval(h, &cx)?.truthy() {
                return Ok(());
            }
        }
        let mut keys = Vec::new();
        for e in &order_exprs {
            keys.push(match e {
                Expr::Lit(Cell::Int(n)) if *n >= 1 && (*n as usize) <= values.len() => {
                    values[*n as usize - 1].clone()
                }
                e => eval(e, &cx)?,
            });
        }
        out.push(Out { values, keys });
        Ok(())
    };
    if grouped {
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<Vec<Cell>>> = HashMap::new();
        for row in &rows {
            let cx = Cx {
                cols: &cols,
                row,
                group: None,
                aliases: &[],
            };
            let key = q
                .group
                .iter()
                .map(|g| eval(g, &cx).map(|v| format!("{v:?}")))
                .collect::<Result<Vec<_>, _>>()?
                .join("|");
            if !groups.contains_key(&key) {
                order.push(key.clone());
            }
            groups.entry(key).or_default().push(row.clone());
        }
        if q.group.is_empty() && order.is_empty() {
            order.push(String::new());
            groups.insert(String::new(), Vec::new());
        }
        for key in order {
            let members = &groups[&key];
            let first = members
                .first()
                .cloned()
                .unwrap_or_else(|| vec![Cell::Null; cols.len()]);
            emit(&first, Some(members))?;
        }
    } else {
        for row in &rows {
            emit(row, None)?;
        }
    }

    if !q.order.is_empty() {
        let mut failure = None;
        out.sort_by(|a, b| {
            for (i, (_, desc)) in q.order.iter().enumerate() {
                let o = match compare(&a.keys[i], &b.keys[i]) {
                    Ok(Some(o)) => o,
                    // NULLs sort first.
                    Ok(None) => match (
                        matches!(a.keys[i], Cell::Null),
                        matches!(b.keys[i], Cell::Null),
                    ) {
                        (true, false) => Ordering::Less,
                        (false, true) => Ordering::Greater,
                        _ => Ordering::Equal,
                    },
                    Err(e) => {
                        failure = Some(e);
                        Ordering::Equal
                    }
                };
                if o != Ordering::Equal {
                    return if *desc { o.reverse() } else { o };
                }
            }
            Ordering::Equal
        });
        if let Some(e) = failure {
            return Err(e);
        }
    }
    let mut result: Vec<Vec<Cell>> = out.into_iter().map(|o| o.values).collect();
    if q.distinct {
        let mut seen = std::collections::HashSet::new();
        result.retain(|r| seen.insert(format!("{r:?}")));
    }
    let result: Vec<Vec<Cell>> = result
        .into_iter()
        .skip(q.offset)
        .take(q.limit.unwrap_or(usize::MAX).min(MAX_ROWS))
        .collect();
    Ok(Answer {
        columns: names,
        rows: result,
    })
}

// --- the app -------------------------------------------------------------------

const EXAMPLES: &[(&str, &str)] = &[
    ("Pick an example…", ""),
    ("Customers on the pro plan", "SELECT name, country, signup\nFROM customers\nWHERE plan = 'pro'\nORDER BY country, name"),
    ("Revenue by product", "SELECT p.name, SUM(o.total) AS revenue, COUNT(*) AS orders\nFROM orders o\nJOIN products p ON p.id = o.product_id\nWHERE o.status != 'cancelled'\nGROUP BY p.name\nORDER BY revenue DESC\nLIMIT 5"),
    ("Orders per customer", "SELECT c.name, COUNT(o.id) AS orders\nFROM customers c\nLEFT JOIN orders o ON o.customer_id = c.id\nGROUP BY c.name\nORDER BY orders DESC, c.name"),
    ("Customers who never ordered", "SELECT c.id, c.name\nFROM customers c\nLEFT JOIN orders o ON o.customer_id = c.id\nWHERE o.id IS NULL"),
    ("Salary by department", "SELECT department, COUNT(*) AS people, AVG(salary) AS average, MAX(salary) AS top\nFROM employees\nGROUP BY department\nHAVING COUNT(*) > 1\nORDER BY average DESC"),
    ("Who reports to whom", "SELECT e.name, m.name AS manager\nFROM employees e\nLEFT JOIN employees m ON m.id = e.manager_id\nORDER BY manager, e.name"),
    ("Running low on stock", "SELECT name, category, stock\nFROM products\nWHERE stock < 50 OR name LIKE '%desk%'\nORDER BY stock"),
];

#[derive(Default)]
pub struct DbClient {
    db: Vec<Table>,
    /// The rendered result: header, rule, then one line per row.
    lines: Vec<String>,
    page: usize,
    history: Vec<String>,
}

fn clip(s: &str) -> String {
    if s.chars().count() > MAX_WIDTH {
        format!("{}…", s.chars().take(MAX_WIDTH - 1).collect::<String>())
    } else {
        s.to_owned()
    }
}

fn render(answer: &Answer) -> Vec<String> {
    let shown: Vec<Vec<String>> = answer
        .rows
        .iter()
        .map(|r| r.iter().map(|c| clip(&c.show())).collect())
        .collect();
    let widths: Vec<usize> = answer
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            shown
                .iter()
                .map(|r| r[i].chars().count())
                .chain([clip(c).chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<String>, numeric: Option<&Vec<Cell>>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let right = numeric.is_some_and(|r| r[i].number().is_some());
                if right {
                    format!("{c:>w$}", w = widths[i])
                } else {
                    format!("{c:<w$}", w = widths[i])
                }
            })
            .collect::<Vec<_>>()
            .join(" │ ")
    };
    let mut lines = vec![
        line(answer.columns.iter().map(|c| clip(c)).collect(), None),
        widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┼─"),
    ];
    for (cells, raw) in shown.into_iter().zip(&answer.rows) {
        lines.push(line(cells, Some(raw)));
    }
    lines
}

impl DbClient {
    fn show_page(&self, out: &mut Outbox) {
        let rows = self.lines.len().saturating_sub(2);
        let pages = rows.div_ceil(PAGE).max(1);
        let mut lines: Vec<String> = Vec::new();
        if !self.lines.is_empty() {
            lines.extend(self.lines[..2].iter().cloned());
            lines.extend(
                self.lines
                    .iter()
                    .skip(2 + self.page * PAGE)
                    .take(PAGE)
                    .cloned(),
            );
        }
        out.signal(json!({"result": {
            "lines": lines,
            "page": format!("Page {} of {pages}", self.page + 1),
            "has_prev": self.page > 0,
            "has_next": self.page + 1 < pages,
        }}));
    }

    fn show_history(&self, out: &mut Outbox) {
        let labels: Vec<String> = self
            .history
            .iter()
            .map(|q| {
                let one: String = q.split_whitespace().collect::<Vec<_>>().join(" ");
                if one.chars().count() > 60 {
                    format!("{}…", one.chars().take(59).collect::<String>())
                } else {
                    one
                }
            })
            .collect();
        out.signal(json!({"history": labels}));
    }

    fn show_schema(&self, out: &mut Outbox, table: &str) {
        let Ok(t) = find(&self.db, table) else { return };
        let mut lines: Vec<String> = t
            .columns
            .iter()
            .map(|(n, ty)| format!("{n:<12} {ty}"))
            .collect();
        lines.resize(8, String::new());
        out.signal(json!({"schema": {"lines": lines, "rows": format!("{} rows", t.rows.len())}}));
    }
}

impl App for DbClient {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        self.db = database();
        out.signal(json!({
            "form": {"sql": "SELECT * FROM customers LIMIT 20", "table": "customers", "example": ""},
            "menu": {
                "tables": self.db.iter().map(|t| json!({"value": t.name, "label": t.name})).collect::<Vec<_>>(),
                "examples": menu(&EXAMPLES.iter().enumerate().map(|(i, (l, _))| (["0","1","2","3","4","5","6","7","8"][i], *l)).collect::<Vec<_>>()),
            },
            "status": {"kind": "info", "text": ""},
            "result": {"lines": [], "page": "", "has_prev": false, "has_next": false, "info": ""},
            "history": [],
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Tabula — SQL Client"),
        );
        out.element(ui::text("conn").parent("main").variant("caption").text(
            "Connected to acme_demo (read-only, in memory). Try SHOW TABLES or DESCRIBE orders.",
        ));

        out.element(
            ui::card("schema")
                .parent("main")
                .title("Schema")
                .gap("small"),
        );
        out.element(
            ui::select("table")
                .parent("schema")
                .bind("form.table")
                .options(cel("s.menu.tables"))
                .on_change(Handler::action("table")),
        );
        out.element(
            ui::badge("table_rows")
                .parent("schema")
                .variant("info")
                .text(cel("s.schema.rows")),
        );
        for k in 0..8 {
            out.element(
                ui::text(format!("col_{k}"))
                    .parent("schema")
                    .variant("code")
                    .visible(cel(format!("s.schema.lines[{k}] != ''")))
                    .text(cel(format!("s.schema.lines[{k}]"))),
            );
        }

        out.element(
            ui::card("query")
                .parent("main")
                .title("Query")
                .gap("medium"),
        );
        out.element(
            ui::select("example")
                .parent("query")
                .label("Examples")
                .bind("form.example")
                .options(cel("s.menu.examples"))
                .on_change(Handler::action("example")),
        );
        out.element(
            ui::input("sql")
                .parent("query")
                .bind("form.sql")
                .variant("multiline")
                .placeholder("SELECT …"),
        );
        out.element(ui::row("query_buttons").parent("query").gap("small"));
        out.element(
            ui::button("run")
                .parent("query_buttons")
                .label("Run")
                .variant("primary")
                .on_press(Handler::action("run")),
        );
        out.element(
            ui::button("clear")
                .parent("query_buttons")
                .label("Clear")
                .variant("plain")
                .on_press(Handler::patch(json!({"form": {"sql": ""}}))),
        );

        out.element(
            ui::card("results")
                .parent("main")
                .title("Result")
                .gap("small"),
        );
        out.element(
            ui::notice("status")
                .parent("results")
                .variant(cel("s.status.kind"))
                .visible(cel("s.status.text != ''"))
                .text(cel("s.status.text")),
        );
        for k in 0..PAGE + 2 {
            out.element(
                ui::text(format!("line_{k}"))
                    .parent("results")
                    .variant("code")
                    .visible(cel(format!("size(s.result.lines) > {k}")))
                    .text(cel(format!("s.result.lines[{k}]"))),
            );
        }
        out.element(
            ui::row("pager")
                .parent("results")
                .gap("small")
                .visible(cel("s.result.has_prev || s.result.has_next")),
        );
        out.element(
            ui::button("prev")
                .parent("pager")
                .label("← Previous")
                .disabled(cel("!s.result.has_prev"))
                .on_press(Handler::action("page").payload(-1)),
        );
        out.element(ui::badge("page").parent("pager").text(cel("s.result.page")));
        out.element(
            ui::button("next")
                .parent("pager")
                .label("Next →")
                .disabled(cel("!s.result.has_next"))
                .on_press(Handler::action("page").payload(1)),
        );

        out.element(
            ui::card("recent")
                .parent("main")
                .title("Recent queries")
                .visible(cel("size(s.history) > 0"))
                .gap("small"),
        );
        for k in 0..HISTORY {
            out.element(
                ui::button(format!("hist_{k}"))
                    .parent("recent")
                    .variant("plain")
                    .visible(cel(format!("size(s.history) > {k}")))
                    .label(cel(format!("s.history[{k}]")))
                    .on_press(Handler::action("recall").payload(k)),
            );
        }
        self.show_schema(out, "customers");
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        match action.name.as_str() {
            "table" => {
                let t = action
                    .signal("form.table")
                    .and_then(Value::as_str)
                    .unwrap_or("customers");
                self.show_schema(out, t);
                if let Ok(table) = find(&self.db, t) {
                    out.signal(json!({"form": {"sql": format!("SELECT * FROM {}", table.name)}}));
                }
            }
            "example" => {
                let k: usize = action
                    .signal("form.example")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if let Some((_, sql)) = EXAMPLES.get(k).filter(|(_, s)| !s.is_empty()) {
                    out.signal(json!({"form": {"sql": sql}}));
                }
            }
            "recall" => {
                if let Some(sql) = action
                    .payload_as::<usize>()
                    .and_then(|k| self.history.get(k))
                {
                    out.signal(json!({"form": {"sql": sql}}));
                }
            }
            "page" => {
                let step = action.payload_as::<i64>().unwrap_or(0);
                let pages = self.lines.len().saturating_sub(2).div_ceil(PAGE).max(1);
                self.page = (self.page as i64 + step).clamp(0, pages as i64 - 1) as usize;
                self.show_page(out);
            }
            "run" => {
                let sql = action
                    .signal("form.sql")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let started = Stopwatch::start();
                match run(&self.db, &sql) {
                    Ok(answer) => {
                        let took = started
                            .ms()
                            .map(|ms| format!(" in {ms:.1} ms"))
                            .unwrap_or_default();
                        self.lines = render(&answer);
                        self.page = 0;
                        out.signal(json!({"status": {"kind": "success", "text": format!(
                            "{} row{}{took}.", answer.rows.len(), if answer.rows.len() == 1 { "" } else { "s" }
                        )}}));
                        self.history.retain(|q| *q != sql);
                        self.history.insert(0, sql);
                        self.history.truncate(HISTORY);
                        self.show_history(out);
                    }
                    Err(e) => {
                        self.lines.clear();
                        self.page = 0;
                        out.signal(json!({"status": {"kind": "failure", "text": e}}));
                    }
                }
                self.show_page(out);
            }
            _ => {}
        }
    }
}

/// Times a query where there is a clock; wasm modules have none.
struct Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Stopwatch {
    fn start() -> Self {
        Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant::now())
    }

    fn ms(&self) -> Option<f64> {
        #[cfg(not(target_arch = "wasm32"))]
        return Some(self.0.elapsed().as_secs_f64() * 1000.0);
        #[cfg(target_arch = "wasm32")]
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn q(sql: &str) -> Result<Vec<Vec<String>>, String> {
        run(&database(), sql).map(|a| {
            a.rows
                .iter()
                .map(|r| r.iter().map(Cell::show).collect())
                .collect()
        })
    }

    fn one(sql: &str) -> String {
        q(sql).unwrap()[0][0].clone()
    }

    #[test]
    fn counts_and_filters() {
        assert_eq!(one("SELECT COUNT(*) FROM customers"), "30");
        assert_eq!(
            one("select count(*) from employees where department = 'engineering'"),
            "5"
        );
        assert_eq!(
            one("SELECT COUNT(*) FROM employees WHERE manager_id IS NULL"),
            "1"
        );
        assert_eq!(
            one("SELECT COUNT(*) FROM employees WHERE salary BETWEEN 80000 AND 100000"),
            "3"
        );
        assert_eq!(
            one("SELECT COUNT(*) FROM employees WHERE department IN ('sales', 'finance')"),
            "5"
        );
        assert_eq!(
            one("SELECT COUNT(*) FROM products WHERE name LIKE '%DESK%'"),
            "3"
        );
        assert_eq!(
            one("SELECT COUNT(*) FROM products WHERE NOT (category = 'home' OR price > 100)"),
            "4"
        );
        assert_eq!(one("SELECT 2 + 3 * 4 - 1 FROM products LIMIT 1"), "13");
        assert_eq!(one("SELECT 7 / 2 FROM products LIMIT 1"), "3.50");
        assert_eq!(
            one("SELECT UPPER(name) FROM employees LIMIT 1"),
            "PRIYA RAMAN"
        );
    }

    #[test]
    fn grouping_ordering_and_limits() {
        let rows = q("SELECT department, COUNT(*) AS n, MAX(salary) FROM employees GROUP BY department ORDER BY n DESC, department").unwrap();
        assert_eq!(rows[0], ["engineering", "5", "142000"]);
        assert_eq!(rows.len(), 5);
        let rows =
            q("SELECT department FROM employees GROUP BY department HAVING COUNT(*) = 1").unwrap();
        assert_eq!(rows, [["management"]]);
        let rows = q("SELECT name FROM employees ORDER BY salary DESC LIMIT 2 OFFSET 1").unwrap();
        assert_eq!(rows, [["Tomás Ruiz"], ["Clara Jones"]]);
        assert_eq!(q("SELECT DISTINCT plan FROM customers").unwrap().len(), 4);
        // An aggregate over nothing is one row of nothing.
        assert_eq!(
            q("SELECT COUNT(*), SUM(salary) FROM employees WHERE salary < 0").unwrap(),
            [["0", "NULL"]]
        );
        // Ordering by position.
        assert_eq!(
            q("SELECT name, salary FROM employees ORDER BY 2 LIMIT 1").unwrap()[0][0],
            "Oscar Webb"
        );
    }

    #[test]
    fn joins() {
        // Every order has a product, so an inner join keeps them all.
        assert_eq!(
            one("SELECT COUNT(*) FROM orders o JOIN products p ON p.id = o.product_id"),
            "80"
        );
        // The boss has no manager; a left join keeps them with NULL.
        let rows = q("SELECT e.name, m.name FROM employees e LEFT JOIN employees m ON m.id = e.manager_id WHERE m.id IS NULL").unwrap();
        assert_eq!(rows, [["Priya Raman", "NULL"]]);
        // Totals add up: revenue per product sums to the revenue overall.
        let per: f64 = q("SELECT SUM(o.total) FROM orders o JOIN products p ON p.id = o.product_id GROUP BY p.name")
            .unwrap()
            .iter()
            .map(|r| r[0].parse::<f64>().unwrap())
            .sum();
        let all: f64 = one("SELECT SUM(total) FROM orders").parse().unwrap();
        assert!((per - all).abs() < 0.5, "{per} vs {all}");
        assert!(
            q("SELECT name FROM customers c JOIN products p ON p.id = c.id")
                .unwrap_err()
                .contains("ambiguous")
        );
        assert_eq!(
            q("SELECT c.* FROM customers c LIMIT 1").unwrap()[0].len(),
            6
        );
    }

    #[test]
    fn builtin_commands_and_errors() {
        assert_eq!(q("SHOW TABLES").unwrap().len(), 4);
        assert_eq!(q("describe orders;").unwrap().len(), 7);
        assert!(q("SELEC * FROM customers")
            .unwrap_err()
            .contains("Expected SELECT"));
        assert!(q("SELECT * FORM customers")
            .unwrap_err()
            .contains("near 'FORM'"));
        assert!(q("SELECT * FROM nothing")
            .unwrap_err()
            .contains("Unknown table 'nothing'"));
        assert!(q("SELECT nope FROM customers")
            .unwrap_err()
            .contains("Unknown column 'nope'"));
        assert!(q("DELETE FROM customers")
            .unwrap_err()
            .contains("read-only"));
        assert!(q("SELECT name FROM customers WHERE name = 'x")
            .unwrap_err()
            .contains("Unterminated"));
        assert!(q("SELECT SUM(name) FROM customers")
            .unwrap_err()
            .contains("needs numbers"));
        assert!(q("SELECT name FROM customers WHERE COUNT(*) > 1")
            .unwrap_err()
            .contains("not allowed here"));
        assert!(q("   ").is_err());
    }

    #[test]
    fn the_examples_all_run() {
        for (label, sql) in EXAMPLES.iter().filter(|(_, s)| !s.is_empty()) {
            let rows = q(sql).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert!(!rows.is_empty(), "{label} returned nothing");
        }
    }

    #[test]
    fn the_interface() {
        let mut l = Loopback::<DbClient>::new();
        assert_eq!(
            l.client.attr("col_0", "text"),
            Some(json!("id           INTEGER"))
        );
        assert_eq!(l.client.attr("line_0", "visible"), Some(json!(false)));
        assert!(l.trigger("run", "onPress"));
        let status = l.client.attr("status", "text").unwrap();
        assert!(status.as_str().unwrap().starts_with("20 rows in "));
        assert_eq!(l.client.attr("status", "variant"), Some(json!("success")));
        // Header, rule and a page of ten rows.
        assert_eq!(l.client.attr("line_11", "visible"), Some(json!(true)));
        assert_eq!(l.client.attr("page", "text"), Some(json!("Page 1 of 2")));
        assert_eq!(l.client.attr("prev", "disabled"), Some(json!(true)));
        assert!(l.trigger("next", "onPress"));
        assert_eq!(l.client.attr("page", "text"), Some(json!("Page 2 of 2")));
        assert_eq!(l.client.attr("next", "disabled"), Some(json!(true)));
        // Choosing an example fills the editor; running it records history.
        l.write("form.example", json!("2"));
        assert!(l.trigger("example", "onChange"));
        assert!(l
            .client
            .value("form.sql")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("SELECT p.name, SUM"));
        assert!(l.trigger("run", "onPress"));
        assert_eq!(l.client.attr("hist_1", "visible"), Some(json!(true)));
        assert!(l
            .client
            .attr("hist_0", "label")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("SELECT p.name, SUM"));
        assert!(l.trigger("hist_1", "onPress"));
        assert_eq!(
            l.client.value("form.sql"),
            Some(&json!("SELECT * FROM customers LIMIT 20"))
        );
        // A mistake is reported, and the old result is gone.
        l.write("form.sql", json!("SELECT FROM"));
        assert!(l.trigger("run", "onPress"));
        assert_eq!(l.client.attr("status", "variant"), Some(json!("failure")));
        assert_eq!(l.client.attr("line_0", "visible"), Some(json!(false)));
        // Picking a table shows its columns and writes a query.
        l.write("form.table", json!("orders"));
        assert!(l.trigger("table", "onChange"));
        assert_eq!(
            l.client.attr("col_6", "text"),
            Some(json!("placed       DATE"))
        );
        assert_eq!(l.client.attr("col_7", "visible"), Some(json!(false)));
        assert_eq!(l.client.attr("table_rows", "text"), Some(json!("80 rows")));
        assert!(l.client.take_diagnostics().is_empty());
    }
}
