//! Property-input generation (impl plan 03 §8.12): random values from
//! types, derived deterministically from a SplitMix64 stream seeded by
//! the definition's `test_hash`, so runs are reproducible and re-seed
//! exactly when the tests change. `where` filters are refused upstream
//! (§9.8); this module only ever generates from the bare type.

use std::collections::BTreeMap;

use soil0::manifest::{TypeBody, TypeDef};
use soil0::types::SigType;

use crate::diag::Diag;

/// SplitMix64 exactly as pinned in soil0-cli §11.1 — the same stream
/// `fake_rand` uses, so nothing about generation depends on an
/// unpinned algorithm.
pub struct Gen {
    state: u64,
}

impl Gen {
    /// Seed from a `sha256:<64 hex>` spec hash: the digest's first 8
    /// bytes, big-endian (micro-pin §8.12).
    pub fn from_hash(hash: &str) -> Gen {
        let hex = hash.strip_prefix("sha256:").unwrap_or(hash);
        let mut seed: u64 = 0;
        for c in hex.bytes().take(16) {
            seed = (seed << 4) | u64::from((c as char).to_digit(16).unwrap_or(0));
        }
        Gen { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut z = self.state.wrapping_add(0x9E3779B97F4A7C15);
        self.state = z;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Recursive sums stop unfolding at this depth (payload-less variants
/// are preferred beyond it).
const DEPTH_CAP: usize = 4;
/// Geometric length parameter: continue with probability 7/8, capped.
const LEN_CAP: usize = 24;

fn geometric_len(g: &mut Gen) -> usize {
    let mut len = 0;
    while len < LEN_CAP && g.below(8) != 0 {
        len += 1;
    }
    len
}

fn ungenerable(what: &str) -> Diag {
    Diag::new(
        "test-ungenerable-type",
        format!(
            "cannot generate random values of {what}; \
             property inputs must be ground data types (impl plan 03 §8.12)"
        ),
    )
}

/// The resolution context: user types (from the root's env) plus the
/// kernel, by name.
pub struct TypeEnv {
    types: BTreeMap<String, TypeDef>,
}

impl TypeEnv {
    pub fn new(user: &[TypeDef]) -> TypeEnv {
        let mut types = BTreeMap::new();
        for td in soil0::kernel::kernel_typedefs() {
            types.insert(td.name.clone(), td);
        }
        for td in user {
            types.insert(td.name.clone(), td.clone());
        }
        TypeEnv { types }
    }
}

/// Generate one value of `ty` in the §7 JSON encoding.
pub fn generate(g: &mut Gen, env: &TypeEnv, ty: &SigType) -> Result<serde_json::Value, Diag> {
    gen_ty(g, env, ty, &BTreeMap::new(), 0)
}

fn gen_ty(
    g: &mut Gen,
    env: &TypeEnv,
    ty: &SigType,
    subst: &BTreeMap<String, SigType>,
    depth: usize,
) -> Result<serde_json::Value, Diag> {
    match ty {
        SigType::SVar { name } => match subst.get(name) {
            Some(bound) => gen_ty(g, env, bound, &BTreeMap::new(), depth),
            None => Err(ungenerable(&format!("the type variable `{name}`"))),
        },
        SigType::SArrow { .. } => Err(ungenerable("a function type")),
        SigType::SCon { name, args } => gen_con(g, env, name, args, subst, depth),
    }
}

fn gen_con(
    g: &mut Gen,
    env: &TypeEnv,
    name: &str,
    args: &[SigType],
    subst: &BTreeMap<String, SigType>,
    depth: usize,
) -> Result<serde_json::Value, Diag> {
    use serde_json::{json, Value};
    match name {
        "I64" => Ok(json!(gen_int(g, i64::MIN as i128, i64::MAX as i128))),
        "I32" => Ok(json!(gen_int(g, i32::MIN as i128, i32::MAX as i128))),
        "I16" => Ok(json!(gen_int(g, i16::MIN as i128, i16::MAX as i128))),
        "I8" => Ok(json!(gen_int(g, i8::MIN as i128, i8::MAX as i128))),
        "U64" => Ok(gen_u64_value(g)),
        "U32" => Ok(json!(gen_int(g, 0, u32::MAX as i128))),
        "U16" => Ok(json!(gen_int(g, 0, u16::MAX as i128))),
        "U8" => Ok(json!(gen_int(g, 0, u8::MAX as i128))),
        "BigInt" => Ok(json!(gen_int(g, -(1_i128 << 53) + 1, (1_i128 << 53) - 1))),
        "F64" => Ok(gen_f64(g)),
        "Utf8" => Ok(Value::String(gen_utf8(g))),
        "Bytes" => Ok(Value::String(gen_bytes_base64(g))),
        "Unit" => Ok(Value::Null),
        "List" => {
            let n = geometric_len(g);
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(gen_ty(g, env, &args[0], subst, depth + 1)?);
            }
            Ok(Value::Array(out))
        }
        "Map" => {
            let n = geometric_len(g);
            let mut entries: Vec<(String, Value, Value)> = Vec::new();
            for _ in 0..n {
                let k = gen_ty(g, env, &args[0], subst, depth + 1)?;
                let v = gen_ty(g, env, &args[1], subst, depth + 1)?;
                let key_id = serde_json::to_string(&k).expect("key serializes");
                // Comparator-consistent: one entry per key (dedup by
                // the canonical key text; decode re-sorts).
                if !entries.iter().any(|(id, _, _)| *id == key_id) {
                    entries.push((key_id, k, v));
                }
            }
            entries.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));
            Ok(Value::Array(
                entries
                    .into_iter()
                    .map(|(_, k, v)| json!({ "key": k, "value": v }))
                    .collect(),
            ))
        }
        _ => gen_named(g, env, name, args, depth),
    }
}

fn gen_named(
    g: &mut Gen,
    env: &TypeEnv,
    name: &str,
    args: &[SigType],
    depth: usize,
) -> Result<serde_json::Value, Diag> {
    use serde_json::{json, Value};
    let Some(td) = env.types.get(name) else {
        return Err(ungenerable(&format!("the unknown type `{name}`")));
    };
    let subst: BTreeMap<String, SigType> = td
        .params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect();
    match &td.body {
        TypeBody::OpaqueBody => Err(ungenerable(&format!("the opaque type `{name}`"))),
        TypeBody::Alias { ty } => gen_ty(g, env, ty, &subst, depth),
        TypeBody::Record { fields } => {
            let mut obj = serde_json::Map::new();
            for f in fields {
                if f.ignored.0.is_some() {
                    continue; // refilled from the default on decode (§7)
                }
                obj.insert(f.name.clone(), gen_ty(g, env, &f.shape, &subst, depth + 1)?);
            }
            Ok(Value::Object(obj))
        }
        TypeBody::Sum { variants } => {
            // Bool is the §7 special case; the kernel declares it.
            if name == "Bool" {
                return Ok(Value::Bool(g.below(2) == 0));
            }
            let leafy: Vec<&soil0::manifest::VariantD> = variants
                .iter()
                .filter(|v| v.payload.0.is_none() && v.record_fields.is_none())
                .collect();
            let pick = if depth >= DEPTH_CAP && !leafy.is_empty() {
                leafy[g.below(leafy.len() as u64) as usize]
            } else {
                &variants[g.below(variants.len() as u64) as usize]
            };
            if let Some(payload) = &pick.payload.0 {
                Ok(json!({ "tag": pick.name,
                           "value": gen_ty(g, env, payload, &subst, depth + 1)? }))
            } else if let Some(fields) = &pick.record_fields {
                let mut obj = serde_json::Map::new();
                for f in fields {
                    if f.ignored.0.is_some() {
                        continue;
                    }
                    obj.insert(f.name.clone(), gen_ty(g, env, &f.shape, &subst, depth + 1)?);
                }
                Ok(json!({ "tag": pick.name, "value": obj }))
            } else {
                Ok(json!({ "tag": pick.name }))
            }
        }
    }
}

/// Uniform-with-boundary-bias integers (§8.12): 1/8 of draws come
/// from the width's boundary set.
fn gen_int(g: &mut Gen, min: i128, max: i128) -> i64 {
    if g.below(8) == 0 {
        let boundaries = [0, 1, -1, min, max, min + 1, max - 1];
        let mut pick = boundaries[g.below(boundaries.len() as u64) as usize];
        if pick < min {
            pick = min;
        }
        return pick as i64;
    }
    let span = (max - min + 1) as u128;
    (min + (u128::from(g.next_u64()) % span) as i128) as i64
}

fn gen_u64_value(g: &mut Gen) -> serde_json::Value {
    if g.below(8) == 0 {
        let boundaries: [u64; 4] = [0, 1, u64::MAX, u64::MAX - 1];
        serde_json::json!(boundaries[g.below(4) as usize])
    } else {
        serde_json::json!(g.next_u64())
    }
}

/// Finite F64 plus the specials (§8.12). Specials and boundary
/// values take 1/8 of draws; the rest are finite bit patterns.
fn gen_f64(g: &mut Gen) -> serde_json::Value {
    use serde_json::{json, Value};
    if g.below(8) == 0 {
        return match g.below(9) {
            0 => Value::String("NaN".into()),
            1 => Value::String("Inf".into()),
            2 => Value::String("-Inf".into()),
            3 => json!(0.0),
            4 => json!(-0.0),
            5 => json!(1.0),
            6 => json!(-1.0),
            7 => json!(f64::MAX),
            _ => json!(f64::from_bits(1)), // the smallest denormal
        };
    }
    let mut bits = g.next_u64();
    while !f64::from_bits(bits).is_finite() {
        bits = g.next_u64();
    }
    json!(f64::from_bits(bits))
}

fn gen_utf8(g: &mut Gen) -> String {
    const POOL: &[char] = &[
        'a', 'b', 'z', 'A', 'Z', '0', '9', ' ', ',', '"', '\\', '\n', 'é', 'ß', '日', '🦀',
    ];
    let n = geometric_len(g);
    let mut s = String::new();
    for _ in 0..n {
        if g.below(4) == 0 {
            s.push(POOL[g.below(POOL.len() as u64) as usize]);
        } else {
            s.push((b'a' + (g.below(26) as u8)) as char);
        }
    }
    s
}

fn gen_bytes_base64(g: &mut Gen) -> String {
    // Hand-rolled base64 (dependency floor): standard alphabet with
    // padding, over a geometric-length byte string.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let n = geometric_len(g);
    let bytes: Vec<u8> = (0..n).map(|_| (g.next_u64() & 0xff) as u8).collect();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let idx = [
            b[0] >> 2,
            ((b[0] & 0x03) << 4) | (b[1] >> 4),
            ((b[1] & 0x0f) << 2) | (b[2] >> 6),
            b[2] & 0x3f,
        ];
        out.push(ALPHABET[idx[0] as usize] as char);
        out.push(ALPHABET[idx[1] as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[idx[2] as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[idx[3] as usize] as char
        } else {
            '='
        });
    }
    out
}
