//! Fixed-base windowed `sk·G` (k19 lab, task 1a).
//!
//! sk (canonical, < n) is split into `FB_NWIN` little-endian W-bit digits by a linear gate on
//! each 64-bit limb. Window i selects `T_i[d]` from a fixed lookup table keyed by
//! (i+1, d) that also carries the point's 4 limbs + native for x and y, so the selected point
//! is pinned by the table (no constants to witness) and every digit is range-checked to
//! [0, 2^W) by the same lookup.
//!
//! Offsets: `T_i[d] = (d+2)·2^{Wi}·G` for i < last, `T_last[d] = d·2^{W·last}·G − C·G` with
//! `C = Σ_{i<last} 2·2^{Wi}`; the offsets cancel, so `Σ T_i[d_i] = sk·G`.
//! Incomplete addition: before window i the accumulator scalar is < (2^W+1)/(2^W−1)·2^{Wi}
//! < 2·2^{Wi} ≤ the window-i scalar, and both are < n, so `acc = ±T` cannot occur for i < last.
//! At the last window `acc = −T` means sk = 0 (rejected: dx = 0 fails div_checked) and
//! `acc = T` fixes sk to ≤ 2^W specific values (completeness loss only, never soundness:
//! add_point proves dx invertible).
use super::*;
use halo2_proofs::plonk::{Fixed, TableColumn};
use std::sync::OnceLock;

pub const FB_W: usize = 8;
pub const FB_NWIN: usize = 256 / FB_W;
const DIGITS_PER_LIMB: usize = 64 / FB_W;
const DIGIT_ROWS: usize = DIGITS_PER_LIMB / 8;

/// Lab switch: force GLV in `prove_key_pairing` even when the fixed-base table is configured.
pub static KEYPAIR_GLV: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct FixedBaseConfig {
    q_digits: Selector,
    q_lookup: Selector,
    win: Column<Fixed>,
    t_win: TableColumn,
    t_d: TableColumn,
    t_x: [TableColumn; 5],
    t_y: [TableColumn; 5],
    advices: [Column<Advice>; 9],
}

type HostPt = (Secp256k1Fp, Secp256k1Fp);

/// table[i][d] (host), computed once.
fn fb_table() -> &'static Vec<Vec<HostPt>> {
    static T: OnceLock<Vec<Vec<HostPt>>> = OnceLock::new();
    T.get_or_init(|| {
        let n = secp_n();
        let g = generator_xy();
        let last = FB_NWIN - 1;
        let mut c = BigUint::zero();
        for i in 0..last {
            c += BigUint::from(2u32) << (FB_W * i);
        }
        let mut out = Vec::with_capacity(FB_NWIN);
        for i in 0..FB_NWIN {
            let base = host_mul(g, &(BigUint::one() << (FB_W * i))).expect("2^Wi G");
            let start = if i < last {
                host_mul(g, &(BigUint::from(2u32) << (FB_W * i))).expect("2·2^Wi G")
            } else {
                host_mul(g, &(&n - &c)).expect("-C G")
            };
            let mut row = Vec::with_capacity(1 << FB_W);
            let mut p = start;
            for d in 0..(1usize << FB_W) {
                if d > 0 {
                    p = host_add(p, base);
                }
                row.push(p);
            }
            out.push(row);
        }
        out
    })
}

fn limbs_native(v: &Secp256k1Fp) -> ([pallas::Base; 4], pallas::Base) {
    let b = fe_to_biguint_host(v);
    (limbs4(&b).map(|l| fe_big(&l)), fe_big(&b))
}

impl FixedBaseConfig {
    pub fn configure(meta: &mut ConstraintSystem<pallas::Base>, advices: [Column<Advice>; 9]) -> Self {
        let q_digits = meta.selector();
        let q_lookup = meta.complex_selector();
        let win = meta.fixed_column();
        let t_win = meta.lookup_table_column();
        let t_d = meta.lookup_table_column();
        let t_x = core::array::from_fn(|_| meta.lookup_table_column());
        let t_y = core::array::from_fn(|_| meta.lookup_table_column());

        meta.create_gate("fb limb = Σ digits", |meta| {
            let q = meta.query_selector(q_digits);
            let limb = meta.query_advice(advices[8], Rotation::cur());
            let mut sum = Expression::Constant(pallas::Base::ZERO);
            for r in 0..DIGIT_ROWS {
                for c in 0..8 {
                    let pos = FB_W * (8 * r + c);
                    let coef = biguint_to_fe_simple(&(BigUint::one() << pos));
                    sum = sum + meta.query_advice(advices[c], Rotation(r as i32)) * Expression::Constant(coef);
                }
            }
            vec![q * (limb - sum)]
        });

        meta.lookup(|meta| {
            let q = meta.query_selector(q_lookup);
            let w = meta.query_fixed(win);
            let mut v = vec![(q.clone() * w, t_win)];
            v.push((q.clone() * meta.query_advice(advices[5], Rotation::cur()), t_d));
            for k in 0..5 {
                v.push((q.clone() * meta.query_advice(advices[k], Rotation::cur()), t_x[k]));
            }
            for k in 0..5 {
                v.push((q.clone() * meta.query_advice(advices[k], Rotation::next()), t_y[k]));
            }
            v
        });

        Self { q_digits, q_lookup, win, t_win, t_d, t_x, t_y, advices }
    }
}

