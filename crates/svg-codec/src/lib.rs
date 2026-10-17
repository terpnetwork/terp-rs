//! SVP1: compact, deterministic SVG template codec.
//!
//! Wire: `"SVP" 0x01 | var table | template ops | 0x00`. Byte-identical to
//! `websites/terp.network/lib/svg-launchpad/pack.js`. The decoder only rebuilds
//! the template string and the `VariableDef` list; slots come from
//! `cw_svg::compute_template_slots`-equivalent scanning in the contract, and all
//! existing validation still runs on the decoded output.
use cw_svg::{RgbRange, VariableDef, VariableKind};

pub const MAX_OUT: usize = 42 * 1024;
const MAGIC: [u8; 4] = [0x53, 0x56, 0x50, 0x01];
const END: u8 = 0;
const LIT: u8 = 1;
const DICTOP: u8 = 2;
const COPY: u8 = 3;
const VAR: u8 = 4;
const NUM: u8 = 5;
const NL: u8 = 6;

#[derive(Debug, PartialEq, Eq)]
pub struct PackError(pub &'static str);

impl core::fmt::Display for PackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "svp: {}", self.0)
    }
}

pub const DICT: [&str; 116] = [
    "<svg ",
    "xmlns=\"http://www.w3.org/2000/svg\"",
    "viewBox=\"0 0 ",
    "width=\"",
    "height=\"",
    "</svg>",
    "<defs>",
    "</defs>",
    "<style>",
    "</style>",
    "<g ",
    "</g>",
    "<path ",
    "d=\"",
    "<circle ",
    "cx=\"",
    "cy=\"",
    "r=\"",
    "<rect ",
    "<ellipse ",
    "rx=\"",
    "ry=\"",
    "<line ",
    "x1=\"",
    "y1=\"",
    "x2=\"",
    "y2=\"",
    "<polygon ",
    "points=\"",
    "<text ",
    "</text>",
    "fill=\"",
    "fill=\"none\"",
    "stroke=\"",
    "stroke-width=\"",
    "stroke-linecap=\"round\"",
    "stroke-linejoin=\"round\"",
    "opacity=\"",
    "fill-opacity=\"",
    "stroke-opacity=\"",
    "transform=\"",
    "translate(",
    "rotate(",
    "scale(",
    "<animate ",
    "<animateTransform ",
    "attributeName=\"",
    "values=\"",
    "keyTimes=\"",
    "keySplines=\"",
    "calcMode=\"spline\"",
    "dur=\"",
    "begin=\"",
    "repeatCount=\"indefinite\"",
    "type=\"rotate\"",
    "<linearGradient ",
    "</linearGradient>",
    "<radialGradient ",
    "</radialGradient>",
    "<stop ",
    "offset=\"",
    "stop-color=\"",
    "stop-opacity=\"",
    "<filter ",
    "</filter>",
    "<feGaussianBlur ",
    "stdDeviation=\"",
    "<feDropShadow ",
    "flood-color=\"",
    "flood-opacity=\"",
    "<feMerge>",
    "</feMerge>",
    "<feMergeNode ",
    "in=\"SourceGraphic\"",
    "<clipPath ",
    "</clipPath>",
    "clip-path=\"url(#",
    "filter=\"url(#",
    "fill=\"url(#",
    "<mask ",
    "</mask>",
    "mask=\"url(#",
    "<use ",
    "href=\"#",
    "id=\"",
    "class=\"",
    "style=\"",
    "@keyframes ",
    "animation:",
    "opacity:",
    "transform:",
    "infinite",
    "ease-in-out",
    "linear",
    "alternate",
    "</animate>",
    "</circle>",
    "</path>",
    "</rect>",
    "\"/>",
    "\">",
    "\" ",
    "rgb(",
    "stroke-dasharray=\"",
    "stroke-dashoffset=\"",
    "gradientUnits=\"userSpaceOnUse\"",
    "x=\"",
    "y=\"",
    "font-family=\"",
    "font-size=\"",
    "text-anchor=\"middle\"",
    "mix-blend-mode",
    "</g>\n",
    "100%",
    "50%",
    "0%",
];

