use cosmwasm_std::coin;

pub mod suite;

pub mod prelude {
    pub use crate::suite::{CwSvgSuite, CwSvgSuiteDeployData};
}

pub mod fractal {
    use super::*;
    use cw_svg::{VariableDef, VariableKind};
    use rand::{rngs::StdRng, SeedableRng};

    /// Minimal SVG template that visualizes attractor params as a geometric constellation.
    /// The real fractal rendering happens off-chain; this gives each token a displayable on-chain preview.
    const FRACTAL_SVG_TEMPLATE: &str = include_str!("../svgs/research/fractals/fractal.svg");

    pub fn default_permissionless_fractal_init(
        seed: &u64,
        admin: &str,
    ) -> cw721_svg::InstantiateMsg {
        let mut rng = StdRng::seed_from_u64(*seed);
        let v = build_variable_defs(
            &(0..64)
                .map(|_| gen_zoom_set(&mut rng))
                .collect::<Vec<Vec<Zoom>>>(),
        );
        cw721_svg::InstantiateMsg {
            name: "Permissionless Fractals".into(),
            symbol: "FRACTAL".into(),
            collection_info_extension: cw721_svg::SvgCollectionMetadata {
                seed: blake3::hash(&seed.to_le_bytes()).as_bytes().into(),
                svg_template: FRACTAL_SVG_TEMPLATE.to_string(),
                variables: v.clone(),
                template_slots: cw_svg::compute_template_slots(FRACTAL_SVG_TEMPLATE, &v),
                price_tiers: vec![
                    cw721_svg::PriceTier {
                        until_count: 1000,
                        price: coin(100_000_000, "uthiol"),
                    },
                    cw721_svg::PriceTier {
                        until_count: 5000,
                        price: coin(500_000_000, "uthiol"),
                    },
                    cw721_svg::PriceTier {
                        until_count: 10000,
                        price: coin(1000_000_000, "uthiol"),
                    },
                ],
                total: 10000,
                mint_start_time: None,
                mint_end_time: None,
                whitelist: None,
            require_auth: false,
            },
            minter: Some(admin.to_string()),
            creator: Some(admin.to_string()),
            withdraw_address: Some(admin.to_string()),
        }
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct Zoom {
        cx: f64,
        cy: f64,
        scale: f64,
    }

    pub fn gen_float(rng: &mut impl rand::Rng, min: f64, max: f64, precision: u32) -> f64 {
        let raw: f64 = rng.gen_range(min..=max);
        let factor = 10f64.powi(precision as i32);
        (raw * factor).round() / factor
    }

    pub fn gen_zoom_set(rng: &mut impl rand::Rng) -> Vec<Zoom> {
        let num_zooms = rng.gen_range(3u32..=5);
        let mut zooms = vec![Zoom {
            cx: 0.0,
            cy: 0.0,
            scale: 1.0,
        }];

        for i in 1..num_zooms {
            let progress = i as f64 / (num_zooms - 1) as f64;
            zooms.push(Zoom {
                cx: gen_float(rng, -4.0, 4.0, 1),
                cy: gen_float(rng, -4.0, 4.0, 1),
                scale: gen_float(rng, 1.0 + progress, 1.0 + progress * 5.0, 1),
            });
        }

        zooms
    }

    /// Build variable definitions using Range for numeric params and Options for zooms.
    pub fn build_variable_defs(zoom_variants: &[Vec<Zoom>]) -> Vec<VariableDef> {
        let range_vars = vec![
            ("a", "-2.5", "2.5", 2u32),
            ("b", "-2.5", "2.5", 2),
            ("c", "-2.5", "2.5", 2),
            ("d", "-2.5", "2.5", 2),
            ("s", "-2.0", "2.0", 1),
            ("f", "-2.0", "2.0", 1),
            ("g", "-2.0", "2.0", 1),
            ("h", "-2.0", "2.0", 1),
            ("base_range", "10", "80", 0),
        ];

        let mut defs: Vec<VariableDef> = range_vars
            .into_iter()
            .map(|(name, min, max, precision)| VariableDef {
                name: name.to_string(),
                kind: VariableKind::Range {
                    min: min.to_string(),
                    max: max.to_string(),
                    precision,
                },
            })
            .collect();

        // Zooms remain as Options since they're complex JSON
        let zoom_options: Vec<String> = zoom_variants
            .iter()
            .map(|z| serde_json::to_string(z).unwrap())
            .collect();

        defs.push(VariableDef {
            name: "zooms".to_string(),
            kind: VariableKind::Options(zoom_options),
        });

        defs
    }
}