impl Secp256k1Config {
    /// `configure` plus the fixed-base window table for `sk·G`.
    pub fn configure_with_fixed_base(
        meta: &mut ConstraintSystem<pallas::Base>,
        advices: [Column<Advice>; 9],
        range_check: LookupRangeCheckConfig<pallas::Base, 10>,
    ) -> Self {
        let mut c = Self::configure(meta, advices, range_check);
        c.fixed_base = Some(FixedBaseConfig::configure(meta, advices));
        c
    }
}

impl Secp256k1Chip {
    /// Load the window table (call once in synthesize when configured with a fixed base).
    pub fn load_fixed_base_table(&self, mut layouter: impl Layouter<pallas::Base>) -> Result<(), PlonkError> {
        let Some(fb) = self.fixed_base.clone() else { return Ok(()) };
        let tab = fb_table();
        layouter.assign_table(
            || "fb window table",
            |mut t| {
                let z = Value::known(pallas::Base::ZERO);
                t.assign_cell(|| "w", fb.t_win, 0, || z)?;
                t.assign_cell(|| "d", fb.t_d, 0, || z)?;
                for k in 0..5 {
                    t.assign_cell(|| "x", fb.t_x[k], 0, || z)?;
                    t.assign_cell(|| "y", fb.t_y[k], 0, || z)?;
                }
                let mut row = 1;
                for (i, ws) in tab.iter().enumerate() {
                    for (d, p) in ws.iter().enumerate() {
                        let (xl, xn) = limbs_native(&p.0);
                        let (yl, yn) = limbs_native(&p.1);
                        t.assign_cell(|| "w", fb.t_win, row, || Value::known(pallas::Base::from(i as u64 + 1)))?;
                        t.assign_cell(|| "d", fb.t_d, row, || Value::known(pallas::Base::from(d as u64)))?;
                        for k in 0..4 {
                            t.assign_cell(|| "x", fb.t_x[k], row, || Value::known(xl[k]))?;
                            t.assign_cell(|| "y", fb.t_y[k], row, || Value::known(yl[k]))?;
                        }
                        t.assign_cell(|| "xn", fb.t_x[4], row, || Value::known(xn))?;
                        t.assign_cell(|| "yn", fb.t_y[4], row, || Value::known(yn))?;
                        row += 1;
                    }
                }
                Ok(())
            },
        )
    }

    pub(crate) fn has_fixed_base(&self) -> bool {
        self.fixed_base.is_some() && !KEYPAIR_GLV.load(core::sync::atomic::Ordering::SeqCst)
    }