fn vlen(mut n: u128) -> usize {
    let mut l = 1;
    while n >= 0x80 {
        n >>= 7;
        l += 1;
    }
    l
}
fn wv(out: &mut Vec<u8>, mut n: u128) {
    while n >= 0x80 {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}
struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> Rd<'a> {
    fn u8(&mut self) -> Result<u8, PackError> {
        let x = *self.b.get(self.i).ok_or(PackError("truncated"))?;
        self.i += 1;
        Ok(x)
    }
    fn v(&mut self) -> Result<u128, PackError> {
        let (mut r, mut s) = (0u128, 0u32);
        loop {
            let x = self.u8()?;
            r |= ((x & 0x7f) as u128) << s;
            if x & 0x80 == 0 {
                return Ok(r);
            }
            s += 7;
            if s > 126 {
                return Err(PackError("varint overflow"));
            }
        }
    }
    fn n(&mut self) -> Result<usize, PackError> {
        let v = self.v()?;
        if v > (1u128 << 32) {
            return Err(PackError("int too big"));
        }
        Ok(v as usize)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], PackError> {
        let end = self.i.checked_add(n).ok_or(PackError("truncated"))?;
        let s = self.b.get(self.i..end).ok_or(PackError("truncated"))?;
        self.i = end;
        Ok(s)
    }
    fn str(&mut self) -> Result<String, PackError> {
        let n = self.n()?;
        let s = self.bytes(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| PackError("utf8"))
    }
}
fn wstr(out: &mut Vec<u8>, s: &str) {
    wv(out, s.len() as u128);
    out.extend_from_slice(s.as_bytes());
}

/// Canonical decimal (`-?(0|[1-9]\d*)(\.\d+)?`, <=30 digits) -> (header, mantissa).
pub fn num_pack(s: &str) -> Option<(u128, u128)> {
    let b = s.as_bytes();
    let neg = b.first() == Some(&b'-');
    let body = if neg { &b[1..] } else { b };
    let dot = body.iter().position(|&c| c == b'.');
    let (ip, fp) = match dot {
        Some(d) => (&body[..d], &body[d + 1..]),
        None => (body, &body[body.len()..]),
    };
    if ip.is_empty() || !ip.iter().all(u8::is_ascii_digit) || (ip.len() > 1 && ip[0] == b'0') {
        return None;
    }
    if dot.is_some() && (fp.is_empty() || !fp.iter().all(u8::is_ascii_digit)) {
        return None;
    }
    if ip.len() + fp.len() > 30 {
        return None;
    }
    let mut m: u128 = 0;
    for &c in ip.iter().chain(fp.iter()) {
        m = m * 10 + (c - b'0') as u128;
    }
    Some(((fp.len() as u128) * 2 + neg as u128, m))
}
pub fn num_unpack(h: u128, m: u128) -> Result<String, PackError> {
    let neg = h & 1 == 1;
    let frac = (h >> 1) as usize;
    if h >> 1 > 64 {
        return Err(PackError("frac too long"));
    }
    let mut d = m.to_string();
    while d.len() < frac + 1 {
        d.insert(0, '0');
    }
    if frac > 0 {
        d.insert(d.len() - frac, '.');
    }
    if neg {
        d.insert(0, '-');
    }
    Ok(d)
}

