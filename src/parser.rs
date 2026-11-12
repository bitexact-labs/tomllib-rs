//! Port of CPython's Lib/tomllib/_parser.py and _re.py (tomli by Taneli Hukkinen, MIT) to Rust.
//! Works on Unicode code points so error positions match the Python original exactly.
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};

pub type Table = IndexMap<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(String), // literal text, converted with int(text, 0) semantics by the caller
    Float(String), // literal text (may contain '_' or be inf/nan forms)
    Bool(bool),
    Date { y: u32, m: u32, d: u32 },
    Time { h: u32, mi: u32, s: u32, us: u32 },
    DateTime { y: u32, m: u32, d: u32, h: u32, mi: u32, s: u32, us: u32, tz: Tz },
    Array(Vec<Value>),
    Table(Table),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tz {
    Local,
    Utc,
    Offset { sign: i32, h: u32, m: u32 },
}

#[derive(Debug)]
pub enum Error {
    /// TOMLDecodeError(msg, doc, pos)
    Decode { msg: String, pos: usize },
    /// Python would raise a bare IndexError (internal invariant), kept for parity
    Index,
}

type R<T> = Result<T, Error>;

fn err<T>(msg: impl Into<String>, pos: usize) -> R<T> {
    Err(Error::Decode { msg: msg.into(), pos })
}

const FROZEN: u8 = 1;
const EXPLICIT_NEST: u8 = 2;

fn is_ctrl(c: char) -> bool {
    (c as u32) < 32 || c as u32 == 127
}
fn illegal_basic(c: char) -> bool {
    is_ctrl(c) && c != '\t'
}
fn illegal_multiline(c: char) -> bool {
    is_ctrl(c) && c != '\t' && c != '\n'
}
fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t'
}
fn is_ws_nl(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n'
}
fn is_bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// repr() of a str the way CPython prints it (enough for keys and single characters in error messages).
pub fn py_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == q => {
                out.push('\\');
                out.push(c)
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if !is_printable(c) => {
                let v = c as u32;
                if v <= 0xff {
                    out.push_str(&format!("\\x{:02x}", v))
                } else if v <= 0xffff {
                    out.push_str(&format!("\\u{:04x}", v))
                } else {
                    out.push_str(&format!("\\U{:08x}", v))
                }
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// Approximation of str.isprintable() for the characters that matter in practice:
/// C0/C1 controls, common format characters, line/paragraph separators, non-ASCII spaces, surrogates, private use, noncharacters.
fn is_printable(c: char) -> bool {
    let v = c as u32;
    !(v < 0x20
        || (0x7f..=0xa0).contains(&v)
        || v == 0xad
        || (0x600..=0x605).contains(&v)
        || v == 0x61c
        || v == 0x6dd
        || v == 0x70f
        || v == 0x1680
        || v == 0x180e
        || (0x2000..=0x200f).contains(&v)
        || (0x2028..=0x202f).contains(&v)
        || (0x205f..=0x2064).contains(&v)
        || (0x2066..=0x206f).contains(&v)
        || v == 0x3000
        || (0xd800..=0xf8ff).contains(&v)
        || v == 0xfeff
        || (0xfff0..=0xfffb).contains(&v)
        || (v & 0xfffe) == 0xfffe
        || (0xe0000..=0xe007f).contains(&v)
        || v >= 0xf0000)
}

fn key_repr(key: &[String]) -> String {
    if key.len() == 1 {
        format!("({},)", py_repr(&key[0]))
    } else {
        format!("({})", key.iter().map(|k| py_repr(k)).collect::<Vec<_>>().join(", "))
    }
}

#[derive(Default)]
struct FlagNode {
    flags: u8,
    rflags: u8,
    nested: HashMap<String, FlagNode>,
}

#[derive(Default)]
struct Flags {
    root: HashMap<String, FlagNode>,
    pending: HashSet<(Vec<String>, u8)>,
}

impl Flags {
    fn add_pending(&mut self, key: Vec<String>, flag: u8) {
        self.pending.insert((key, flag));
    }
    fn finalize_pending(&mut self) {
        let p: Vec<_> = self.pending.drain().collect();
        for (k, f) in p {
            self.set(&k, f, false);
        }
    }
    fn unset_all(&mut self, key: &[String]) {
        let mut cont = &mut self.root;
        for k in &key[..key.len() - 1] {
            match cont.get_mut(k) {
                Some(n) => cont = &mut n.nested,
                None => return,
            }
        }
        cont.shift_remove_compat(&key[key.len() - 1]);
    }
    fn set(&mut self, key: &[String], flag: u8, recursive: bool) {
        let mut cont = &mut self.root;
        for k in &key[..key.len() - 1] {
            cont = &mut cont.entry(k.clone()).or_default().nested;
        }
        let node = cont.entry(key[key.len() - 1].clone()).or_default();
        if recursive {
            node.rflags |= flag
        } else {
            node.flags |= flag
        }
    }
    fn is(&self, key: &[String], flag: u8) -> bool {
        if key.is_empty() {
            return false;
        }
        let mut cont = &self.root;
        for k in &key[..key.len() - 1] {
            match cont.get(k) {
                None => return false,
                Some(n) => {
                    if n.rflags & flag != 0 {
                        return true;
                    }
                    cont = &n.nested;
                }
            }
        }
        match cont.get(&key[key.len() - 1]) {
            Some(n) => (n.flags | n.rflags) & flag != 0,
            None => false,
        }
    }
}

trait RemoveCompat {
    fn shift_remove_compat(&mut self, k: &str);
}
impl RemoveCompat for HashMap<String, FlagNode> {
    fn shift_remove_compat(&mut self, k: &str) {
        self.remove(k);
    }
}

enum NestErr {
    Key,
    Index,
}

/// NestedDict.get_or_create_nest (the root dict is the starting table)
fn walk<'a>(table: &'a mut Table, key: &[String], access_lists: bool) -> Result<&'a mut Table, NestErr> {
    if key.is_empty() {
        return Ok(table);
    }
    let k = &key[0];
    if !table.contains_key(k) {
        table.insert(k.clone(), Value::Table(Table::new()));
    }
    let mut v = table.get_mut(k).unwrap();
    if access_lists {
        if let Value::Array(a) = v {
            v = a.last_mut().ok_or(NestErr::Index)?;
        }
    }
    match v {
        Value::Table(t) => walk(t, &key[1..], access_lists),
        _ => Err(NestErr::Key),
    }
}