    /// sk·G with sk canonical (caller asserts sk < n).
    pub(crate) fn scalar_mul_fixed_base(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        sk: &ProperCrtUint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let fb = self.fixed_base.clone().expect("fixed base configured");
        let adv = fb.advices;
        let mask = (1u64 << FB_W) - 1;
        let digit_vals: Value<Vec<u64>> = sk.value.as_ref().map(|k| {
            let k = k.to_biguint().expect("non-negative");
            let mut ds: Vec<u64> = (0..FB_NWIN)
                .map(|i| ((&k >> (FB_W * i)) & BigUint::from(mask)).iter_u64_digits().next().unwrap_or(0))
                .collect();
            if forge::is(forge::FB_DIGIT) {
                ds[0] = (ds[0] + 1) & mask;
            }
            ds
        });
        // 1. digits of each limb
        let mut digits: Vec<AssignedCell<pallas::Base, pallas::Base>> = Vec::with_capacity(FB_NWIN);
        for (j, limb) in sk.truncation.limbs.iter().enumerate() {
            let cells = layouter.assign_region(
                || "fb digits",
                |mut region| {
                    fb.q_digits.enable(&mut region, 0)?;
                    limb.copy_advice(|| "limb", &mut region, adv[8], 0)?;
                    let mut cs = Vec::new();
                    for t in 0..DIGITS_PER_LIMB {
                        let i = j * DIGITS_PER_LIMB + t;
                        cs.push(region.assign_advice(
                            || "digit",
                            adv[t % 8],
                            t / 8,
                            || digit_vals.as_ref().map(|d| pallas::Base::from(d[i])),
                        )?);
                    }
                    Ok(cs)
                },
            )?;
            digits.extend(cells);
        }
        // 2. table lookups
        let tab = fb_table();
        let mut pts = Vec::with_capacity(FB_NWIN);
        for (i, dcell) in digits.iter().enumerate() {
            // forge::FB_COLLIDE: window 1 claims T_1 := acc_1 = T_0[d_0] (forces dx = dy = 0).
            let pv: Value<HostPt> = digit_vals.as_ref().map(|d| {
                if i == 1 && forge::is(forge::FB_COLLIDE) { tab[0][d[0] as usize] } else { tab[i][d[i] as usize] }
            });
            let pt = layouter.assign_region(
                || "fb lookup",
                |mut region| {
                    fb.q_lookup.enable(&mut region, 0)?;
                    region.assign_fixed(|| "win", fb.win, 0, || Value::known(pallas::Base::from(i as u64 + 1)))?;
                    dcell.copy_advice(|| "d", &mut region, adv[5], 0)?;
                    let mut coords = Vec::new();
                    for (r, is_y) in [(0usize, false), (1usize, true)] {
                        let c = pv.map(|p| if is_y { p.1 } else { p.0 });
                        let ln = c.map(|c| limbs_native(&c));
                        let mut limbs = Vec::new();
                        for k in 0..4 {
                            let forged = i == 0 && !is_y && k == 0 && forge::is(forge::FB_POINT);
                            limbs.push(region.assign_advice(|| "limb", adv[k], r, || {
                                ln.map(|(l, _)| if forged { l[k] + pallas::Base::ONE } else { l[k] })
                            })?);
                        }
                        let nat = region.assign_advice(|| "native", adv[4], r, || ln.map(|(_, n)| n))?;
                        let val = c.map(|c| BigInt::from(fe_to_biguint_host(&c)));
                        coords.push(CrtInteger::new(OverflowInteger::new(limbs, LIMB_BITS), nat, val));
                    }
                    let y = coords.pop().unwrap();
                    let x = coords.pop().unwrap();
                    Ok((x, y))
                },
            )?;
            pts.push(pt);
        }
        // 3. accumulate (no doublings)
        let mut it = pts.into_iter();
        let mut acc = it.next().unwrap();
        let last = FB_NWIN - 1;
        for (i, p) in it.enumerate() {
            let w = i + 1;
            acc = if w < last && !FB_ALWAYS_CHECKED.load(core::sync::atomic::Ordering::SeqCst) {
                // acc ≠ ±T_w by the scalar-size argument (module doc), so dx ≠ 0: plain div.
                self.add_point_distinct(layouter.namespace(|| format!("fb add {w}")), &acc, &p)?
            } else {
                // last window: acc = T is reachable for a few sk, so prove dx invertible.
                self.add_point(layouter.namespace(|| format!("fb add {w}")), &acc, &p)?
            };
        }
        Ok(acc)
    }

    /// Incomplete add for points whose x-coordinates are known distinct by construction.
    fn add_point_distinct(
        &self,
        mut layouter: impl Layouter<pallas::Base>,
        p: &SecpPoint<pallas::Base>,
        q: &SecpPoint<pallas::Base>,
    ) -> Result<SecpPoint<pallas::Base>, PlonkError> {
        let fp = &self.fp;
        let dy = fp.sub(layouter.namespace(|| "dy"), &q.1, &p.1)?;
        let dx = fp.sub(layouter.namespace(|| "dx"), &q.0, &p.0)?;
        let lambda = fp.div(layouter.namespace(|| "lambda"), &dy, &dx)?;
        let lambda_sq = fp.mul(layouter.namespace(|| "lambda_sq"), &lambda, &lambda)?;
        let x3 = fp.sub(layouter.namespace(|| "x3 part1"), &lambda_sq, &p.0)?;
        let x3 = fp.sub(layouter.namespace(|| "x3"), &x3, &q.0)?;
        let dx_new = fp.sub(layouter.namespace(|| "dx_new"), &p.0, &x3)?;
        let temp = fp.mul(layouter.namespace(|| "temp"), &lambda, &dx_new)?;
        let y3 = fp.sub(layouter.namespace(|| "y3"), &temp, &p.1)?;
        Ok((x3, y3))
    }
}

/// Lab switch: use div_checked for every fixed-base add (1a baseline).
pub static FB_ALWAYS_CHECKED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod audit_tests {
    use super::*;
    /// Plain-div audit: for every window w in 1..last, max acc scalar < min T_w scalar and
    /// max acc + max T_w < n, so acc != +-T_w (mod n) for ALL digit choices.
    #[test]
    fn fb_plain_div_scalar_bound() {
        let n = secp_n();
        let m = BigUint::from((1u64 << FB_W) - 1);
        for w in 1..FB_NWIN - 1 {
            let mut amax = BigUint::zero();
            let mut amin = BigUint::zero();
            for i in 0..w {
                amax += (&m + 2u32) << (FB_W * i);
                amin += BigUint::from(2u32) << (FB_W * i);
            }
            let tmin = BigUint::from(2u32) << (FB_W * w);
            let tmax = (&m + 2u32) << (FB_W * w);
            assert!(amin > BigUint::zero());
            assert!(amax < tmin, "w={w}: acc may equal T");
            assert!(&amax + &tmax < n, "w={w}: acc may equal -T");
        }
    }
}