fn enc_vars(out: &mut Vec<u8>, vars: &[VariableDef]) {
    wv(out, vars.len() as u128);
    for v in vars {
        wstr(out, &v.name);
        match &v.kind {
            VariableKind::Options(o) => {
                out.push(0);
                wv(out, o.len() as u128);
                for s in o {
                    wstr(out, s);
                }
            }
            VariableKind::Range { min, max, precision } => match (num_pack(min), num_pack(max)) {
                (Some(a), Some(b)) => {
                    out.push(1);
                    wv(out, *precision as u128);
                    wv(out, a.0);
                    wv(out, a.1);
                    wv(out, b.0);
                    wv(out, b.1);
                }
                _ => {
                    out.push(4);
                    wv(out, *precision as u128);
                    wstr(out, min);
                    wstr(out, max);
                }
            },
            VariableKind::Rgb => out.push(2),
            VariableKind::RgbStyled(r) => {
                out.push(3);
                wv(out, r.len() as u128);
                for x in r {
                    out.extend_from_slice(&[x.r_min, x.r_max, x.g_min, x.g_max, x.b_min, x.b_max]);
                }
            }
        }
    }
}
fn dec_vars(r: &mut Rd) -> Result<Vec<VariableDef>, PackError> {
    let n = r.n()?;
    let mut vars = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let name = r.str()?;
        let kind = match r.u8()? {
            0 => {
                let c = r.n()?;
                let mut o = Vec::with_capacity(c.min(1024));
                for _ in 0..c {
                    o.push(r.str()?);
                }
                VariableKind::Options(o)
            }
            1 => {
                let precision = r.n()? as u32;
                let (h, m) = (r.v()?, r.v()?);
                let min = num_unpack(h, m)?;
                let (h, m) = (r.v()?, r.v()?);
                let max = num_unpack(h, m)?;
                VariableKind::Range { min, max, precision }
            }
            2 => VariableKind::Rgb,
            3 => {
                let c = r.n()?;
                let mut v = Vec::with_capacity(c.min(1024));
                for _ in 0..c {
                    let x = r.bytes(6)?;
                    v.push(RgbRange { r_min: x[0], r_max: x[1], g_min: x[2], g_max: x[3], b_min: x[4], b_max: x[5] });
                }
                VariableKind::RgbStyled(v)
            }
            4 => {
                let precision = r.n()? as u32;
                let min = r.str()?;
                let max = r.str()?;
                VariableKind::Range { min, max, precision }
            }
            _ => return Err(PackError("bad var kind")),
        };
        vars.push(VariableDef { name, kind });
    }
    Ok(vars)
}

fn num_at(s: &[u8], i: usize) -> usize {
    let mut j = i;
    if s.get(j) == Some(&b'-') {
        j += 1;
    }
    if !s.get(j).map_or(false, u8::is_ascii_digit) {
        return 0;
    }
    while s.get(j).map_or(false, u8::is_ascii_digit) {
        j += 1;
    }
    if s.get(j) == Some(&b'.') && s.get(j + 1).map_or(false, u8::is_ascii_digit) {
        j += 1;
        while s.get(j).map_or(false, u8::is_ascii_digit) {
            j += 1;
        }
    }
    j - i
}

/// Encode a template + variables. Off-chain (launchpad / CLI); not needed in the contract.
pub fn pack(template: &str, vars: &[VariableDef]) -> Result<Vec<u8>, PackError> {
    let src = template.as_bytes();
    let n = src.len();
    let mut out = MAGIC.to_vec();
    enc_vars(&mut out, vars);
    let mut lit: Vec<u8> = vec![];
    fn flush(out: &mut Vec<u8>, lit: &mut Vec<u8>) {
        if !lit.is_empty() {
            out.push(LIT);
            wv(out, lit.len() as u128);
            out.extend_from_slice(lit);
            lit.clear();
        }
    }
    let mut i = 0;
    while i < n {
        if src[i] == b'$' && src.get(i + 1) == Some(&b'{') {
            let mut j = i + 2;
            while j < n && src[j] != b'}' {
                j += 1;
            }
            if j >= n {
                return Err(PackError("unclosed placeholder"));
            }
            let name = &template[i + 2..j];
            let idx = vars.iter().position(|v| v.name == name).ok_or(PackError("undefined variable"))?;
            flush(&mut out, &mut lit);
            out.push(VAR);
            wv(&mut out, idx as u128);
            i = j + 1;
            continue;
        }
        // (op, covered, a, b)
        let mut best: Option<(u8, usize, u128, u128)> = None;
        let mut best_save: isize = 1;
        if src[i] == b'\n' {
            let mut s = 0;
            while i + 1 + s < n && src[i + 1 + s] == b' ' {
                s += 1;
            }
            let sv = (1 + s) as isize - (1 + vlen(s as u128)) as isize;
            if sv > best_save {
                best_save = sv;
                best = Some((NL, 1 + s, s as u128, 0));
            }
        }
        let nl = num_at(src, i);
        if nl > 0 {
            if let Some((h, m)) = core::str::from_utf8(&src[i..i + nl]).ok().and_then(num_pack) {
                let sv = nl as isize - (1 + vlen(h) + vlen(m)) as isize;
                if sv > best_save {
                    best_save = sv;
                    best = Some((NUM, nl, h, m));
                }
            }
        }
        for (d, w) in DICT.iter().enumerate() {
            let w = w.as_bytes();
            if src[i..].starts_with(w) {
                let sv = w.len() as isize - 2;
                if sv > best_save {
                    best_save = sv;
                    best = Some((DICTOP, w.len(), d as u128, 0));
                }
            }
        }
        let (mut cl, mut cd) = (0usize, 0usize);
        for j in (0..i).rev() {
            let mut l = 0;
            while i + l < n && src[j + l] == src[i + l] {
                l += 1;
            }
            if l > cl {
                cl = l;
                cd = i - j;
            }
        }
        if cl >= 4 {
            let sv = cl as isize - (1 + vlen(cd as u128) + vlen((cl - 4) as u128)) as isize;
            if sv > best_save {
                best = Some((COPY, cl, cd as u128, (cl - 4) as u128));
            }
        }
        match best {
            None => {
                lit.push(src[i]);
                i += 1;
            }
            Some((op, len, a, b)) => {
                flush(&mut out, &mut lit);
                out.push(op);
                match op {
                    NL => wv(&mut out, a),
                    DICTOP => out.push(a as u8),
                    _ => {
                        wv(&mut out, a);
                        wv(&mut out, b);
                    }
                }
                i += len;
            }
        }
    }
    flush(&mut out, &mut lit);
    out.push(END);
    Ok(out)
}