fn append_nest_to_list(root: &mut Table, key: &[String]) -> Result<(), NestErr> {
    let cont = walk(root, &key[..key.len() - 1], true)?;
    let last = &key[key.len() - 1];
    match cont.get_mut(last) {
        Some(Value::Array(a)) => {
            a.push(Value::Table(Table::new()));
            Ok(())
        }
        Some(_) => Err(NestErr::Key),
        None => {
            cont.insert(last.clone(), Value::Array(vec![Value::Table(Table::new())]));
            Ok(())
        }
    }
}

struct P {
    s: Vec<char>,
}

impl P {
    fn at(&self, pos: usize) -> Option<char> {
        self.s.get(pos).copied()
    }
    fn starts(&self, pos: usize, pat: &str) -> bool {
        let mut i = pos;
        for c in pat.chars() {
            if self.at(i) != Some(c) {
                return false;
            }
            i += 1;
        }
        true
    }
    fn skip(&self, mut pos: usize, f: fn(char) -> bool) -> usize {
        while let Some(c) = self.at(pos) {
            if !f(c) {
                break;
            }
            pos += 1;
        }
        pos
    }
    fn index_of(&self, pat: &str, from: usize) -> Option<usize> {
        let p: Vec<char> = pat.chars().collect();
        if p.is_empty() {
            return Some(from);
        }
        let n = self.s.len();
        if n < p.len() {
            return None;
        }
        let mut i = from;
        while i + p.len() <= n {
            if self.s[i] == p[0] && self.s[i..i + p.len()] == p[..] {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn skip_until(&self, pos: usize, expect: &str, error_on: fn(char) -> bool, error_on_eof: bool) -> R<usize> {
        let new_pos = match self.index_of(expect, pos) {
            Some(p) => p,
            None => {
                let n = self.s.len();
                if error_on_eof {
                    return err(format!("Expected {}", py_repr(expect)), n);
                }
                n
            }
        };
        if let Some(off) = self.s[pos.min(new_pos)..new_pos].iter().position(|&c| error_on(c)) {
            let p = pos + off;
            return err(format!("Found invalid character {}", py_repr(&self.s[p].to_string())), p);
        }
        Ok(new_pos)
    }

    fn skip_comment(&self, pos: usize) -> R<usize> {
        if self.at(pos) == Some('#') {
            return self.skip_until(pos + 1, "\n", illegal_basic, false);
        }
        Ok(pos)
    }

    fn skip_comments_and_array_ws(&self, mut pos: usize) -> R<usize> {
        loop {
            let before = pos;
            pos = self.skip(pos, is_ws_nl);
            pos = self.skip_comment(pos)?;
            if pos == before {
                return Ok(pos);
            }
        }
    }

    fn parse_key(&self, pos: usize) -> R<(usize, Vec<String>)> {
        let (mut pos, part) = self.parse_key_part(pos)?;
        let mut key = vec![part];
        pos = self.skip(pos, is_ws);
        loop {
            if self.at(pos) != Some('.') {
                return Ok((pos, key));
            }
            pos += 1;
            pos = self.skip(pos, is_ws);
            let (p, part) = self.parse_key_part(pos)?;
            key.push(part);
            pos = self.skip(p, is_ws);
        }
    }

    fn parse_key_part(&self, pos: usize) -> R<(usize, String)> {
        match self.at(pos) {
            Some(c) if is_bare(c) => {
                let end = self.skip(pos, is_bare);
                Ok((end, self.s[pos..end].iter().collect()))
            }
            Some('\'') => self.parse_literal_str(pos),
            Some('"') => self.parse_basic_str(pos + 1, false),
            _ => err("Invalid initial character for a key part", pos),
        }
    }

    fn parse_key_value_pair(&self, pos: usize) -> R<(usize, Vec<String>, Value)> {
        let (mut pos, key) = self.parse_key(pos)?;
        if self.at(pos) != Some('=') {
            return err("Expected '=' after a key in a key/value pair", pos);
        }
        pos += 1;
        pos = self.skip(pos, is_ws);
        let (pos, v) = self.parse_value(pos)?;
        Ok((pos, key, v))
    }

    fn parse_array(&self, pos: usize) -> R<(usize, Value)> {
        let mut pos = pos + 1;
        let mut arr = vec![];
        pos = self.skip_comments_and_array_ws(pos)?;
        if self.starts(pos, "]") {
            return Ok((pos + 1, Value::Array(arr)));
        }
        loop {
            let (p, v) = self.parse_value(pos)?;
            arr.push(v);
            pos = self.skip_comments_and_array_ws(p)?;
            match self.at(pos) {
                Some(']') => return Ok((pos + 1, Value::Array(arr))),
                Some(',') => {}
                _ => return err("Unclosed array", pos),
            }
            pos += 1;
            pos = self.skip_comments_and_array_ws(pos)?;
            if self.starts(pos, "]") {
                return Ok((pos + 1, Value::Array(arr)));
            }
        }
    }

    fn parse_inline_table(&self, pos: usize) -> R<(usize, Value)> {
        let mut pos = pos + 1;
        let mut dict = Table::new();
        let mut flags = Flags::default();
        pos = self.skip(pos, is_ws);
        if self.starts(pos, "}") {
            return Ok((pos + 1, Value::Table(dict)));
        }
        loop {
            let (p, key, value) = self.parse_key_value_pair(pos)?;
            pos = p;
            let (parent, stem) = (&key[..key.len() - 1], &key[key.len() - 1]);
            if flags.is(&key, FROZEN) {
                return err(format!("Cannot mutate immutable namespace {}", key_repr(&key)), pos);
            }
            let nest = match walk(&mut dict, parent, false) {
                Ok(n) => n,
                Err(NestErr::Key) => return err("Cannot overwrite a value", pos),
                Err(NestErr::Index) => return Err(Error::Index),
            };
            if nest.contains_key(stem) {
                return err(format!("Duplicate inline table key {}", py_repr(stem)), pos);
            }
            let is_container = matches!(value, Value::Table(_) | Value::Array(_));
            nest.insert(stem.clone(), value);
            pos = self.skip(pos, is_ws);
            match self.at(pos) {
                Some('}') => return Ok((pos + 1, Value::Table(dict))),
                Some(',') => {}
                _ => return err("Unclosed inline table", pos),
            }
            if is_container {
                flags.set(&key, FROZEN, true);
            }
            pos += 1;
            pos = self.skip(pos, is_ws);
        }
    }

    fn parse_hex_char(&self, pos: usize, len: usize) -> R<(usize, String)> {
        let end = (pos + len).min(self.s.len());
        let hex = &self.s[pos.min(end)..end];
        if hex.len() != len || !hex.iter().all(|c| c.is_ascii_hexdigit()) {
            return err("Invalid hex value", pos);
        }
        let pos = pos + len;
        let v = u32::from_str_radix(&hex.iter().collect::<String>(), 16).unwrap();
        match char::from_u32(v) {
            Some(c) if v <= 55295 || (57344..=1114111).contains(&v) => Ok((pos, c.to_string())),
            _ => err("Escaped character is not a Unicode scalar value", pos),
        }
    }

    fn parse_basic_str_escape(&self, pos: usize, multiline: bool) -> R<(usize, String)> {
        let end = (pos + 2).min(self.s.len());
        let id: String = self.s[pos..end].iter().collect();
        let mut pos = pos + 2;
        if multiline && (id == "\\ " || id == "\\\t" || id == "\\\n") {
            if id != "\\\n" {
                pos = self.skip(pos, is_ws);
                match self.at(pos) {
                    None => return Ok((pos, String::new())),
                    Some('\n') => {}
                    Some(_) => return err("Unescaped '\\' in a string", pos),
                }
                pos += 1;
            }
            pos = self.skip(pos, is_ws_nl);
            return Ok((pos, String::new()));
        }
        match id.as_str() {
            "\\u" => self.parse_hex_char(pos, 4),
            "\\U" => self.parse_hex_char(pos, 8),
            "\\b" => Ok((pos, "\u{8}".into())),
            "\\t" => Ok((pos, "\t".into())),
            "\\n" => Ok((pos, "\n".into())),
            "\\f" => Ok((pos, "\u{c}".into())),
            "\\r" => Ok((pos, "\r".into())),
            "\\\"" => Ok((pos, "\"".into())),
            "\\\\" => Ok((pos, "\\".into())),
            _ => err("Unescaped '\\' in a string", pos),
        }
    }

    fn parse_literal_str(&self, pos: usize) -> R<(usize, String)> {
        let start = pos + 1;
        let p = self.skip_until(start, "'", illegal_basic, true)?;
        Ok((p + 1, self.s[start..p].iter().collect()))
    }

    fn parse_multiline_str(&self, pos: usize, literal: bool) -> R<(usize, String)> {
        let mut pos = pos + 3;
        if self.starts(pos, "\n") {
            pos += 1;
        }
        let (delim, mut result);
        if literal {
            delim = '\'';
            let end = self.skip_until(pos, "'''", illegal_multiline, true)?;
            result = self.s[pos..end].iter().collect::<String>();
            pos = end + 3;
        } else {
            delim = '"';
            let (p, r) = self.parse_basic_str(pos, true)?;
            pos = p;
            result = r;
        }
        if self.at(pos) != Some(delim) {
            return Ok((pos, result));
        }
        pos += 1;
        if self.at(pos) != Some(delim) {
            result.push(delim);
            return Ok((pos, result));
        }
        pos += 1;
        result.push(delim);
        result.push(delim);
        Ok((pos, result))
    }

    fn parse_basic_str(&self, mut pos: usize, multiline: bool) -> R<(usize, String)> {
        let error_on: fn(char) -> bool = if multiline { illegal_multiline } else { illegal_basic };
        let mut result = String::new();
        let mut start = pos;
        loop {
            let c = match self.at(pos) {
                None => return err("Unterminated string", pos),
                Some(c) => c,
            };
            if c == '"' {
                if !multiline {
                    result.extend(&self.s[start..pos]);
                    return Ok((pos + 1, result));
                }
                if self.starts(pos, "\"\"\"") {
                    result.extend(&self.s[start..pos]);
                    return Ok((pos + 3, result));
                }
                pos += 1;
                continue;
            }
            if c == '\\' {
                result.extend(&self.s[start..pos]);
                let (p, esc) = self.parse_basic_str_escape(pos, multiline)?;
                result.push_str(&esc);
                pos = p;
                start = pos;
                continue;
            }
            if error_on(c) {
                return err(format!("Illegal character {}", py_repr(&c.to_string())), pos);
            }
            pos += 1;
        }
    }

    // ---- regex ports (_re.py) ----
    fn digits(&self, pos: usize, n: usize) -> Option<u32> {
        let mut v = 0;
        for i in 0..n {
            let c = self.at(pos + i)?;
            v = v * 10 + c.to_digit(10).filter(|_| c.is_ascii_digit())?;
        }
        Some(v)
    }

    /// _TIME_RE_STR at pos: returns (end, h, m, s, micros)
    fn match_time(&self, pos: usize) -> Option<(usize, u32, u32, u32, u32)> {
        let h = self.digits(pos, 2)?;
        let first = self.at(pos)?;
        if !((first == '0' || first == '1') || (first == '2' && h <= 23)) || h > 23 {
            return None;
        }
        if self.at(pos + 2)? != ':' {
            return None;
        }
        let m = self.digits(pos + 3, 2)?;
        if self.at(pos + 3)? > '5' {
            return None;
        }
        if self.at(pos + 5)? != ':' {
            return None;
        }
        let s = self.digits(pos + 6, 2)?;
        if self.at(pos + 6)? > '5' {
            return None;
        }
        let mut end = pos + 8;
        let mut us = 0;
        if self.at(end) == Some('.') && self.at(end + 1).map_or(false, |c| c.is_ascii_digit()) {
            let mut i = end + 1;
            let mut frac = String::new();
            while let Some(c) = self.at(i) {
                if !c.is_ascii_digit() {
                    break;
                }
                if frac.len() < 6 {
                    frac.push(c);
                }
                i += 1;
            }
            while frac.len() < 6 {
                frac.push('0');
            }
            us = frac.parse().unwrap();
            end = i;
        }
        let _ = m;
        Some((end, h, m, s, us))
    }

    fn match_datetime(&self, pos: usize) -> Option<(usize, R<Value>)> {
        let y = self.digits(pos, 4)?;
        if self.at(pos + 4)? != '-' {
            return None;
        }
        let m = self.digits(pos + 5, 2)?;
        if !(1..=12).contains(&m) {
            return None;
        }
        if self.at(pos + 7)? != '-' {
            return None;
        }
        let d = self.digits(pos + 8, 2)?;
        if !(1..=31).contains(&d) {
            return None;
        }
        let mut end = pos + 10;
        let mut val = Value::Date { y, m, d };
        if let Some(sep) = self.at(end) {
            if sep == 'T' || sep == 't' || sep == ' ' {
                if let Some((tend, h, mi, s, us)) = self.match_time(end + 1) {
                    end = tend;
                    let mut tz = Tz::Local;
                    match self.at(end) {
                        Some('Z') | Some('z') => {
                            tz = Tz::Utc;
                            end += 1;
                        }
                        Some(sg) if sg == '+' || sg == '-' => {
                            if let (Some(oh), Some(':'), Some(om)) = (self.digits(end + 1, 2), self.at(end + 3), self.digits(end + 4, 2)) {
                                if oh <= 23 && self.at(end + 4).unwrap() <= '5' {
                                    tz = Tz::Offset { sign: if sg == '+' { 1 } else { -1 }, h: oh, m: om };
                                    end += 6;
                                }
                            }
                        }
                        _ => {}
                    }
                    val = Value::DateTime { y, m, d, h, mi, s, us, tz };
                }
            }
        }
        // date validity like datetime.date(...)
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let dim = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(m - 1) as usize];
        if y < 1 || d > dim {
            return Some((end, err("Invalid date or datetime", pos)));
        }
        Some((end, Ok(val)))
    }

    fn match_number(&self, pos: usize) -> Option<(usize, Value)> {
        let c0 = self.at(pos)?;
        if c0 == '0' {
            if let Some(k) = self.at(pos + 1) {
                let test: Option<fn(char) -> bool> = match k {
                    'x' => Some(|c: char| c.is_ascii_hexdigit()),
                    'b' => Some(|c: char| c == '0' || c == '1'),
                    'o' => Some(|c: char| ('0'..='7').contains(&c)),
                    _ => None,
                };
                if let Some(t) = test {
                    if self.at(pos + 2).map_or(false, t) {
                        let end = self.underscored(pos + 3, t);
                        return Some((end, Value::Int(self.s[pos..end].iter().collect())));
                    }
                }
            }
        }
        // decimal
        let mut p = pos;
        if c0 == '+' || c0 == '-' {
            p += 1;
        }
        let d0 = self.at(p)?;
        if d0 == '0' {
            p += 1;
        } else if ('1'..='9').contains(&d0) {
            p = self.underscored(p + 1, |c| c.is_ascii_digit());
        } else {
            return None;
        }
        let int_end = p;
        let isd = |c: char| c.is_ascii_digit();
        if self.at(p) == Some('.') && self.at(p + 1).map_or(false, isd) {
            p = self.underscored(p + 2, isd);
        }
        if matches!(self.at(p), Some('e') | Some('E')) {
            let mut q = p + 1;
            if matches!(self.at(q), Some('+') | Some('-')) {
                q += 1;
            }
            if self.at(q).map_or(false, isd) {
                p = self.underscored(q + 1, isd);
            }
        }
        let text: String = self.s[pos..p].iter().collect();
        if p > int_end {
            Some((p, Value::Float(text)))
        } else {
            Some((p, Value::Int(text)))
        }
    }

    /// (?:_?[class])*  starting at pos
    fn underscored(&self, mut pos: usize, t: fn(char) -> bool) -> usize {
        loop {
            match self.at(pos) {
                Some(c) if t(c) => pos += 1,
                Some('_') if self.at(pos + 1).map_or(false, t) => pos += 2,
                _ => return pos,
            }
        }
    }

    fn parse_value(&self, pos: usize) -> R<(usize, Value)> {
        let c = self.at(pos);
        match c {
            Some('"') => {
                if self.starts(pos, "\"\"\"") {
                    let (p, s) = self.parse_multiline_str(pos, false)?;
                    return Ok((p, Value::Str(s)));
                }
                let (p, s) = self.parse_basic_str(pos + 1, false)?;
                return Ok((p, Value::Str(s)));
            }
            Some('\'') => {
                if self.starts(pos, "'''") {
                    let (p, s) = self.parse_multiline_str(pos, true)?;
                    return Ok((p, Value::Str(s)));
                }
                let (p, s) = self.parse_literal_str(pos)?;
                return Ok((p, Value::Str(s)));
            }
            Some('t') if self.starts(pos, "true") => return Ok((pos + 4, Value::Bool(true))),
            Some('f') if self.starts(pos, "false") => return Ok((pos + 5, Value::Bool(false))),
            Some('[') => return self.parse_array(pos),
            Some('{') => return self.parse_inline_table(pos),
            _ => {}
        }
        if let Some((end, v)) = self.match_datetime(pos) {
            return Ok((end, v?));
        }
        if let Some((end, h, mi, s, us)) = self.match_time(pos) {
            return Ok((end, Value::Time { h, mi, s, us }));
        }
        if let Some((end, v)) = self.match_number(pos) {
            return Ok((end, v));
        }
        for sp in ["inf", "nan"] {
            if self.starts(pos, sp) {
                return Ok((pos + 3, Value::Float(sp.into())));
            }
        }
        for sp in ["-inf", "+inf", "-nan", "+nan"] {
            if self.starts(pos, sp) {
                return Ok((pos + 4, Value::Float(sp.into())));
            }
        }
        err("Invalid value", pos)
    }
}

fn nest_err<T>(e: NestErr, pos: usize) -> R<T> {
    match e {
        NestErr::Key => err("Cannot overwrite a value", pos),
        NestErr::Index => Err(Error::Index),
    }
}

/// tomllib.loads. `s` must already have "\r\n" replaced by "\n" (positions refer to that string).
pub fn loads(src: &str) -> R<Table> {
    let p = P { s: src.chars().collect() };
    let mut pos = 0usize;
    let mut data = Table::new();
    let mut flags = Flags::default();
    let mut header: Vec<String> = vec![];
    loop {
        pos = p.skip(pos, is_ws);
        let c = match p.at(pos) {
            None => break,
            Some(c) => c,
        };
        if c == '\n' {
            pos += 1;
            continue;
        }
        if is_bare(c) || c == '"' || c == '\'' {
            // key_value_rule
            let (np, key, value) = p.parse_key_value_pair(pos)?;
            pos = np;
            let (kparent, kstem) = (&key[..key.len() - 1], &key[key.len() - 1]);
            let abs_parent: Vec<String> = header.iter().chain(kparent.iter()).cloned().collect();
            for i in 1..key.len() {
                let cont_key: Vec<String> = header.iter().chain(key[..i].iter()).cloned().collect();
                if flags.is(&cont_key, EXPLICIT_NEST) {
                    return err(format!("Cannot redefine namespace {}", key_repr(&cont_key)), pos);
                }
                flags.add_pending(cont_key, EXPLICIT_NEST);
            }
            if flags.is(&abs_parent, FROZEN) {
                return err(format!("Cannot mutate immutable namespace {}", key_repr(&abs_parent)), pos);
            }
            let nest = match walk(&mut data, &abs_parent, true) {
                Ok(n) => n,
                Err(e) => return nest_err(e, pos),
            };
            if nest.contains_key(kstem) {
                return err("Cannot overwrite a value", pos);
            }
            let container = matches!(value, Value::Table(_) | Value::Array(_));
            nest.insert(kstem.clone(), value);
            if container {
                let full: Vec<String> = header.iter().chain(key.iter()).cloned().collect();
                flags.set(&full, FROZEN, true);
            }
            pos = p.skip(pos, is_ws);
        } else if c == '[' {
            let second = p.at(pos + 1);
            flags.finalize_pending();
            if second == Some('[') {
                // create_list_rule
                let mut q = p.skip(pos + 2, is_ws);
                let (np, key) = p.parse_key(q)?;
                q = np;
                if flags.is(&key, FROZEN) {
                    return err(format!("Cannot mutate immutable namespace {}", key_repr(&key)), q);
                }
                flags.unset_all(&key);
                flags.set(&key, EXPLICIT_NEST, false);
                if let Err(e) = append_nest_to_list(&mut data, &key) {
                    return nest_err(e, q);
                }
                if !p.starts(q, "]]") {
                    return err("Expected ']]' at the end of an array declaration", q);
                }
                pos = q + 2;
                header = key;
            } else {
                // create_dict_rule
                let mut q = p.skip(pos + 1, is_ws);
                let (np, key) = p.parse_key(q)?;
                q = np;
                if flags.is(&key, EXPLICIT_NEST) || flags.is(&key, FROZEN) {
                    return err(format!("Cannot declare {} twice", key_repr(&key)), q);
                }
                flags.set(&key, EXPLICIT_NEST, false);
                if let Err(e) = walk(&mut data, &key, true) {
                    return nest_err(e, q);
                }
                if !p.starts(q, "]") {
                    return err("Expected ']' at the end of a table declaration", q);
                }
                pos = q + 1;
                header = key;
            }
            pos = p.skip(pos, is_ws);
        } else if c != '#' {
            return err("Invalid statement", pos);
        }
        pos = p.skip_comment(pos)?;
        match p.at(pos) {
            None => break,
            Some('\n') => pos += 1,
            Some(_) => return err("Expected newline or end of document after a statement", pos),
        }
    }
    Ok(data)
}