/// Decode only the variable table (cheap path for mint).
pub fn unpack_vars(bytes: &[u8]) -> Result<Vec<VariableDef>, PackError> {
    if bytes.get(..4) != Some(&MAGIC[..]) {
        return Err(PackError("bad magic"));
    }
    let mut r = Rd { b: bytes, i: 4 };
    dec_vars(&mut r)
}

/// Deterministic decoder. Output bounded by `MAX_OUT`.
pub fn unpack(bytes: &[u8]) -> Result<(String, Vec<VariableDef>), PackError> {
    if bytes.get(..4) != Some(&MAGIC[..]) {
        return Err(PackError("bad magic"));
    }
    let mut r = Rd { b: bytes, i: 4 };
    let vars = dec_vars(&mut r)?;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() * 3);
    macro_rules! room {
        ($k:expr) => {
            if out.len() + $k > MAX_OUT {
                return Err(PackError("output too large"));
            }
        };
    }
    loop {
        match r.u8()? {
            END => break,
            LIT => {
                let l = r.n()?;
                room!(l);
                let s = r.bytes(l)?;
                out.extend_from_slice(s);
            }
            DICTOP => {
                let w = DICT.get(r.u8()? as usize).ok_or(PackError("bad dict idx"))?;
                room!(w.len());
                out.extend_from_slice(w.as_bytes());
            }
            COPY => {
                let d = r.n()?;
                let l = r.n()? + 4;
                if d == 0 || d > out.len() {
                    return Err(PackError("bad copy"));
                }
                room!(l);
                let s = out.len() - d;
                for k in 0..l {
                    let x = out[s + k];
                    out.push(x);
                }
            }
            VAR => {
                let v = vars.get(r.n()?).ok_or(PackError("bad var idx"))?;
                room!(v.name.len() + 3);
                out.extend_from_slice(b"${");
                out.extend_from_slice(v.name.as_bytes());
                out.push(b'}');
            }
            NUM => {
                let (h, m) = (r.v()?, r.v()?);
                let s = num_unpack(h, m)?;
                room!(s.len());
                out.extend_from_slice(s.as_bytes());
            }
            NL => {
                let s = r.n()?;
                room!(s + 1);
                out.push(b'\n');
                out.resize(out.len() + s, b' ');
            }
            _ => return Err(PackError("bad op")),
        }
    }
    if r.i != bytes.len() {
        return Err(PackError("trailing bytes"));
    }
    let t = String::from_utf8(out).map_err(|_| PackError("utf8"))?;
    Ok((t, vars))
}

/// Optional big-integer view: little-endian u128 limbs.
pub fn to_limbs(bytes: &[u8]) -> Vec<u128> {
    bytes
        .chunks(16)
        .map(|c| c.iter().rev().fold(0u128, |a, &x| (a << 8) | x as u128))
        .collect()
}
